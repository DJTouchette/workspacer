//! Direct-child ownership, bounded exponential restart, and deterministic process seam.
#[cfg(windows)]
#[path = "windows_job.rs"]
pub(crate) mod windows_job;
use anyhow::Result;
use claudemon::child_env::SanitizeChildEnvironment;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
#[path = "process_logs.rs"]
mod process_logs;
use process_logs::LogPipe;
pub use process_logs::LogSink;

#[derive(Clone)]
pub struct Spec {
    pub command: String,
    pub args: Vec<String>,
    pub directory: PathBuf,
    pub env: BTreeMap<String, String>,
    pub health_url: Option<String>,
    pub log: Option<LogSink>,
}
pub trait Process: Send {
    fn exited(&mut self) -> Result<bool>;
    fn stop(&mut self) -> Result<()>;
    fn pid(&self) -> u32 {
        0
    }
    fn exit_error(&self) -> String {
        String::new()
    }
}
pub trait Factory: Send + Sync {
    fn spawn(&self, spec: &Spec) -> Result<Box<dyn Process>>;
    fn healthy(&self, url: &str) -> bool {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .and_then(|client| client.get(url).send())
            .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
    }
}
pub struct NativeFactory;
struct NativeChild {
    child: std::process::Child,
    _parent_pipe: os_pipe::PipeWriter,
    logs: Vec<LogPipe>,
    exit_status: Option<std::process::ExitStatus>,
    #[cfg(windows)]
    _job: Option<windows_job::Job>,
}
impl Process for NativeChild {
    fn pid(&self) -> u32 {
        self.child.id()
    }
    fn exit_error(&self) -> String {
        match self.exit_status {
            Some(status) if !status.success() => status
                .code()
                .map(|code| format!("exit status {code}"))
                .unwrap_or_else(|| status.to_string()),
            _ => String::new(),
        }
    }
    fn exited(&mut self) -> Result<bool> {
        for pipe in &mut self.logs {
            pipe.drain(false);
        }
        self.exit_status = self.child.try_wait()?;
        Ok(self.exit_status.is_some())
    }
    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            #[cfg(unix)]
            {
                // The child is unreaped, so its PID cannot yet be recycled.
                let status = unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
                if status < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
                {
                    self.child.kill()?;
                }
                let deadline = Instant::now() + Duration::from_secs(5);
                while self.child.try_wait()?.is_none() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(20));
                }
                if self.child.try_wait()?.is_none() {
                    self.child.kill()?;
                }
            }
            #[cfg(not(unix))]
            self.child.kill()?;
        }
        self.child.wait()?;
        for pipe in &mut self.logs {
            pipe.drain(true);
        }
        Ok(())
    }
}
impl Drop for NativeChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
impl Factory for NativeFactory {
    fn spawn(&self, spec: &Spec) -> Result<Box<dyn Process>> {
        let (reader, writer) = os_pipe::pipe()?;
        let mut command = Command::new(&spec.command);
        command
            .scrub_host_authority()
            .args(&spec.args)
            .current_dir(&spec.directory)
            .envs(&spec.env)
            .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
            .stdin(Stdio::from(reader))
            .stdout(if spec.log.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(if spec.log.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        #[cfg(windows)]
        let (child, job) = windows_job::Job::spawn(&mut command, 0)?;
        #[cfg(not(windows))]
        let child = command.spawn()?;
        let mut owned = NativeChild {
            child,
            _parent_pipe: writer,
            logs: vec![],
            exit_status: None,
            #[cfg(windows)]
            _job: Some(job),
        };
        if let Some(sink) = &spec.log {
            if let Some(stdout) = owned.child.stdout.take() {
                owned
                    .logs
                    .push(LogPipe::new(stdout, "stdout", sink.clone())?);
            }
            if let Some(stderr) = owned.child.stderr.take() {
                owned
                    .logs
                    .push(LogPipe::new(stderr, "stderr", sink.clone())?);
            }
        }
        Ok(Box::new(owned))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Starting,
    Running,
    Healthy,
    Unhealthy,
    Crashed,
    Stopped,
}
#[derive(Clone, Copy)]
pub struct Timing {
    pub initial: Duration,
    pub maximum: Duration,
    pub reset_after: Duration,
    pub poll: Duration,
    pub health_period: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            maximum: Duration::from_secs(30),
            reset_after: Duration::from_secs(30),
            poll: Duration::from_millis(50),
            health_period: Duration::from_secs(2),
        }
    }
}
impl Timing {
    fn base_delay(self) -> Duration {
        self.initial.min(self.maximum)
    }
    fn after_run(self, delay: Duration, uptime: Duration) -> Duration {
        if uptime >= self.reset_after {
            self.base_delay()
        } else {
            delay
        }
    }
    fn advance(self, delay: Duration) -> Duration {
        delay.saturating_mul(2).min(self.maximum)
    }
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub state: State,
    #[serde(skip_serializing_if = "is_zero")]
    pub pid: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub err: String,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
            Self::Crashed => "crashed",
            Self::Stopped => "stopped",
        }
    }
}
pub struct Supervisor {
    cancel: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    state: Arc<Mutex<State>>,
}
impl Supervisor {
    pub fn start(
        spec: Spec,
        factory: Arc<dyn Factory>,
        timing: Timing,
        notify: Arc<dyn Fn(State) + Send + Sync>,
    ) -> Result<Self> {
        Self::start_observed(
            spec,
            factory,
            timing,
            Arc::new(move |status| notify(status.state)),
        )
    }
    pub fn start_observed(
        spec: Spec,
        factory: Arc<dyn Factory>,
        timing: Timing,
        notify: Arc<dyn Fn(Status) + Send + Sync>,
    ) -> Result<Self> {
        let (cancel, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::Starting));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("hub-plugin".into())
            .spawn(move || {
                let emit = |state, pid, err| {
                    *shared.lock().unwrap() = state;
                    notify(Status { state, pid, err });
                };
                let mut delay = timing.base_delay();
                'outer: loop {
                    if rx.try_recv().is_ok() {
                        break;
                    }
                    let error = match factory.spawn(&spec) {
                        Ok(mut child) => {
                            emit(State::Running, child.pid(), String::new());
                            let started = Instant::now();
                            let mut next_health = started + timing.health_period;
                            let mut was_healthy = false;
                            let error = loop {
                                match rx.recv_timeout(timing.poll) {
                                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                        let _ = child.stop();
                                        break 'outer;
                                    }
                                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                                }
                                match child.exited() {
                                    Ok(false) => {
                                        if let Some(url) = &spec.health_url {
                                            if Instant::now() >= next_health {
                                                let healthy = factory.healthy(url);
                                                if healthy != was_healthy {
                                                    emit(
                                                        if healthy {
                                                            State::Healthy
                                                        } else {
                                                            State::Unhealthy
                                                        },
                                                        child.pid(),
                                                        String::new(),
                                                    );
                                                }
                                                was_healthy = healthy;
                                                next_health = Instant::now() + timing.health_period;
                                            }
                                        }
                                    }
                                    Ok(true) => {
                                        let error = child.exit_error();
                                        let _ = child.stop();
                                        break error;
                                    }
                                    Err(error) => {
                                        let _ = child.stop();
                                        break error.to_string();
                                    }
                                }
                            };
                            delay = timing.after_run(delay, started.elapsed());
                            error
                        }
                        Err(error) => error.to_string(),
                    };
                    emit(State::Crashed, 0, error);
                    match rx.recv_timeout(delay) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    delay = timing.advance(delay);
                }
                emit(State::Stopped, 0, String::new());
            })?;
        Ok(Self {
            cancel,
            worker: Some(worker),
            state,
        })
    }
    pub fn state(&self) -> State {
        *self.state.lock().unwrap()
    }
    pub fn stop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sidecar_output_is_discarded_without_log_sink() {
        const CHILD: &str = "WKS_SUPERVISOR_QUIET_TEST_DIRECTORY";
        const MARKER: &str = "SIDECAR-CHATTER-NOBODY-ASKED-FOR";
        if let Some(directory) = std::env::var_os(CHILD) {
            #[cfg(unix)]
            let (command, args) = (
                "/bin/sh",
                vec![
                    "-c",
                    "printf 'SIDECAR-CHATTER-NOBODY-ASKED-FOR\\n'; printf 'SIDECAR-CHATTER-NOBODY-ASKED-FOR\\n' >&2; : > sidecar-ran",
                ],
            );
            #[cfg(windows)]
            let (command, args) = (
                "cmd.exe",
                vec![
                    "/D",
                    "/C",
                    "echo SIDECAR-CHATTER-NOBODY-ASKED-FOR & echo SIDECAR-CHATTER-NOBODY-ASKED-FOR 1>&2 & type nul > sidecar-ran",
                ],
            );
            let directory = PathBuf::from(directory);
            let mut child = NativeFactory
                .spawn(&Spec {
                    command: command.into(),
                    args: args.into_iter().map(str::to_owned).collect(),
                    directory: directory.clone(),
                    env: Default::default(),
                    health_url: None,
                    log: None,
                })
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !child.exited().unwrap() {
                assert!(Instant::now() < deadline, "quiet sidecar did not exit");
                thread::sleep(Duration::from_millis(10));
            }
            child.stop().unwrap();
            assert!(
                directory.join("sidecar-ran").is_file(),
                "the sidecar must actually execute"
            );
            println!("supervisor-fixture-ran");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "plugins::supervisor::tests::native_sidecar_output_is_discarded_without_log_sink",
                "--nocapture",
            ])
            .env(CHILD, directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("supervisor-fixture-ran"),
            "fixture did not execute: {stdout}"
        );
        assert!(!stdout.contains(MARKER));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(MARKER));
    }

    #[test]
    fn initial_restart_wait_uses_the_cap_in_the_actual_worker_loop() {
        struct Missing;
        impl Factory for Missing {
            fn spawn(&self, _: &Spec) -> Result<Box<dyn Process>> {
                anyhow::bail!("fixture missing executable")
            }
        }
        let (tx, rx) = mpsc::channel();
        let mut supervisor = Supervisor::start(
            Spec {
                command: "fixture".into(),
                args: vec![],
                directory: ".".into(),
                env: Default::default(),
                health_url: None,
                log: None,
            },
            Arc::new(Missing),
            Timing {
                initial: Duration::from_secs(60),
                maximum: Duration::from_millis(5),
                ..Timing::default()
            },
            Arc::new(move |state| {
                let _ = tx.send(state);
            }),
        )
        .unwrap();
        let first = rx.recv_timeout(Duration::from_secs(2));
        let second = rx.recv_timeout(Duration::from_secs(2));
        supervisor.stop();
        assert_eq!(first.unwrap(), State::Crashed);
        assert_eq!(second.unwrap(), State::Crashed);
        assert_eq!(supervisor.state(), State::Stopped);
    }

    #[test]
    fn reference_backoff_caps_overflow_and_resets_at_the_uptime_boundary() {
        let timing = Timing::default();
        for (current, expected) in [(1, 2), (2, 4), (16, 30), (30, 30)] {
            assert_eq!(
                timing.advance(Duration::from_secs(current)),
                Duration::from_secs(expected)
            );
        }
        assert_eq!(
            timing.advance(Duration::from_nanos(1 << 62)),
            Duration::from_secs(30)
        );
        assert_eq!(timing.advance(Duration::MAX), Duration::from_secs(30));
        assert_eq!(
            timing.after_run(Duration::from_secs(16), Duration::from_millis(29999)),
            Duration::from_secs(16)
        );
        assert_eq!(
            timing.after_run(Duration::from_secs(16), Duration::from_secs(30)),
            Duration::from_secs(1)
        );
        let capped = Timing {
            initial: Duration::from_secs(60),
            maximum: Duration::from_secs(5),
            ..timing
        };
        assert_eq!(capped.base_delay(), Duration::from_secs(5));
        assert_eq!(
            capped.after_run(Duration::from_secs(5), Duration::from_secs(31)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn native_health_probe_keeps_the_reference_two_second_budget() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "health probe never connected");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("health fixture accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 1024];
            assert!(socket.read(&mut request).unwrap() > 0);
            thread::sleep(Duration::from_millis(1100));
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        });
        let healthy = NativeFactory.healthy(&format!("http://{address}/health"));
        server.join().unwrap();
        assert!(
            healthy,
            "a healthy response after one second still fits the reference budget"
        );
    }
}
