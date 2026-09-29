//! Bounded command capture with cleanup before releasing the child PID anchor.
use anyhow::{Context, Result, anyhow, bail};
use claudemon::child_env::SanitizeChildEnvironment;
use std::{
    process::{ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};
#[derive(Debug)]
pub(crate) struct OutputLimit(pub usize);
impl std::fmt::Display for OutputLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command output exceeded {} bytes", self.0)
    }
}
impl std::error::Error for OutputLimit {}
pub(crate) struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
struct Owner {
    pid: u32,
    armed: bool,
    #[cfg(windows)]
    job: claudemon::child_job::Job,
}
#[cfg(unix)]
fn exited(pid: u32) -> Result<bool> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}
#[cfg(unix)]
fn signal_with_anchor_retry(
    mut observe: impl FnMut() -> Result<bool>,
    mut signal: impl FnMut() -> std::io::Result<()>,
    retries: usize,
    mut pause: impl FnMut(),
) -> Result<()> {
    for attempt in 0..=retries {
        // WNOWAIT proof is required again before EVERY numeric group signal.
        // A false result still proves a live unreaped child; an error proves
        // nothing and must stop even a previously admitted retry sequence.
        observe().context("command anchor was reaped; refusing a numeric group signal")?;
        match signal() {
            Ok(()) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EPERM) && attempt < retries => pause(),
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("the last attempt always returns")
}
impl Owner {
    fn new(child: &Child, #[cfg(windows)] job: claudemon::child_job::Job) -> Result<Self> {
        let pid = child.id().context("new command lacks process identity")?;
        #[cfg(target_os = "macos")]
        claudemon::child_group::verify_anchor(pid)?;
        #[cfg(all(unix, not(target_os = "macos")))]
        if pid == 0
            || pid > i32::MAX as u32
            || unsafe { libc::getpgid(pid as i32) } != pid as i32
            || unsafe { libc::getpgrp() } == pid as i32
        {
            bail!("command does not own a separate process group");
        }
        Ok(Self {
            pid,
            armed: true,
            #[cfg(windows)]
            job,
        })
    }
    // A successful signal submission is not proof that inherited pipes closed:
    // a child can fork while the kernel traverses the group. Keep the original
    // unreaped anchor and cancellation cleanup armed during the final drain.
    fn signal_retained(&self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }
        #[cfg(unix)]
        {
            // XNU can return EPERM while a fast command is passing through
            // exit: killpg filters zombies, while libproc's complete group
            // observation may not yet establish that all members exited.
            // Keep the direct-child PID anchor unreaped during a bounded retry.
            // This is deliberately NOT in the shared foreground-group helper:
            // a foreground group need not own this direct-child PID anchor.
            let retries = if cfg!(target_os = "macos") { 20 } else { 0 };
            signal_with_anchor_retry(
                || exited(self.pid),
                || claudemon::child_group::signal(self.pid as i32, libc::SIGKILL),
                retries,
                || std::thread::sleep(Duration::from_millis(5)),
            )?;
        }
        #[cfg(windows)]
        self.job.terminate()?;
        Ok(())
    }
    fn kill(&mut self) -> Result<()> {
        self.signal_retained()?;
        self.armed = false;
        Ok(())
    }
    async fn wait(&mut self, child: &mut Child) -> Result<ExitStatus> {
        #[cfg(unix)]
        {
            while !exited(self.pid)? {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.kill()?;
        }
        let status = child.wait().await?;
        #[cfg(windows)]
        self.kill()?;
        Ok(status)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Err(error) = self.kill() {
            eprintln!("owned command cleanup incomplete: {error}");
        }
    }
}
async fn bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut data).await?;
    if data.len() > limit {
        return Err(OutputLimit(limit).into());
    }
    Ok(data)
}

