//! `claudemon wrap <command> [args...]`
//!
//! Spawns the command in a PTY, forwards the user's terminal I/O to it
//! transparently, and mirrors the byte stream to the daemon over WebSocket.
//! The daemon can push input back (approve/deny, new prompts, signals).

pub mod pty;

use std::io::{IsTerminal, Read};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::{SinkExt, StreamExt};
use portable_pty::PtySize;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::protocol::WrapperMessage;

pub async fn run_with_daemon(argv: Vec<String>, daemon_ws: &str) -> Result<()> {
    if argv.is_empty() {
        bail!("usage: claudemon wrap <command> [args...]");
    }

    let session_id = Uuid::new_v4().to_string();
    let cwd = std::env::current_dir()?
        .to_string_lossy()
        .into_owned();
    let (cols, rows) = terminal_size();

    tracing::info!(%session_id, %cwd, ?argv, "starting wrapper");
    eprintln!("[claudemon] session {session_id} — Ctrl-D to exit");

    let pty = pty::spawn(
        &argv,
        &cwd,
        PtySize { cols, rows, pixel_width: 0, pixel_height: 0 },
        &std::collections::HashMap::new(),
    )?;
    let pty = Arc::new(pty);

    // Open WS to daemon. If it's unreachable we still want the wrapper to work
    // locally — just log and continue without mirroring.
    let ws_url = format!("{daemon_ws}/{session_id}");
    let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((stream, _)) => {
            tracing::info!(%ws_url, "connected to daemon");
            Some(stream)
        }
        Err(err) => {
            tracing::warn!(%ws_url, ?err, "daemon unreachable; running detached");
            None
        }
    };

    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WrapperMessage>();

    // Register with daemon (if connected) and split into read/write tasks.
    if let Some(ws) = ws_stream {
        let (mut sink, mut stream) = ws.split();
        let register = WrapperMessage::Register {
            session_id: session_id.clone(),
            cwd: cwd.clone(),
            argv: argv.clone(),
            cols,
            rows,
        };
        sink.send(Message::Text(serde_json::to_string(&register)?)).await?;

        // ws_rx → sink (daemon-bound messages from anywhere in this process)
        let sink = Arc::new(Mutex::new(sink));
        let sink_for_writer = sink.clone();
        tokio::spawn(async move {
            while let Some(msg) = ws_rx.recv().await {
                let text = match serde_json::to_string(&msg) {
                    Ok(t) => t,
                    Err(err) => {
                        tracing::warn!(?err, "serialize ws msg");
                        continue;
                    }
                };
                let mut s = sink_for_writer.lock().await;
                if let Err(err) = s.send(Message::Text(text)).await {
                    tracing::warn!(?err, "ws send failed");
                    break;
                }
            }
        });

        // stream → PTY (input/signal/resize from daemon)
        let pty_for_reader = pty.clone();
        tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                let Ok(Message::Text(text)) = frame else { continue };
                let msg: WrapperMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(err) => {
                        tracing::warn!(?err, "decode ws msg");
                        continue;
                    }
                };
                match msg {
                    WrapperMessage::Input { bytes } => {
                        if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                            let _ = pty::write_bytes(&pty_for_reader, &decoded).await;
                        }
                    }
                    WrapperMessage::Signal { signal } => {
                        tracing::info!(?signal, "delivering signal");
                        match signal {
                            // Interactive interrupt: Ctrl-C byte through the tty.
                            crate::protocol::Signal::Sigint => {
                                let _ = pty::write_bytes(&pty_for_reader, b"\x03").await;
                            }
                            // Terminate / kill: real process signal.
                            other => {
                                if let Err(err) = pty::signal_child(&pty_for_reader, other) {
                                    tracing::warn!(?err, "signal delivery failed");
                                }
                            }
                        }
                    }
                    WrapperMessage::Resize { cols, rows } => {
                        let _ = pty::resize(&pty_for_reader, cols, rows).await;
                    }
                    _ => {} // ignore wrapper→daemon variants
                }
            }
        });
    } else {
        // No daemon connection: drain ws_rx so senders don't block.
        tokio::spawn(async move { while ws_rx.recv().await.is_some() {} });
    }

    // PTY reader → (stdout + daemon)
    let (pty_tx, mut pty_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    pty::start_reader(&pty, pty_tx)?;
    let ws_tx_for_pty = ws_tx.clone();
    let pty_pump = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut stdout = tokio::io::stdout();
        while let Some(chunk) = pty_rx.recv().await {
            if stdout.write_all(&chunk).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
            let _ = ws_tx_for_pty.send(WrapperMessage::Output {
                bytes: B64.encode(&chunk),
            });
        }
    });

    // stdin → PTY (only if we have an interactive terminal)
    let stdin_is_tty = std::io::stdin().is_terminal();
    let raw_enabled = if stdin_is_tty {
        enable_raw_mode().is_ok()
    } else {
        false
    };
    let pty_for_stdin = pty.clone();
    let stdin_pump = tokio::task::spawn_blocking(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Already on a blocking thread — write synchronously rather
                    // than re-entering the async runtime via block_on.
                    if pty::write_bytes_blocking(&pty_for_stdin, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait on the child. portable-pty's Child::wait is blocking.
    let child = pty.child.clone();
    let wait_handle = tokio::task::spawn_blocking(move || {
        let mut c = child.lock().expect("PTY child mutex poisoned");
        c.wait().ok()
    });

    let exit = wait_handle.await.context("waiting on child")?;
    let code = exit.and_then(|s| s.exit_code().try_into().ok());

    if raw_enabled {
        let _ = disable_raw_mode();
    }
    let _ = ws_tx.send(WrapperMessage::Exited { code });
    // Give the WS sender a moment to flush the Exited message.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    pty_pump.abort();
    stdin_pump.abort();
    Ok(())
}

fn terminal_size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((80, 24))
}
