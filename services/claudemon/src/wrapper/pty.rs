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
#[path = "pty_owned.rs"]
mod owned;

pub struct PtyHandle {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    scope: Arc<owned::Scope>,
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
    let pair = pty_system.openpty(size).context("openpty failed")?;

    let mut cmd = CommandBuilder::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.cwd(cwd);
    // Pass through current env so the child sees the same shell/tool config,
    // then layer the caller-supplied overrides on top.  Both steps are local
    // to this CommandBuilder — no process-global mutation occurs.
    for (k, v) in std::env::vars_os() {
        if crate::child_env::is_host_authority(&k) {
            cmd.env_remove(k);
        } else {
            cmd.env(k, v);
        }
    }
    for (k, v) in extra_env {
        if crate::child_env::is_host_authority(std::ffi::OsStr::new(k)) {
            cmd.env_remove(k);
        } else {
            cmd.env(k, v);
        }
    }
    // Preserve portable-pty's shell/Windows registry defaults while excluding
    // authority keys that may have entered through that base environment.
    for key in crate::child_env::HOST_AUTHORITY_KEYS {
        cmd.env_remove(key);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .context("spawning child in PTY")?;
    // Once the child has the slave, we don't need it.
    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));
    let (child, scope) = owned::wrap(child, master.clone())?;
    let writer = {
        let master = master.lock().expect("PTY master mutex poisoned");
        master.take_writer()
    }
    .context("taking PTY writer")?;
    Ok(PtyHandle {
        master,
        writer: Arc::new(Mutex::new(writer)),
        child: Arc::new(Mutex::new(child)),
        scope,
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

/// Signal only the verified owned PTY session/group (or per-child Windows job).
/// The direct-child mutex excludes concurrent reap before numeric Unix signals.
pub fn signal_child(handle: &PtyHandle, sig: crate::protocol::Signal) -> Result<()> {
    let _child = handle.child.lock().expect("PTY child mutex poisoned");
    handle.scope.signal(sig).context("signal owned PTY")
}

/// A blocking waiter that releases the child mutex between polls, so an external
/// wrapper's lifetime wait does not prevent another thread from terminating it.
pub fn wait_child(handle: &PtyHandle) -> Result<portable_pty::ExitStatus> {
    loop {
        if let Some(status) = handle
            .child
            .lock()
            .expect("PTY child mutex poisoned")
            .try_wait()?
        {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
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
        m.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    })
    .await
    .context("join resize task")??;
    Ok(())
}