// Called only inside the exchange's existing overall timeout. Every repetition
// must re-prove the original child anchor; no wait/reap occurs in this loop.
async fn signal_until_drained(
    mut signal: impl FnMut() -> Result<()>,
    drained: &AtomicBool,
) -> Result<()> {
    loop {
        signal()?;
        if drained.load(Ordering::Acquire) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
pub(crate) async fn capture(
    command: &mut Command,
    limit: usize,
    timeout: Duration,
) -> Result<Captured> {
    capture_limits(command, limit, limit, timeout).await
}
pub(crate) async fn capture_limits(
    command: &mut Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
) -> Result<Captured> {
    capture_inner(command, None, stdout_limit, stderr_limit, timeout, false).await
}
pub(crate) async fn capture_input(
    command: &mut Command,
    input: &[u8],
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
) -> Result<Captured> {
    capture_inner(
        command,
        Some(input),
        stdout_limit,
        stderr_limit,
        timeout,
        false,
    )
    .await
}
/// Preserve the kernel pipe's combined stdout/stderr order with the same owned
/// cancellation and descendant cleanup as ordinary capture. stderr is empty.
pub(crate) async fn capture_combined(
    command: &mut Command,
    limit: usize,
    timeout: Duration,
) -> Result<Captured> {
    capture_inner(command, None, limit, 0, timeout, true).await
}

/// Polling an anonymous pipe avoids a detached blocking reader on cancellation.
/// The sole reader only reads available bytes; every wait remains cancellable.
async fn combined_bounded(mut reader: os_pipe::PipeReader, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = reader.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        #[cfg(not(windows))]
        let read_count = buffer.len();
        #[cfg(windows)]
        let read_count = {
            use std::os::windows::io::AsRawHandle;
            let mut available = 0;
            let ok = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    reader.as_raw_handle() as _,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(109 | 233)) {
                    return Ok(data);
                }
                return Err(error.into());
            }
            if available == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            }
            (available as usize).min(buffer.len())
        };
        match reader.read(&mut buffer[..read_count]) {
            Ok(0) => return Ok(data),
            Ok(count) => {
                if data.len().saturating_add(count) > limit {
                    return Err(OutputLimit(limit).into());
                }
                data.extend_from_slice(&buffer[..count]);
                tokio::task::yield_now().await;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(5)).await
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

async fn capture_inner(
    command: &mut Command,
    input: Option<&[u8]>,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    combined: bool,
) -> Result<Captured> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    let combined_reader = if combined {
        let (reader, writer) = os_pipe::pipe()?;
        command.stdout(writer.try_clone()?).stderr(writer);
        Some(reader)
    } else {
        None
    };
    #[cfg(unix)]
    command.process_group(0);
    command.scrub_host_authority();
    #[cfg(windows)]
    let (mut child, job) = claudemon::child_job::Job::spawn_tokio(command, 0)?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;
    if combined {
        // Command retains its configured handles after spawn. Release the
        // parent's write ends now, otherwise a finished child cannot yield EOF.
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut owner = match Owner::new(
        &child,
        #[cfg(windows)]
        job,
    ) {
        Ok(owner) => owner,
        Err(error) => {
            // A failed Windows assignment still has the original process handle.
            // On Unix, never address even the direct PID after a foreign reap.
            #[cfg(unix)]
            let may_kill = child.id().is_some_and(|pid| exited(pid).is_ok());
            #[cfg(not(unix))]
            let may_kill = true;
            if may_kill {
                let _ = child.start_kill();
            }
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(error);
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let result = tokio::time::timeout(timeout, async {
        let (stdout, stderr, status, ()) = tokio::try_join!(
            async {
                if let Some(reader) = combined_reader {
                    combined_bounded(reader, stdout_limit).await
                } else {
                    bounded(stdout.expect("piped stdout"), stdout_limit).await
                }
            },
            async {
                if let Some(stderr) = stderr {
                    bounded(stderr, stderr_limit).await
                } else {
                    Ok(Vec::new())
                }
            },
            owner.wait(&mut child),
            async {
                if let (Some(mut stdin), Some(input)) = (stdin, input) {
                    stdin.write_all(input).await?;
                    stdin.shutdown().await?;
                }
                Ok::<_, anyhow::Error>(())
            }
        )?;
        Ok(Captured {
            status,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        result => {
            let cleanup = owner.kill();
            let reaped = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            if let Err(error) = cleanup {
                return Err(error.context("command cleanup outcome is unknown"));
            }
            if !matches!(reaped, Ok(Ok(_))) {
                bail!("command cleanup did not confirm child exit");
            }
            match result {
                Ok(Err(error)) => Err(error),
                Err(_) => Err(anyhow!("command timed out after {timeout:?}")),
                _ => unreachable!(),
            }
        }
    }
}

/// Bounded metadata-only JSONL dialogue. The caller owns the protocol state;
/// the process cannot outlive the exchange or inherit host bus authority.
pub(crate) async fn json_exchange(
    command: &mut Command,
    initial: &[serde_json::Value],
    mut next: impl FnMut(
        serde_json::Value,
    ) -> Result<(Vec<serde_json::Value>, Option<serde_json::Value>)>
    + Send,
    limit: usize,
    timeout: Duration,
) -> Result<serde_json::Value> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    #[cfg(unix)]
    command.process_group(0);
    command.scrub_host_authority();
    #[cfg(windows)]
    let (mut child, job) = claudemon::child_job::Job::spawn_tokio(command, 0)?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;
    let mut owner = match Owner::new(
        &child,
        #[cfg(windows)]
        job,
    ) {
        Ok(owner) => owner,
        Err(error) => {
            #[cfg(unix)]
            let may_kill = child.id().is_some_and(|id| exited(id).is_ok());
            #[cfg(not(unix))]
            let may_kill = true;
            if may_kill {
                let _ = child.start_kill();
            }
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(error);
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let result = tokio::time::timeout(timeout, async {
        let stderr_drained = AtomicBool::new(false);
        let drain_stderr = async {
            let bytes = bounded(stderr, limit).await?;
            stderr_drained.store(true, Ordering::Release);
            Ok::<_, anyhow::Error>(bytes)
        };
        let exchange = async {
            async fn write(
                writer: &mut tokio::process::ChildStdin,
                frames: &[serde_json::Value],
            ) -> Result<()> {
                for frame in frames {
                    let mut bytes = serde_json::to_vec(frame)?;
                    bytes.push(b'\n');
                    writer.write_all(&bytes).await?;
                }
                writer.flush().await?;
                Ok(())
            }
            write(&mut stdin, initial).await?;
            let mut reader = BufReader::new(stdout.take(limit as u64 + 1));
            let mut received = 0;
            loop {
                let mut bytes = Vec::new();
                let count = reader.read_until(b'\n', &mut bytes).await?;
                received += count;
                if received > limit {
                    return Err(OutputLimit(limit).into());
                }
                if count == 0 {
                    bail!("metadata process closed before completing its exchange");
                }
                let frame = serde_json::from_slice(&bytes)
                    .context("metadata process emitted invalid JSON")?;
                let (frames, answer) = next(frame)?;
                write(&mut stdin, &frames).await?;
                if let Some(answer) = answer {
                    signal_until_drained(|| owner.signal_retained(), &stderr_drained).await?;
                    return Ok(answer);
                }
            }
        };
        let (answer, _) = tokio::try_join!(exchange, drain_stderr)?;
        Ok::<_, anyhow::Error>(answer)
    })
    .await;
    let cleanup = owner.kill();
    let reaped = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    cleanup.context("metadata process cleanup outcome is unknown")?;
    if !matches!(reaped, Ok(Ok(_))) {
        bail!("metadata process cleanup did not confirm child exit");
    }
    match result {
        Ok(result) => result,
        Err(_) => bail!("metadata exchange timed out after {timeout:?}"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn json_exchange_owns_multi_step_dialogue_and_kills_after_final_answer() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c",r#"read first; printf '{"id":1,"result":"ready"}\n'; read second; printf '{"id":2,"result":"answer"}\n'; sleep 30"#]);
        let mut received_ids = Vec::new();
        let mut phase = "awaiting first fixture reply";
        let value = json_exchange(
            &mut command,
            &[serde_json::json!({"id":1})],
            |frame| {
                received_ids.push(frame["id"].as_i64());
                phase = if frame["id"] == 1 {
                    "awaiting final fixture reply"
                } else {
                    "final answer callback completed; awaiting process and stderr cleanup"
                };
                Ok(if frame["id"] == 1 {
                    (vec![serde_json::json!({"id":2})], None)
                } else {
                    (vec![], Some(frame["result"].clone()))
                })
            },
            1024,
            Duration::from_secs(2),
        )
        .await
        .unwrap_or_else(|error| panic!("{error:#}; fixture IDs: {received_ids:?}; phase: {phase}"));
        assert_eq!(value, "answer");
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn owner_can_be_created_after_fast_child_exits_before_verification() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]).process_group(0);
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !exited(pid).unwrap() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let mut owner = Owner::new(&child).unwrap();
        assert!(owner.wait(&mut child).await.unwrap().success());
    }
    #[tokio::test]
    async fn stdin_is_delivered_without_arguments_and_closed_before_exit() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "test $# -eq 0 && cat"]);
        let output = capture_input(
            &mut command,
            b"private prompt",
            1024,
            1024,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"private prompt");
    }
    #[tokio::test]
    async fn leader_exit_closes_ignored_hup_descendant_pipes_before_reap() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "trap '' HUP; sh -c 'trap \"\" HUP; sleep 30' & printf complete; exit 0",
        ]);
        let result = capture(&mut command, 1024, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"complete");
    }
    #[tokio::test]
    async fn output_limit_terminates_owned_group_without_partial_success() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 4096 /dev/zero; sleep 30"]);
        let error = match capture(&mut command, 64, Duration::from_secs(2)).await {
            Ok(_) => panic!("oversized command succeeded"),
            Err(error) => error,
        };
        assert!(error.downcast_ref::<OutputLimit>().is_some(), "{error:#}");
    }
    #[tokio::test]
    async fn timeout_kills_pending_descendants_and_does_not_run_later_side_effects() {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(root.path())
            .args(["-c", "sleep 0.2; touch escaped"]);
        let error = match capture(&mut command, 1024, Duration::from_millis(30)).await {
            Ok(_) => panic!("timed-out command succeeded"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("timed out"));
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(!root.path().join("escaped").exists());
    }
    #[tokio::test]
    async fn a_reaped_anchor_never_authorizes_a_later_group_signal() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]).process_group(0);
        let mut child = command.spawn().unwrap();
        let mut owner = Owner::new(&child).unwrap();
        child.wait().await.unwrap();
        assert!(owner.kill().is_err());
    }
}

