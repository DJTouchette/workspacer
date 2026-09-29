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
            .timeout(Duration::from_secs(1))
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
        let child = Command::new(&spec.command)
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
            })
            .spawn()?;
        let mut owned = NativeChild {
            child,
            _parent_pipe: writer,
            logs: vec![],
            exit_status: None,
            #[cfg(windows)]
            _job: None,
        };
        #[cfg(windows)]
        {
            match windows_job::Job::assign(&owned.child) {
                Ok(job) => owned._job = Some(job),
                Err(error) => eprintln!("plugin process job confinement unavailable: {error}"),
            }
        }
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
                let mut delay = timing.initial;
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
                            if started.elapsed() >= timing.reset_after {
                                delay = timing.initial;
                            }
                            error
                        }
                        Err(error) => error.to_string(),
                    };
                    emit(State::Crashed, 0, error);
                    match rx.recv_timeout(delay) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    delay = delay.saturating_mul(2).min(timing.maximum);
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
