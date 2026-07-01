//! PTY plumbing for the wrapper.
//!
//! `portable-pty` exposes blocking `Read`/`Write` handles for the master
//! side. We bridge them to tokio with `spawn_blocking` for reads and a
//! plain `Mutex<Box<dyn Write>>` behind tokio tasks for writes.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;

pub struct PtyHandle {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
}

/// Spawn a command in a new PTY.
///
/// `extra_env` is merged on top of the daemon's current environment.  The
/// overrides are passed directly into the `CommandBuilder` rather than being
/// applied to the process-global environment, which eliminates the data race
/// that existed when two concurrent spawns with overlapping keys both called
/// `std::env::set_var`.
pub fn spawn(
    argv: &[String],
    cwd: &str,
    size: PtySize,
    extra_env: &std::collections::HashMap<String, String>,
) -> Result<PtyHandle> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .context("openpty failed")?;

    let mut cmd = CommandBuilder::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.cwd(cwd);
    // Pass through current env so the child sees the same shell/tool config,
    // then layer the caller-supplied overrides on top.  Both steps are local
    // to this CommandBuilder — no process-global mutation occurs.
    for (k, v) in std::env::vars() {
        cmd.env(k, v);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let child = pair.slave.spawn_command(cmd).context("spawning child in PTY")?;
    // Once the child has the slave, we don't need it.
    drop(pair.slave);

    let writer = pair.master.take_writer().context("taking PTY writer")?;
    Ok(PtyHandle {
        master: Arc::new(Mutex::new(pair.master)),
        writer: Arc::new(Mutex::new(writer)),
        child: Arc::new(Mutex::new(child)),
    })
}

/// Spawn a blocking reader that pumps PTY output to an mpsc channel.
/// `tx` carries owned `Vec<u8>` chunks (each up to 8 KiB). Channel close
/// signals EOF / child exit.
pub fn start_reader(handle: &PtyHandle, tx: mpsc::UnboundedSender<Vec<u8>>) -> Result<()> {
    let master = handle.master.clone();
    let mut reader = {
        let m = master.lock().expect("PTY master mutex poisoned");
        m.try_clone_reader().context("clone PTY reader")?
    };
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::debug!(?err, "PTY reader ended");
                    break;
                }
            }
        }
    });
    Ok(())
}

pub async fn write_bytes(handle: &PtyHandle, bytes: &[u8]) -> Result<()> {
    let writer = handle.writer.clone();
    let chunk = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut w = writer.lock().expect("PTY writer mutex poisoned");
        w.write_all(&chunk)?;
        w.flush()?;
        Ok(())
    })
    .await
    .context("join write task")??;
    Ok(())
}

/// Synchronous PTY write for callers already running on a blocking thread.
///
/// The async `write_bytes` dispatches a `spawn_blocking`; calling it via
/// `Handle::block_on` from inside another blocking thread re-enters the runtime
/// (deadlocks on a current-thread runtime, exhausts the blocking pool on a
/// multi-thread one). The stdin pump is already on its own `spawn_blocking`
/// thread, so it writes directly to the PTY writer here instead.
pub fn write_bytes_blocking(handle: &PtyHandle, bytes: &[u8]) -> Result<()> {
    let mut w = handle.writer.lock().expect("PTY writer mutex poisoned");
    w.write_all(bytes)?;
    w.flush()?;
    Ok(())
}

/// Deliver a real process signal to the PTY child.
///
/// SIGINT is intentionally NOT handled here — callers send the Ctrl-C byte
/// (`\x03`) through the tty, which is how an interactive interrupt reaches the
/// foreground process group. This covers the terminate/kill signals a Ctrl-C
/// cannot express, so a runaway session can actually be stopped.
///
/// On Unix, SIGTERM is sent to the child's pid via `nix`. SIGKILL uses
/// portable-pty's `kill()` (SIGKILL on Unix). On non-Unix, both fall back to
/// `kill()` (TerminateProcess), since there is no SIGTERM equivalent.
pub fn signal_child(handle: &PtyHandle, sig: crate::protocol::Signal) -> Result<()> {
    use crate::protocol::Signal;
    let mut child = handle.child.lock().expect("PTY child mutex poisoned");
    match sig {
        Signal::Sigkill => {
            child.kill().context("SIGKILL child")?;
        }
        Signal::Sigterm | Signal::Sigint => {
            #[cfg(unix)]
            {
                let posix = match sig {
                    Signal::Sigterm => nix::sys::signal::Signal::SIGTERM,
                    _ => nix::sys::signal::Signal::SIGINT,
                };
                if let Some(pid) = child.process_id() {
                    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), posix)
                        .with_context(|| format!("send {posix:?} to pid {pid}"))?;
                }
            }
            #[cfg(not(unix))]
            {
                // No SIGTERM on Windows — terminate the process.
                child.kill().context("terminate child")?;
            }
        }
    }
    Ok(())
}

/// Non-blocking check whether the PTY child has already exited. Used by hybrid
/// managed adapters to notice their TUI dying so the whole session tears down
/// (rather than leaving the driver + provider server running against a dead
/// thread). Reaps the child if it has exited, so it doesn't linger as a zombie.
pub fn has_exited(handle: &PtyHandle) -> bool {
    let mut child = handle.child.lock().expect("PTY child mutex poisoned");
    matches!(child.try_wait(), Ok(Some(_)))
}

pub async fn resize(handle: &PtyHandle, cols: u16, rows: u16) -> Result<()> {
    let master = handle.master.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let m = master.lock().expect("PTY master mutex poisoned");
        m.resize(PtySize { cols, rows, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    })
    .await
    .context("join resize task")??;
    Ok(())
}
