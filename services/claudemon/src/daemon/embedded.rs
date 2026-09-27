//! An owned, in-process daemon. No signals, stdin, or process-wide job objects
//! are installed here; the host explicitly owns its lifetime.
use super::{DaemonLease, ServeConfig};
use anyhow::{anyhow, Context, Result};
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use serde_json::{json, Value};
use std::{net::SocketAddr, thread::JoinHandle};
use tokio::sync::{mpsc, oneshot, watch};
use tower::ServiceExt;

#[derive(Debug, Clone)]
pub struct ReadyInfo {
    pub hook_addr: SocketAddr,
    pub api_addr: SocketAddr,
}
#[derive(Debug, Clone)]
pub enum Status {
    Starting,
    Ready(ReadyInfo),
    Stopped,
    Failed(String),
}
/// Host-owned settings that must not require changing process-global state.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// None preserves the standalone environment/default behavior.
    pub usage_poll_on_boot: Option<bool>,
}
/// Only local session operations are exposed. Workspacer launch setup must
/// continue through the hub/brain's agents.spawn capability.
#[derive(Debug)]
pub enum Command {
    Sessions,
    Conversation { id: String, since: Option<u64> },
    Message { id: String, text: String },
    Approve { id: String, decision: String },
    Answer { id: String, answer: Value },
    Interrupt { id: String },
}
struct Envelope {
    command: Command,
    reply: oneshot::Sender<Result<Value>>,
}
#[derive(Clone)]
pub struct EmbeddedClient {
    commands: mpsc::Sender<Envelope>,
    status: watch::Receiver<Status>,
}
impl EmbeddedClient {
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
    pub async fn request(&self, command: Command) -> Result<Value> {
        let (reply, result) = oneshot::channel();
        self.commands
            .try_send(Envelope { command, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    anyhow!("embedded daemon command queue is full; command was not submitted")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    anyhow!("embedded daemon has stopped; command was not submitted")
                }
            })?;
        tokio::time::timeout(std::time::Duration::from_secs(30), result)
            .await
            .context(
                "embedded daemon reply timed out; outcome is unknown, do not automatically retry",
            )?
            .context("embedded daemon stopped before replying; outcome is unknown")?
    }
}
pub struct EmbeddedDaemon {
    client: EmbeddedClient,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}
