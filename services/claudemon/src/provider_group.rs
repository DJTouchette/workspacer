//! A provider child that is torn down together with everything it started.
//!
//! A provider CLI starts its own children: MCP servers over stdio, background
//! shells (`run_in_background` dev servers, watchers), tool subprocesses.
//! Killing only the direct child leaves those reparented to init and running
//! with nothing attached — the leak an idle shutdown would otherwise repeat
//! for every session it pauses.
//!
//! - Unix: the child leads its own process group. Teardown signals the GROUP,
//!   and only while the direct child is still unreaped: an unreaped child pins
//!   its PID, so the numeric group id cannot have been recycled for an
//!   unrelated process. Exit is therefore observed with `WNOWAIT`, and the
//!   child is reaped only after the final group signal.
//! - Windows: [`crate::tied_child`] already puts the child in a kill-on-close
//!   job; terminating the job reaches the whole tree.
//!
//! Teardown is graceful first. The caller closes the child's stdin (the
//! provider's own "wind down" signal, which lets it flush its transcript and
//! stop its MCP servers), then [`ProviderGroup::shutdown`] waits a short grace,
//! sends SIGTERM to the group, and finally SIGKILL to whatever is left. Drop
//! is the backstop for a cancelled runtime: it SIGKILLs the group.
use std::io;
use std::time::Duration;

/// How long [`ProviderGroup::shutdown`] lets the group react to SIGTERM before
/// SIGKILL.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// Poll interval while observing exit without reaping.
const EXIT_POLL: Duration = Duration::from_millis(50);

/// How a group teardown ended. Logged; callers do not branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teardown {
    /// The provider exited by itself within the grace (stdin EOF honoured).
    Graceful,
    /// It needed SIGTERM.
    Terminated,
    /// It needed SIGKILL.
    Killed,
    /// A signal could not be delivered, or the reap failed.
    Failed,
}

pub struct ProviderGroup {
    child: tokio::process::Child,
    pid: u32,
    /// Set once the direct child has been reaped: no group signal may follow.
    reaped: bool,
    // Holds the OS-level tie (PDEATHSIG thread on Linux, job on Windows).
    #[cfg_attr(unix, allow(dead_code))]
    tie: crate::tied_child::Tie,
}

impl ProviderGroup {
    /// Spawn `cmd` as a group leader tied to this process. The command's own
    /// settings (pipes, env, cwd, `kill_on_drop`) are kept.
    pub fn spawn(cmd: &mut tokio::process::Command) -> io::Result<Self> {
        #[cfg(unix)]
        cmd.process_group(0);
        let (child, tie) = crate::tied_child::spawn(cmd)?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("spawned provider has no pid"))?;
        Ok(Self {
            child,
            pid,
            reaped: false,
            tie,
        })
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    /// The child, for taking its pipes. Never wait on it directly: reaping it
    /// outside [`Self::shutdown`] would void the group-signal guarantee.
    pub fn child_mut(&mut self) -> &mut tokio::process::Child {
        &mut self.child
    }

