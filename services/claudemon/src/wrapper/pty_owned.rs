//! Kill owned PTY groups before releasing their direct-child PID anchor.
//! portable-pty's Unix Child::kill sends HUP to a PID, not KILL to its group.
#[cfg(windows)]
use crate::child_job as windows_job;
#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};
use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct State {
    reaped: bool,
    killed: bool,
}
pub(super) struct Scope {
    pid: u32,
    state: Mutex<State>,
    #[cfg(unix)]
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    #[cfg(windows)]
    job: windows_job::Job,
}
impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedPty").field("pid", &self.pid).finish()
    }
}
impl Scope {
    fn new(child: &dyn Child, master: Arc<Mutex<Box<dyn MasterPty + Send>>>) -> Result<Self> {
        let pid = child
            .process_id()
            .context("PTY child lacks process identity")?;
        #[cfg(target_os = "macos")]
        {
            // portable-pty's checked pre_exec calls setsid before exec. XNU
            // destroys the old session-leader marker at exit, so it cannot be
            // queried on a zombie. Verify its original child/PGID identity;
            // every live child still needs the expected SID. Foreground group
            // signalling separately requires tcgetsid(master)==this PID.
            let exited = crate::child_group::verify_anchor(pid)?;
            if !exited
                && unsafe { nix::libc::getsid(pid as i32) } != pid as i32
                && !crate::child_group::verify_anchor(pid)?
            {
                bail!("PTY child does not own a separate session");
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use nix::libc;
            if pid == 0
                || pid > i32::MAX as u32
                || unsafe { libc::getsid(pid as i32) } != pid as i32
                || unsafe { libc::getpgid(pid as i32) } != pid as i32
                || unsafe { libc::getpgrp() } == pid as i32
            {
                bail!("PTY child does not own a separate session and process group");
            }
        }
        #[cfg(windows)]
        let job = windows_job::Job::assign(
            child
                .as_raw_handle()
                .context("PTY child lacks Windows process handle")?,
        )?;
        #[cfg(not(unix))]
        let _ = master;
        Ok(Self {
            pid,
            state: Mutex::new(State::default()),
            #[cfg(unix)]
            master,
            #[cfg(windows)]
            job,
        })
    }
    #[cfg(unix)]
    fn observe_anchor(&self) -> io::Result<bool> {
        use nix::libc;
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { info.assume_init().si_pid() } != 0)
    }
    pub(super) fn signal(&self, signal: crate::protocol::Signal) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.reaped || state.killed {
            return Ok(());
        }
        #[cfg(unix)]
        {
            use nix::libc;
            // No numeric signal unless our original, unreaped child still anchors it.
            self.observe_anchor()?;
            let signal = match signal {
                crate::protocol::Signal::Sigkill => libc::SIGKILL,
                crate::protocol::Signal::Sigterm => libc::SIGTERM,
                crate::protocol::Signal::Sigint => libc::SIGINT,
            };
            let mut foreground_error = None;
            let master = self.master.lock().unwrap();
            // Interactive shells place foreground jobs in a different group. The
            // controlling terminal proves that group belongs to THIS session.
            if let (Some(fd), Some(group)) = (master.as_raw_fd(), master.process_group_leader()) {
                if group > 0
                    && group != self.pid as i32
                    && group != unsafe { libc::getpgrp() }
                    && unsafe { libc::tcgetsid(fd) } == self.pid as i32
                {
                    if let Err(error) = crate::child_group::signal(group, signal) {
                        foreground_error = Some(error);
                    }
                }
            }
            crate::child_group::signal(self.pid as i32, signal)?;
            if signal == libc::SIGKILL {
                state.killed = true;
            }
            match foreground_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        #[cfg(windows)]
        {
            let _ = signal;
            self.job.terminate()?;
            state.killed = true;
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = signal;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "PTY ownership unavailable",
            ))
        }
    }
    fn reaped(&self) {
        self.state.lock().unwrap().reaped = true;
    }
}
#[derive(Debug)]
struct OwnedChild {
    inner: Box<dyn Child + Send + Sync>,
    scope: Arc<Scope>,
}
impl ChildKiller for OwnedChild {
    fn kill(&mut self) -> io::Result<()> {
        self.scope.signal(crate::protocol::Signal::Sigkill)
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Killer(self.scope.clone()))
    }
}
#[derive(Debug)]
struct Killer(Arc<Scope>);
impl ChildKiller for Killer {
    fn kill(&mut self) -> io::Result<()> {
        self.0.signal(crate::protocol::Signal::Sigkill)
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self(self.0.clone()))
    }
}
impl Child for OwnedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(unix)]
        {
            if !self.scope.state.lock().unwrap().reaped {
                if !self.scope.observe_anchor()? {
                    return Ok(None);
                }
                self.scope.signal(crate::protocol::Signal::Sigkill)?;
            }
        }
        let result = self.inner.try_wait()?;
        if result.is_some() {
            #[cfg(windows)]
            self.scope.signal(crate::protocol::Signal::Sigkill)?;
            self.scope.reaped();
        }
        Ok(result)
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn process_id(&self) -> Option<u32> {
        self.inner.process_id()
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        self.inner.as_raw_handle()
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.scope.state.lock().unwrap().reaped {
            return;
        }
        if let Err(error) = self.scope.signal(crate::protocol::Signal::Sigkill) {
            tracing::warn!(
                ?error,
                pid = self.scope.pid,
                "PTY ownership cleanup could not signal original child"
            );
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match self.inner.try_wait() {
                Ok(Some(_)) => {
                    self.scope.reaped();
                    return;
                }
                Err(error) => {
                    tracing::warn!(?error, "PTY owner final reap failed");
                    return;
                }
                Ok(None) => (),
            };
            if std::time::Instant::now() >= deadline {
                tracing::warn!(pid = self.scope.pid, "PTY owner final reap timed out");
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
pub(super) fn wrap(
    mut child: Box<dyn Child + Send + Sync>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
) -> Result<(Box<dyn Child + Send + Sync>, Arc<Scope>)> {
    let scope = match Scope::new(child.as_ref(), master) {
        Ok(scope) => Arc::new(scope),
        Err(error) => {
            let _ = child.kill();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return Err(error),
                    Err(reap) => {
                        return Err(error.context(format!(
                            "PTY rejection cleanup could not reap child: {reap}"
                        )));
                    }
                    Ok(None) => (),
                }
                if std::time::Instant::now() >= deadline {
                    return Err(error.context(
                        "PTY rejection cleanup timed out; child outcome remains unknown",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    Ok((
        Box::new(OwnedChild {
            inner: child,
            scope: scope.clone(),
        }),
        scope,
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    #[test]
    fn already_exited_pty_retains_child_group_ownership_before_adoption() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let master = Arc::new(Mutex::new(pair.master));
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let mut child = pair.slave.spawn_command(command).unwrap();
        let pid = child.process_id().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !crate::child_group::verify_anchor(pid).unwrap() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("PTY child did not exit");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let scope = Arc::new(Scope::new(child.as_ref(), master).unwrap());
        let mut owned = OwnedChild {
            inner: child,
            scope,
        };
        assert!(owned.wait().unwrap().success());
        assert!(crate::child_group::verify_anchor(pid).is_err());
    }
}
