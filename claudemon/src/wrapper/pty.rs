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

pub fn spawn(argv: &[String], cwd: &str, size: PtySize) -> Result<PtyHandle> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .context("openpty failed")?;

    let mut cmd = CommandBuilder::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.cwd(cwd);
    // Pass through current env so the child sees the same shell/tool config.
    for (k, v) in std::env::vars() {
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