impl EmbeddedDaemon {
    /// Returns immediately; await `ready` before connecting the local hub.
    pub fn start(cfg: ServeConfig) -> Result<Self> {
        Self::start_with_options(cfg, Options::default())
    }
    pub fn start_with_options(cfg: ServeConfig, options: Options) -> Result<Self> {
        let lease = DaemonLease::acquire()?;
        let (commands, receiver) = mpsc::channel(64);
        let (status_tx, status) = watch::channel(Status::Starting);
        let (shutdown, shutdown_rx) = oneshot::channel();
        let cleanup =
            std::sync::Arc::new(std::sync::Mutex::new(None::<crate::session::SessionStore>));
        let thread = std::thread::Builder::new()
            .name("claudemon-backend".into())
            .spawn(move || {
                // The lease outlives runtime destruction, including provider tasks.
                let mut cleanup_timed_out = false;
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .thread_name("claudemon-worker")
                        .enable_all()
                        .build()?;
                    let result = runtime.block_on(super::run_controlled(
                        cfg,
                        async {
                            let _ = shutdown_rx.await;
                        },
                        Some(Control {
                            receiver,
                            status: status_tx.clone(),
                            cleanup: cleanup.clone(),
                            options,
                        }),
                    ));
                    let cleanup_store = cleanup.lock().unwrap().take();
                    if let Some(store) = cleanup_store {
                        // Adapter startup runs after HTTP admission returns.
                        // Sweep while cancelling async tasks: a late hybrid PTY
                        // must die even if a blocking writer delays runtime drop.
                        std::thread::scope(|scope| {
                            let (finished, stopping) = std::sync::mpsc::channel::<()>();
                            let reaper = scope.spawn(move || {
                                loop {
                                    store.kill_all_ptys();
                                    if !matches!(
                                        stopping.recv_timeout(std::time::Duration::from_millis(25)),
                                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                                    ) {
                                        break;
                                    }
                                }
                                // Runtime cancellation has stopped normal async startup;
                                // sweep again for its final registrations.
                                store.reap_shutdown_ptys();
                            });
                            let began = std::time::Instant::now();
                            let limit = std::time::Duration::from_secs(5);
                            runtime.shutdown_timeout(limit);
                            drop(finished);
                            reaper.join().expect("embedded PTY reaper panicked");
                            cleanup_timed_out = began.elapsed() >= limit;
                            anyhow::ensure!(!cleanup_timed_out, "embedded runtime shutdown reached its five-second limit; blocking work may still be finishing; restart the application before starting another backend");
                            Ok::<(), anyhow::Error>(())
                        })?;
                    } else {
                        runtime.shutdown_timeout(std::time::Duration::from_secs(5));
                    }
                    result
                })();
                if cleanup_timed_out {
                    // Never let a replacement runtime reuse provider callbacks
                    // while timed-out blocking work might still reference them.
                    std::mem::forget(lease);
                }
                status_tx.send_replace(match &result {
                    Ok(()) => Status::Stopped,
                    Err(error) => Status::Failed(format!("{error:#}")),
                });
                result
            })
            .context("starting claudemon backend thread")?;
        Ok(Self {
            client: EmbeddedClient { commands, status },
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }
    pub fn client(&self) -> EmbeddedClient {
        self.client.clone()
    }
    pub async fn ready(&mut self) -> Result<ReadyInfo> {
        loop {
            let status = self.client.status.borrow().clone();
            match status {
                Status::Ready(info) => return Ok(info),
                Status::Failed(error) => return Err(anyhow!(error)),
                Status::Stopped => return Err(anyhow!("embedded daemon stopped before readiness")),
                Status::Starting => {}
            }
            self.client
                .status
                .changed()
                .await
                .context("backend readiness channel closed")?;
        }
    }
    pub fn request_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
    /// Joins off the caller's executor thread. UI destruction may instead drop
    /// this owner, which requests shutdown without blocking the UI.
    pub async fn shutdown(mut self) -> Result<()> {
        self.request_shutdown();
        let thread = self.thread.take().expect("backend thread present");
        tokio::task::spawn_blocking(move || {
            thread
                .join()
                .map_err(|_| anyhow!("claudemon backend thread panicked"))?
        })
        .await
        .context("joining claudemon backend")?
    }
}
impl Drop for EmbeddedDaemon {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
pub(super) struct Control {
    receiver: mpsc::Receiver<Envelope>,
    status: watch::Sender<Status>,
    pub(super) cleanup: std::sync::Arc<std::sync::Mutex<Option<crate::session::SessionStore>>>,
    pub(super) options: Options,
}
pub(super) fn serve_commands(
    mut control: Control,
    router: Router,
    hook_addr: SocketAddr,
    api_addr: SocketAddr,
) -> tokio::task::JoinHandle<()> {
    control.status.send_replace(Status::Ready(ReadyInfo {
        hook_addr,
        api_addr,
    }));
    tokio::spawn(async move {
        while let Some(envelope) = control.receiver.recv().await {
            if envelope.reply.is_closed() {
                continue;
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                dispatch(router.clone(), envelope.command),
            )
            .await
            .unwrap_or_else(|_| {
                Err(anyhow!(
                    "embedded command timed out; outcome is unknown, do not automatically retry"
                ))
            });
            let _ = envelope.reply.send(result);
        }
    })
}
async fn dispatch(router: Router, command: Command) -> Result<Value> {
    let session_path = |id: &str, suffix: &str| -> Result<String> {
        anyhow::ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && id != "."
                && id != "..",
            "invalid session id"
        );
        Ok(format!("/sessions/{id}/{suffix}"))
    };
    let (path, payload) = match command {
        Command::Sessions => ("/sessions".to_owned(), None),
        Command::Conversation { id, since } => {
            let mut path = session_path(&id, "conversation")?;
            if let Some(seq) = since {
                path.push_str(&format!("?since={seq}"));
            }
            (path, None)
        }
        Command::Message { id, text } => {
            (session_path(&id, "message")?, Some(json!({"text": text})))
        }
        Command::Approve { id, decision } => (
            session_path(&id, "approve")?,
            Some(json!({"decision": decision})),
        ),
        Command::Answer { id, answer } => (session_path(&id, "answer")?, Some(answer)),
        Command::Interrupt { id } => (
            session_path(&id, "signal")?,
            Some(json!({"signal": "SIGINT"})),
        ),
    };
    let request = Request::builder()
        .method(if payload.is_some() { "POST" } else { "GET" })
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(
            payload.map(|p| p.to_string()).unwrap_or_default(),
        ))?;
    let response = router.oneshot(request).await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 32 * 1024 * 1024).await?;
    anyhow::ensure!(
        status.is_success(),
        "daemon returned {status}: {}",
        String::from_utf8_lossy(&body)
    );
    let value: Value = serde_json::from_slice(&body)?;
    anyhow::ensure!(
        value.get("ok").and_then(Value::as_bool) != Some(false),
        "daemon rejected command: {value}"
    );
    Ok(value)
}
