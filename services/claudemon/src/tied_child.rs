//! Provider children that cannot outlive this process.
//!
//! A clean shutdown stops every managed provider itself. This covers the rest:
//! a crash, SIGKILL, or a terminal closed under the app. A provider that keeps
//! no pipe to us (Codex's app-server listens on a socket, its stdin is null)
//! never notices its parent is gone, and used to run on with nothing connected.
//!
//! - Linux: the kernel sends the child SIGKILL when its parent dies
//!   (`PR_SET_PDEATHSIG`). That fires when the *thread* that forked it exits,
//!   so every tied spawn happens on one thread that lives as long as the
//!   process — never a runtime's blocking-pool thread, which retires when idle.
//! - Windows: the child starts suspended inside a kill-on-close job, so it and
//!   everything it starts die when the job's last handle closes: when [`Tie`]
//!   drops, or with the process.
//! - Elsewhere: an ordinary spawn.
use std::io;

/// Keeps a tied child's OS-level ownership alive. Hold it as long as the child.
pub struct Tie {
    #[cfg(windows)]
    job: crate::child_job::Job,
}

impl Tie {
    /// Kill the child and everything it started. Unix owners signal their own
    /// process group instead.
    #[cfg(windows)]
    pub fn terminate(&self) -> io::Result<()> {
        self.job.terminate()
    }
}

/// Spawn `cmd` tied to this process. The command's own settings are kept.
pub fn spawn(cmd: &mut tokio::process::Command) -> io::Result<(tokio::process::Child, Tie)> {
    #[cfg(target_os = "linux")]
    {
        linux::spawn(cmd).map(|child| (child, Tie {}))
    }
    #[cfg(windows)]
    {
        let (child, job) =
            crate::child_job::Job::spawn_tokio(cmd, crate::background_process::CREATION_FLAGS)
                .map_err(io::Error::other)?;
        Ok((child, Tie { job }))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        cmd.spawn().map(|child| (child, Tie {}))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::sync::{mpsc, Mutex, OnceLock};

    type Work = Box<dyn FnOnce() + Send>;

    /// A pointer to the caller's command, valid because the caller blocks
    /// until the spawner thread has finished with it.
    struct Borrowed(*mut tokio::process::Command);
    unsafe impl Send for Borrowed {}

    fn spawner() -> Option<mpsc::Sender<Work>> {
        static SPAWNER: OnceLock<Option<Mutex<mpsc::Sender<Work>>>> = OnceLock::new();
        SPAWNER
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel::<Work>();
                std::thread::Builder::new()
                    .name("claudemon-spawner".into())
                    .spawn(move || {
                        while let Ok(work) = rx.recv() {
                            work();
                        }
                        // Never return: children forked here die with this
                        // thread. The sender is static, so this is not reached.
                        loop {
                            std::thread::park();
                        }
                    })
                    .ok()
                    .map(|_| Mutex::new(tx))
            })
            .as_ref()
            .and_then(|tx| tx.lock().ok().map(|tx| tx.clone()))
    }

    pub(super) fn spawn(cmd: &mut tokio::process::Command) -> io::Result<tokio::process::Child> {
        let Some(spawner) = spawner() else {
            return cmd.spawn();
        };
        let runtime = tokio::runtime::Handle::try_current().ok();
        let parent = std::process::id() as i32;
        let borrowed = Borrowed(cmd as *mut _);
        let (done, result) = mpsc::sync_channel(1);
        let work: Work = Box::new(move || {
            let borrowed = borrowed;
            // SAFETY: the caller is blocked on `result` below until this
            // closure has sent, so the command is alive and not aliased.
            let cmd = unsafe { &mut *borrowed.0 };
            // SAFETY: only async-signal-safe calls between fork and exec.
            unsafe {
                cmd.pre_exec(move || {
                    use nix::libc;
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    // The parent may have died before the request took hold.
                    if libc::getppid() != parent {
                        return Err(io::Error::other("parent exited during spawn"));
                    }
                    Ok(())
                });
            }
            let _entered = runtime.as_ref().map(|r| r.enter());
            let _ = done.send(cmd.spawn());
        });
        if spawner.send(work).is_err() {
            return cmd.spawn();
        }
        result
            .recv()
            .unwrap_or_else(|_| Err(io::Error::other("spawner thread stopped")))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    fn alive(pid: i32) -> bool {
        // A zombie still answers kill(0); read its state instead.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(") ")
                    .map(|(_, rest)| !rest.starts_with('Z'))
            })
            .unwrap_or(false)
    }

    /// The child outlives the task (and runtime thread) that spawned it, and
    /// is killed when its parent process dies without cleaning up.
    #[test]
    fn a_tied_child_dies_with_its_parent_but_not_with_the_spawning_thread() {
        // The parent is a throwaway process: this test binary re-run as a
        // helper that spawns a tied `sleep`, prints its pid, then is killed.
        if std::env::var_os("CLAUDEMON_TIED_CHILD_HELPER").is_some() {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let child = tokio::task::spawn_blocking(|| {
                    let mut cmd = tokio::process::Command::new("sleep");
                    cmd.arg("600").stdin(Stdio::null());
                    // A blocking-pool thread that exits right after.
                    let handle = tokio::runtime::Handle::current();
                    let _g = handle.enter();
                    spawn(&mut cmd).unwrap()
                })
                .await
                .unwrap();
                println!("{}", child.0.id().unwrap());
                std::mem::forget(child);
                tokio::time::sleep(Duration::from_secs(600)).await;
            });
            return;
        }
        let mut helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tied_child::tests::a_tied_child_dies_with_its_parent_but_not_with_the_spawning_thread",
                "--nocapture",
            ])
            .env("CLAUDEMON_TIED_CHILD_HELPER", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut out = std::io::BufReader::new(helper.stdout.take().unwrap());
        let pid = loop {
            let mut line = String::new();
            use std::io::BufRead;
            assert!(out.read_line(&mut line).unwrap() > 0, "helper exited early");
            if let Ok(pid) = line.trim().parse::<i32>() {
                break pid;
            }
        };
        // Outlives the blocking thread that asked for it.
        std::thread::sleep(Duration::from_millis(300));
        assert!(alive(pid), "the child died with the spawning thread");
        helper.kill().unwrap();
        helper.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) {
            if Instant::now() > deadline {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
                panic!("the tied child outlived its parent");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