#[cfg(all(test, unix))]
mod retry_tests {
    use super::*;

    #[tokio::test]
    async fn post_answer_drain_rechecks_anchor_and_stops_before_signaling_reaped_identity() {
        let drained = AtomicBool::new(false);
        let mut observed = 0;
        let mut signals = 0;
        let error = signal_until_drained(
            || {
                signal_with_anchor_retry(
                    || {
                        observed += 1;
                        if observed == 1 {
                            Ok(false)
                        } else {
                            Err(std::io::Error::from_raw_os_error(libc::ECHILD).into())
                        }
                    },
                    || {
                        signals += 1;
                        Ok(())
                    },
                    0,
                    || unreachable!(),
                )
            },
            &drained,
        )
        .await
        .unwrap_err();
        assert_eq!(observed, 2);
        assert_eq!(signals, 1);
        assert!(error.to_string().contains("anchor was reaped"));
    }

    #[tokio::test]
    async fn post_answer_drain_repeats_until_inherited_pipe_closes() {
        let drained = AtomicBool::new(false);
        let mut signals = 0;
        tokio::time::timeout(
            Duration::from_secs(1),
            signal_until_drained(
                || {
                    signals += 1;
                    if signals == 3 {
                        drained.store(true, Ordering::Release);
                    }
                    Ok(())
                },
                &drained,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(signals, 3);
    }
    use std::cell::Cell;
    #[test]
    fn transient_permission_retry_is_bounded_and_rechecks_anchor_before_every_signal() {
        let probes = Cell::new(0);
        let signals = Cell::new(0);
        let waits = Cell::new(0);
        signal_with_anchor_retry(
            || {
                probes.set(probes.get() + 1);
                Ok(false)
            },
            || {
                signals.set(signals.get() + 1);
                if signals.get() < 3 {
                    Err(std::io::Error::from_raw_os_error(libc::EPERM))
                } else {
                    Ok(())
                }
            },
            2,
            || waits.set(waits.get() + 1),
        )
        .unwrap();
        assert_eq!((probes.get(), signals.get(), waits.get()), (3, 3, 2));
        signals.set(0);
        let error = signal_with_anchor_retry(
            || Ok(true),
            || {
                signals.set(signals.get() + 1);
                Err(std::io::Error::from_raw_os_error(libc::EPERM))
            },
            2,
            || {},
        )
        .unwrap_err();
        assert_eq!(signals.get(), 3);
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EPERM)
        );
    }
    #[test]
    fn lost_anchor_or_other_error_never_authorizes_an_extra_numeric_signal() {
        let probes = Cell::new(0);
        let signals = Cell::new(0);
        let error = signal_with_anchor_retry(
            || {
                probes.set(probes.get() + 1);
                if probes.get() == 1 {
                    Ok(true)
                } else {
                    Err(std::io::Error::from_raw_os_error(libc::ECHILD).into())
                }
            },
            || {
                signals.set(signals.get() + 1);
                Err(std::io::Error::from_raw_os_error(libc::EPERM))
            },
            20,
            || {},
        )
        .unwrap_err();
        assert_eq!(signals.get(), 1);
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::ECHILD)
        );
        signals.set(0);
        let error = signal_with_anchor_retry(
            || Ok(false),
            || {
                signals.set(signals.get() + 1);
                Err(std::io::Error::from_raw_os_error(libc::EACCES))
            },
            20,
            || panic!("non-EPERM cannot retry"),
        )
        .unwrap_err();
        assert_eq!(signals.get(), 1);
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EACCES)
        );
    }
    #[tokio::test]
    async fn repeated_fast_oversized_commands_preserve_output_limit_after_owned_cleanup() {
        for _ in 0..16 {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf '%65536s' x"]);
            let error = match capture(&mut command, 128, Duration::from_secs(3)).await {
                Ok(_) => panic!("oversized command cannot produce a successful partial capture"),
                Err(error) => error,
            };
            assert!(error.downcast_ref::<OutputLimit>().is_some(), "{error:#}");
        }
    }
}