    /// Whether the direct child has exited, WITHOUT reaping it on Unix.
    pub fn has_exited(&mut self) -> io::Result<bool> {
        if self.reaped {
            return Ok(true);
        }
        #[cfg(unix)]
        {
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
        #[cfg(not(unix))]
        {
            let exited = self.child.try_wait()?.is_some();
            self.reaped = exited;
            Ok(exited)
        }
    }

    /// Resolves once the direct child has exited (not reaped on Unix). An
    /// observation error resolves too: the caller then tears down.
    pub async fn exited(&mut self) {
        loop {
            match self.has_exited() {
                Ok(false) => tokio::time::sleep(EXIT_POLL).await,
                Ok(true) | Err(_) => return,
            }
        }
    }

    async fn exited_within(&mut self, limit: Duration) -> bool {
        tokio::time::timeout(limit, self.exited()).await.is_ok()
    }

    /// Signal the whole group. Refused once the anchor has been reaped.
    #[cfg(unix)]
    fn signal_group(&mut self, signal: nix::sys::signal::Signal) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        use nix::{errno::Errno, sys::signal::killpg, unistd::Pid};
        match killpg(Pid::from_raw(self.pid as i32), signal) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(err) => Err(io::Error::from_raw_os_error(err as i32)),
        }
    }

    fn kill_tree(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.signal_group(nix::sys::signal::Signal::SIGKILL)
        }
        #[cfg(windows)]
        {
            self.tie.terminate().or_else(|_| self.child.start_kill())
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.child.start_kill()
        }
    }

    /// Tear the group down. Call after closing the child's stdin: `grace` is
    /// how long the provider gets to exit on its own before SIGTERM. Always
    /// ends with a SIGKILL to the group (a no-op when it is already empty)
    /// so nothing the provider started survives it, then reaps.
    pub async fn shutdown(&mut self, grace: Duration) -> Teardown {
        if self.reaped {
            return Teardown::Graceful;
        }
        let mut outcome = Teardown::Graceful;
        if !self.exited_within(grace).await {
            outcome = Teardown::Terminated;
            #[cfg(unix)]
            let termed = self.signal_group(nix::sys::signal::Signal::SIGTERM).is_ok();
            #[cfg(not(unix))]
            let termed = false;
            if !termed || !self.exited_within(TERM_GRACE).await {
                outcome = Teardown::Killed;
            }
        }
        // The provider is gone (or will not go); its descendants may not be.
        // SIGKILL whatever is left while the unreaped anchor still pins the id.
        if self.kill_tree().is_err() {
            outcome = Teardown::Failed;
        }
        match tokio::time::timeout(TERM_GRACE, self.child.wait()).await {
            Ok(Ok(_)) => self.reaped = true,
            _ => outcome = Teardown::Failed,
        }
        outcome
    }
}

impl Drop for ProviderGroup {
    /// A cancelled driver (runtime shutdown) never reaches `shutdown`. Kill
    /// the group synchronously; `kill_on_drop` on the command reaps the child.
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.kill_tree();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// Running, as opposed to gone or a zombie awaiting a reaper that may
    /// not exist in a container (signal 0 succeeds on zombies).
    fn alive(pid: i32) -> bool {
        #[cfg(target_os = "linux")]
        {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                // `pid (comm) S ...`: the state follows the last ')'.
                Ok(stat) => stat
                    .rsplit_once(')')
                    .and_then(|(_, rest)| rest.trim_start().chars().next())
                    .is_some_and(|state| state != 'Z' && state != 'X'),
                Err(_) => false,
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            unsafe { nix::libc::kill(pid, 0) == 0 }
        }
    }

    /// A grandchild the provider started (an MCP server, a background shell)
    /// dies with the provider instead of being reparented and left running.
    #[tokio::test]
    async fn shutdown_kills_descendants_not_just_the_direct_child() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 600 & echo $!; exec cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut group = ProviderGroup::spawn(&mut cmd).unwrap();
        let stdout = group.child_mut().stdout.take().unwrap();
        let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdout));
        let grandchild: i32 = lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(grandchild));

        // Close stdin: `cat` exits by itself, the backgrounded sleep would not.
        drop(group.child_mut().stdin.take());
        let outcome = group.shutdown(Duration::from_secs(5)).await;
        assert_eq!(outcome, Teardown::Graceful);
        for _ in 0..100 {
            if !alive(grandchild) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived its provider's teardown");
    }

    /// A provider that ignores stdin EOF is terminated after the grace.
    #[tokio::test]
    async fn shutdown_terminates_a_provider_that_ignores_eof() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg("exec sleep 600")
            .stdin(Stdio::piped())
            .kill_on_drop(true);
        let mut group = ProviderGroup::spawn(&mut cmd).unwrap();
        drop(group.child_mut().stdin.take());
        let outcome = group.shutdown(Duration::from_millis(100)).await;
        assert_eq!(outcome, Teardown::Terminated);
        assert!(group.has_exited().unwrap());
    }

    /// Dropping without `shutdown` (a cancelled runtime) still kills the group.
    #[tokio::test]
    async fn drop_kills_the_group() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 600 & echo $!; exec sleep 600")
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut group = ProviderGroup::spawn(&mut cmd).unwrap();
        let stdout = group.child_mut().stdout.take().unwrap();
        let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdout));
        let grandchild: i32 = lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        drop(group);
        for _ in 0..100 {
            if !alive(grandchild) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived a dropped provider group");
    }
}