#[cfg(test)]
mod combined_tests {
    use super::*;
    use std::io::Write;
    const MODE: &str = "WKS_COMBINED_CAPTURE_FIXTURE";
    fn fixture_name() -> &'static str {
        concat!(module_path!(), "::combined_fixture")
            .split_once("::")
            .unwrap()
            .1
    }
    fn command(mode: &str, directory: &std::path::Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", fixture_name(), "--nocapture"])
            .env(MODE, mode)
            .env("WKS_COMBINED_CAPTURE_DIRECTORY", directory);
        command
    }
    #[test]
    fn combined_fixture() {
        let Ok(mode) = std::env::var(MODE) else {
            return;
        };
        let directory =
            std::path::PathBuf::from(std::env::var_os("WKS_COMBINED_CAPTURE_DIRECTORY").unwrap());
        match mode.as_str() {
            "ordered" => {
                std::io::stdout().write_all(b"STDOUT-FIRST\n").unwrap();
                std::io::stdout().flush().unwrap();
                std::io::stderr().write_all(b"STDERR-MIDDLE\n").unwrap();
                std::io::stderr().flush().unwrap();
                std::io::stdout().write_all(b"STDOUT-LAST\n").unwrap();
                std::io::stdout().flush().unwrap();
                std::process::exit(7);
            }
            "parent" => {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", fixture_name(), "--nocapture"])
                    .env(MODE, "descendant")
                    .spawn()
                    .unwrap();
                std::fs::write(directory.join("pid"), child.id().to_string()).unwrap();
                child.wait().unwrap();
            }
            "descendant" => {
                std::fs::write(directory.join("ready"), "ready").unwrap();
                while !directory.join("release").exists() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                std::fs::write(
                    directory.join("escaped"),
                    "descendant survived cancellation",
                )
                .unwrap();
            }
            _ => panic!("unknown fixture mode"),
        }
    }
    #[tokio::test]
    async fn combined_capture_preserves_cross_stream_order_and_nonzero_exit() {
        let directory = tempfile::tempdir().unwrap();
        let output = capture_combined(
            &mut command("ordered", directory.path()),
            4096,
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert!(output.stderr.is_empty());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("STDOUT-FIRST\nSTDERR-MIDDLE\nSTDOUT-LAST\n")
        );
    }
    #[tokio::test]
    async fn combined_capture_rejects_oversized_output_instead_of_partial_success() {
        let directory = tempfile::tempdir().unwrap();
        let result = capture_combined(
            &mut command("ordered", directory.path()),
            8,
            Duration::from_secs(3),
        )
        .await;
        let error = match result {
            Ok(_) => panic!("output limit ignored"),
            Err(error) => error,
        };
        assert!(error.downcast_ref::<OutputLimit>().is_some(), "{error:#}");
    }
    #[tokio::test]
    async fn cancelled_combined_capture_stops_descendants_holding_the_pipe() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = command("parent", directory.path());
        let task = tokio::spawn(async move {
            capture_combined(&mut command, 4096, Duration::from_secs(30)).await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !directory.path().join("ready").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        std::fs::write(directory.path().join("release"), "release").unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(
            !directory.path().join("escaped").exists(),
            "descendant executed after cancellation"
        );
    }
}
