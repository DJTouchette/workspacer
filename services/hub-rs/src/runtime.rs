use crate::auth::{Identity, Kind, Store};
use crate::protocol::{Event, Frame, matches, now};
use anyhow::{Result, anyhow, bail};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};

const COMMAND_QUEUE: usize = 256;
const MAX_CONNECTIONS: usize = 1024;
const MAX_TOPICS: usize = 512;
const MAX_FRAME_TOPICS: usize = 256;
const MAX_PENDING: usize = 4096;
const MAX_DEMAND_TOPICS: usize = 4096;
static SHUTDOWN_UNCERTAIN: AtomicBool = AtomicBool::new(false);

/// Supplied by the runtime, never deserialized from a caller's parameters.
#[derive(Clone, Debug)]
pub struct Caller {
    pub call_id: u64,
    pub activity_seq: u64,
    pub federated: bool,
    pub connection_id: u64,
    pub authenticated_host: bool,
    pub trusted: bool,
    pub scope: String,
    pub plugin_id: String,
    pub token_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchPermit {
    pub(crate) call_id: u64,
    pub(crate) connection_id: u64,
    pub(crate) session_id: String,
    pub(crate) plugin_id: String,
    pub(crate) nonce: String,
}

type Handler = Arc<dyn Fn(Caller, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

pub struct Options {
    #[cfg(feature = "test-support")]
    pub(crate) test_after_services: Option<crate::test_support::AfterServices>,
    /// None starts an entirely in-memory hub: no listener or token required.
    pub listen: Option<SocketAddr>,
    pub mcp_listen: Option<SocketAddr>,
    pub token: String,
    pub(crate) live_streams: Option<Arc<crate::services::live_streams::LiveStreams>>,
    pub external_claudemon_url: Option<String>,
    pub mcp_untokened: Option<crate::mcp::UntokenedAccess>,
    pub mcp_static_token: Option<String>,
    mcp_access: Option<Arc<crate::mcp::access::Policy>>,
    pub(crate) external_claudemon: Option<Arc<crate::services::external_claudemon::ExternalDaemon>>,
    pub(crate) provider_utilities: Option<Arc<crate::services::provider_utilities::Service>>,
    pub(crate) workflow_artifacts: Option<Arc<crate::services::workflow_artifacts::Service>>,
    pub claude_hook_settings: Option<std::path::PathBuf>,
    pub network_admin_socket: Option<std::path::PathBuf>,
    pub network_admin_token_file: Option<std::path::PathBuf>,
    pub uploads_to_worker: bool,
    pub control_plane_only: bool,
    pub provider_relay: Option<crate::provider_relay::Config>,
    pub(crate) upstream_caller: Option<Arc<crate::provider_relay::UpstreamCaller>>,
    pub(crate) upstream_layout: Option<Arc<std::sync::RwLock<Value>>>,
    pub scoped_tokens: Option<std::path::PathBuf>,
    pub data_dir: Option<std::path::PathBuf>,
    pub config_dir: Option<std::path::PathBuf>,
    pub home_dir: Option<std::path::PathBuf>,
    pub jobs_file: Option<std::path::PathBuf>,
    pub(crate) jobs_service: Option<Arc<crate::services::jobs::Service>>,
    pub(crate) terminals: Option<Arc<crate::services::terminals::Terminals>>,
    pub machine_power_provider: Option<Arc<dyn crate::services::machine_power::PowerProvider>>,
    pub(crate) analytics_watcher: Option<Arc<crate::services::analytics::Watcher>>,
    pub federation_peers: Vec<crate::federation::Peer>,
    pub peers_file: Option<std::path::PathBuf>,
    pub plugins_dir: Option<std::path::PathBuf>,
    pub plugins_stream_logs: bool,
    pub sidecar_node: Option<String>,
    pub plugin_origin: String,
    pub trusted_hosts: Vec<String>,
    pub webapp_dir: Option<std::path::PathBuf>,
    pub push_dir: Option<std::path::PathBuf>,
    pub nodes_file: Option<std::path::PathBuf>,
    pub nodes_keep_failed_wakes_running: bool,
    pub plugin_examples_dir: Option<std::path::PathBuf>,
    pub(crate) launch_lifecycle: Option<Arc<crate::services::agent_lifecycle::Lifecycle>>,
    pub(crate) routing: Option<Arc<crate::services::routing::RoutingService>>,
    pub(crate) session_snapshots: Arc<std::sync::RwLock<BTreeMap<String, Value>>>,
    pub(crate) confirmed_controls: Arc<crate::services::live_controls::ConfirmedControls>,
    pub(crate) workflow_runtime: Option<Arc<crate::services::workflow_runtime::WorkflowRuntime>>,
    pub(crate) replacements: Option<Arc<crate::services::manager_replacements::ReplacementState>>,
    pub(crate) worktrees: Option<Arc<crate::services::worktrees::Worktrees>>,
    pub(crate) review_store: Option<Arc<crate::services::fleet_review::ReviewStore>>,
    pub(crate) paired_target: Option<crate::services::remote_dispatch::paired::Target>,
    pub(crate) paired_dispatch: Option<Arc<crate::services::remote_dispatch::paired::Paired>>,
    pub(crate) remote_proxy_snapshots: Arc<std::sync::RwLock<BTreeMap<String, Value>>>,
    pub(crate) remote_dispatch_delivery:
        Option<Arc<dyn crate::services::remote_dispatch::Delivery>>,
    pub(crate) remote_origin: Option<Arc<crate::services::remote_dispatch::Origin>>,
    pub(crate) remote_dispatch_execution:
        Option<Arc<dyn crate::services::remote_dispatch::Execution>>,
    pub(crate) remote_receiver: Option<Arc<crate::services::remote_dispatch::Receiver>>,
    pub(crate) spawn_coordinator: Option<Arc<crate::services::agent_spawn::SpawnCoordinator>>,
    pub(crate) launch_preparation: Option<Arc<crate::plugins::launch::Preparation>>,
    pub(crate) message_tracker: Option<Arc<crate::services::manager_replacements::MessageTracker>>,
    pub(crate) wakes: Option<Arc<crate::services::wakes::Wakes>>,
    pub(crate) mcp_ready: Arc<AtomicBool>,
    pub(crate) replacement_service:
        Option<Arc<crate::services::manager_replacements::ReplacementService>>,
    pub engine: Option<claudemon::daemon::embedded::EmbeddedClient>,
    pub(crate) layout: Option<Arc<crate::services::layout::Layout>>,
    pub call_timeout: Duration,
    pub event_buffer: usize,
    handlers: BTreeMap<String, Handler>,
    plugins: BTreeMap<String, (String, Vec<String>)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            #[cfg(feature = "test-support")]
            test_after_services: None,
            listen: None,
            mcp_listen: None,
            token: String::new(),
            live_streams: None,
            external_claudemon_url: None,
            mcp_untokened: None,
            mcp_static_token: None,
            mcp_access: None,
            external_claudemon: None,
            provider_utilities: None,
            workflow_artifacts: None,
            claude_hook_settings: None,
            network_admin_socket: None,
            network_admin_token_file: None,
            uploads_to_worker: false,
            control_plane_only: false,
            provider_relay: None,
            upstream_caller: None,
            upstream_layout: None,
            scoped_tokens: None,
            data_dir: None,
            config_dir: None,
            home_dir: None,
            jobs_file: None,
            jobs_service: None,
            analytics_watcher: None,
            terminals: None,
            machine_power_provider: None,
            federation_peers: Vec::new(),
            peers_file: None,
            plugins_dir: None,
            plugins_stream_logs: false,
            sidecar_node: None,
            plugin_origin: String::new(),
            trusted_hosts: Vec::new(),
            webapp_dir: None,
            push_dir: None,
            nodes_file: None,
            nodes_keep_failed_wakes_running: false,
            plugin_examples_dir: None,
            launch_lifecycle: None,
            routing: None,
            session_snapshots: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            confirmed_controls: Default::default(),
            workflow_runtime: None,
            replacements: None,
            worktrees: None,
            review_store: None,
            paired_target: None,
            paired_dispatch: None,
            remote_proxy_snapshots: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            remote_dispatch_delivery: None,
            remote_origin: None,
            remote_dispatch_execution: None,
            remote_receiver: None,
            spawn_coordinator: None,
            launch_preparation: None,
            message_tracker: None,
            wakes: None,
            mcp_ready: Arc::new(AtomicBool::new(false)),
            replacement_service: None,
            engine: None,
            layout: None,
            call_timeout: Duration::from_secs(30),
            event_buffer: 64,
            handlers: BTreeMap::new(),
            plugins: BTreeMap::new(),
        }
    }
}

impl Options {
    pub(crate) fn has_handler(&self, method: &str) -> bool {
        self.handlers.contains_key(method)
    }
    pub fn plugin_token(
        mut self,
        token: impl Into<String>,
        id: impl Into<String>,
        provides: Vec<String>,
    ) -> Self {
        let id = id.into();
        let prefix = format!("{id}.");
        let provides = provides
            .into_iter()
            .filter(|p| {
                !id.is_empty()
                    && p.starts_with(&prefix)
                    && p != &prefix
                    && (!p[prefix.len()..].contains('*') || p == &(prefix.clone() + "*"))
            })
            .collect();
        self.plugins.insert(token.into(), (id, provides));
        self
    }
    /// Install a host-owned service before admission opens. Local methods cannot
    /// be claimed by a socket provider, and use the same dispatch for both hosts.
    pub fn handler<F, Fut>(mut self, method: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Caller, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        self.handlers
            .insert(method.into(), Arc::new(move |c, p| Box::pin(handler(c, p))));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready {
        address: Option<SocketAddr>,
        mcp_address: Option<SocketAddr>,
    },
    Stopped,
    Failed(String),
}

/// Host owns this object. A window or a client disappearing never shuts down
/// the hub. No process-wide signal handlers are installed by the library.
pub struct Hub {
    handle: Handle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Command>,
    status: watch::Receiver<Status>,
}

/// Reserved before a persisted peer update begins. Publication is synchronous
/// inside the runtime-owned blocking transaction, even if its caller expires.
pub(crate) struct PeerReplacementReservation {
    permit: mpsc::OwnedPermit<Command>,
    reply: oneshot::Sender<Result<()>>,
}
impl PeerReplacementReservation {
    pub(crate) fn publish(self, peers: Vec<crate::federation::Peer>) {
        self.permit.send(Command::ReplacePeers(peers, self.reply));
    }
}

impl Hub {
    pub fn start(mut options: Options) -> Result<Self> {
        if options
            .provider_relay
            .as_ref()
            .is_some_and(|relay| relay.scope == crate::provider_relay::Scope::Full)
            && options.upstream_layout.is_none()
        {
            options.upstream_layout = Some(Arc::new(std::sync::RwLock::new(Value::Null)));
        }
        anyhow::ensure!(
            !SHUTDOWN_UNCERTAIN.load(Ordering::Acquire),
            "a previous hub shutdown left blocking work unconfirmed; restart the application before starting another hub"
        );
        anyhow::ensure!(
            !options.control_plane_only || options.engine.is_none(),
            "control-plane-only cannot own an execution engine"
        );
        if let Some(url) = &options.external_claudemon_url {
            anyhow::ensure!(
                options.control_plane_only && options.engine.is_none(),
                "external claudemon observation requires the control-plane-only role"
            );
            options.external_claudemon = Some(
                crate::services::external_claudemon::ExternalDaemon::new(url)?,
            );
        }
        if let Some(address) = options.mcp_listen {
            options.mcp_access = Some(crate::mcp::access::Policy::new(
                options
                    .config_dir
                    .as_ref()
                    .map(|path| path.join("config.yaml")),
                options.mcp_untokened,
                options.mcp_static_token.clone().unwrap_or_default(),
                address.ip(),
            )?);
        }
        if options.event_buffer == 0 || options.event_buffer > 65536 {
            bail!("invalid event buffer");
        }
        if options.call_timeout.is_zero() {
            bail!("call timeout must be positive");
        }
        if (options.listen.is_some() || options.mcp_listen.is_some()) && options.token.is_empty() {
            bail!("a listener requires an explicit host token");
        }
        crate::server::policy::Policy::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            &options.trusted_hosts,
        )?;
        // Listener selection is host-owned. Every network adapter requires the
        // explicit credential above and pins browser policy to its actual socket.
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE);
        let (state, status) = watch::channel(Status::Starting);
        let handle = Handle { tx, status };
        let worker_handle = handle.clone();
        let (stop, stopping) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("hub-backend".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(2)
                            .thread_name("hub-worker")
                            .enable_all()
                            .build()?;
                        let result = rt.block_on(run(options, worker_handle, rx, stopping, &state));
                        let began=Instant::now();
                        let limit=Duration::from_secs(2);
                        rt.shutdown_timeout(limit);
                        if began.elapsed()>=limit {
                            SHUTDOWN_UNCERTAIN.store(true, Ordering::Release);
                            bail!("hub shutdown reached its two-second runtime cleanup limit; blocking work may still be finishing; restart the application before starting another hub");
                        }
                        result
                    }))
                    .unwrap_or_else(|_| Err(anyhow!("hub runtime panicked")));
                state.send_replace(match &result {
                    Ok(()) => Status::Stopped,
                    Err(e) => Status::Failed(e.to_string()),
                });
                result
            })?;
        Ok(Self {
            handle,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub async fn ready(&self) -> Result<Option<SocketAddr>> {
        self.handle.ready().await
    }
    pub fn shutdown(mut self) -> Result<()> {
        self.stop_and_join()
    }
    fn stop_and_join(&mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow!("hub thread panicked"))??;
        }
        Ok(())
    }
}
impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

impl Handle {
    pub(crate) async fn publish_live_stream(
        &self,
        delivery: crate::services::live_streams::Delivery,
    ) -> Result<()> {
        self.tx
            .send(Command::LiveStream(delivery))
            .await
            .map_err(|_| anyhow!("hub stopped"))
    }
    pub(crate) async fn connect_provider_caller(
        &self,
        proof: crate::protocol::ProviderCaller,
    ) -> Result<Connection> {
        let identity = Identity::from_provider_caller(proof)?;
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(
            None,
            ConnectMode::Delegated(identity),
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting provider caller"))?
    }

    pub(crate) async fn provider_connection(&self, method: &str) -> Result<Option<u64>> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::ProviderConnection(method.into(), reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped during provider observation"))
    }
    pub(crate) async fn evict_provider(
        &self,
        method: &str,
        expected_connection: u64,
    ) -> Result<bool> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::EvictProvider(
            method.into(),
            expected_connection,
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped during provider eviction"))
    }

    pub(crate) async fn disconnect_for_machine_stop(&self) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::MachineStop(reply))?;
        let clients = result
            .await
            .map_err(|_| anyhow!("hub stopped before client drain"))?;
        tokio::time::timeout(Duration::from_secs(5), async move {
            futures_util::future::join_all(clients.into_iter().map(|mut done| async move {
                while !*done.borrow() {
                    if done.changed().await.is_err() {
                        break;
                    }
                }
            }))
            .await;
        })
        .await
        .map_err(|_| anyhow!("interactive client close acknowledgement timed out"))?;
        Ok(())
    }

    pub(crate) async fn connect_service(&self) -> Result<Connection> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(None, ConnectMode::Service, reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting service"))?
    }
    pub(crate) async fn quiescence_clients(
        &self,
    ) -> Result<Vec<crate::services::quiescence::ClientInfo>> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::QuiescenceClients(reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while reading client activity"))
    }

    /// Facade delegation preserves the inbound operator tier without granting
    /// authenticated-host administration. This never accepts wire metadata.
    pub(crate) async fn connect_facade(&self, token: String) -> Result<Connection> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(
            Some((token, false)),
            ConnectMode::Facade,
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting facade"))?
    }
    pub(crate) async fn connect_ephemeral_facade(
        &self,
        lease: crate::mcp::access::Lease,
    ) -> Result<Connection> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(
            None,
            ConnectMode::EphemeralFacade(lease),
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting facade"))?
    }
    pub(crate) async fn begin_launch_preparation(
        &self,
        caller: &Caller,
        session_id: String,
        plugin_id: String,
    ) -> Result<LaunchPermit> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::BeginLaunch(
            caller.clone(),
            session_id,
            plugin_id,
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped during launch preparation"))?
    }
    pub(crate) async fn check_launch_preparation(
        &self,
        permit: &LaunchPermit,
        finish: bool,
    ) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::CheckLaunch(permit.clone(), finish, reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped during launch preparation"))?
    }
    pub(crate) async fn reserve_peer_replacement(
        &self,
    ) -> Result<(PeerReplacementReservation, oneshot::Receiver<Result<()>>)> {
        let permit = self
            .tx
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| anyhow!("hub stopped before peer configuration commit"))?;
        let (reply, result) = oneshot::channel();
        Ok((PeerReplacementReservation { permit, reply }, result))
    }
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
    pub async fn ready(&self) -> Result<Option<SocketAddr>> {
        let mut status = self.status.clone();
        loop {
            match status.borrow_and_update().clone() {
                Status::Ready { address, .. } => return Ok(address),
                Status::Starting => (),
                Status::Stopped => bail!("hub stopped before readiness"),
                Status::Failed(e) => bail!("hub startup failed: {e}"),
            }
            status
                .changed()
                .await
                .map_err(|_| anyhow!("hub readiness channel closed"))?;
        }
    }
    /// A trusted in-process connection. Wire credentials are verified by the
    /// transport before it obtains this handle's connection.
    pub async fn connect(&self) -> Result<Connection> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(None, ConnectMode::User, reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting"))?
    }
    pub async fn connect_authenticated(
        &self,
        token: String,
        federated: bool,
    ) -> Result<Connection> {
        self.connect_authenticated_context(token, federated, false)
            .await
    }
    pub(crate) async fn connect_authenticated_context(
        &self,
        token: String,
        federated: bool,
        identity: bool,
    ) -> Result<Connection> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Connect(
            Some((token, federated)),
            if identity {
                ConnectMode::UserIdentity
            } else {
                ConnectMode::User
            },
            reply,
        ))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while connecting"))?
    }
    #[cfg(feature = "test-support")]
    pub(crate) async fn test_receiver(
        &self,
    ) -> Result<Option<Arc<crate::services::remote_dispatch::Receiver>>> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::TestReceiver(reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped before fixture inspection"))
    }
    pub async fn health(&self) -> Result<Value> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Health(reply))?;
        result.await.map_err(|_| anyhow!("hub stopped"))
    }
    /// Plugin lifecycle is host-owned; no bus frame can mint or revoke identity.
    pub async fn register_plugin(
        &self,
        token: String,
        id: String,
        provides: Vec<String>,
    ) -> Result<()> {
        anyhow::ensure!(
            !token.is_empty() && !id.is_empty(),
            "plugin identity must not be empty"
        );
        let (reply, result) = oneshot::channel();
        self.submit(Command::Plugin {
            token,
            identity: Some((id, provides)),
            reply,
        })?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while registering plugin"))?
    }
    pub async fn revoke_plugin(&self, token: String) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::Plugin {
            token,
            identity: None,
            reply,
        })?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while revoking plugin"))?
    }
    /// HTTP settings entitlement uses the same live registry as bus admission.
    pub(crate) async fn plugin_token_matches(&self, token: String, id: String) -> Result<bool> {
        let (reply, result) = oneshot::channel();
        self.submit(Command::PluginTokenMatches(token, id, reply))?;
        result
            .await
            .map_err(|_| anyhow!("hub stopped while checking plugin identity"))
    }
    pub fn publish(&self, event: Event) -> Result<()> {
        self.submit(Command::Publish(event))
    }
    /// Backpressure for owned asynchronous producers. Dropping a GUI receiver
    /// does not turn a temporary full command queue into a backend failure.
    pub async fn publish_wait(&self, event: Event) -> Result<()> {
        self.tx
            .send(Command::Publish(event))
            .await
            .map_err(|_| anyhow!("hub stopped before publishing event"))
    }
    fn submit(&self, command: Command) -> Result<()> {
        self.tx.try_send(command).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                anyhow!("hub command queue full; command was not submitted")
            }
            mpsc::error::TrySendError::Closed(_) => {
                anyhow!("hub stopped; command was not submitted")
            }
        })
    }
}

pub struct Connection {
    close_code: Arc<AtomicU16>,
    network: Arc<AtomicBool>,
    close_done: watch::Sender<bool>,
    id: u64,
    handle: Handle,
    reliable: mpsc::Receiver<Frame>,
    events: mpsc::Receiver<Frame>,
    desynced: Arc<Mutex<BTreeSet<String>>>,
    desync_frames: VecDeque<Frame>,
    closed: watch::Receiver<bool>,
    release: mpsc::UnboundedSender<u64>,
}

impl Connection {
    pub(crate) fn mark_network_transport(&self) {
        self.network.store(true, Ordering::Release);
    }
    pub fn close_code(&self) -> Option<u16> {
        let code = self.close_code.load(Ordering::Acquire);
        (code != 0).then_some(code)
    }

    pub fn send(&self, frame: Frame) -> Result<()> {
        if *self.closed.borrow() {
            bail!("connection closed; command was not submitted");
        }
        self.handle.submit(Command::Frame(self.id, Box::new(frame)))
    }
    pub async fn recv(&mut self) -> Option<Frame> {
        if *self.closed.borrow() {
            return None;
        }
        if let Some(frame) = self.desync_frames.pop_front() {
            return Some(frame);
        }
        tokio::select! {
            biased;
            _ = self.closed.changed() => None,
            frame = self.reliable.recv() => frame,
            frame = self.events.recv() => {
                let desynced = std::mem::take(&mut *self.desynced.lock().unwrap());
                if frame.is_none() {None} else if desynced.is_empty() {frame} else if !desynced.iter().any(|t|t.starts_with("agent.conversation.")) {
                    // Preserve the established PTY byte-then-desync contract.
                    for topic in desynced {
                        if let Some(id)=topic.strip_prefix("pty.bytes.") {
                            let mut ev=Event::new("pty.desync","hub",json!({"sessionId":id}));ev.time=now();
                            self.desync_frames.push_back(Frame{event:Some(ev),..Frame::op("event")});
                        }
                    }
                    frame
                } else {
                    // Drain exactly the backlog present at this boundary. New
                    // arrivals remain behind ready; none of the older fragments
                    // for a desynced topic may follow the repair instruction.
                    let mut retained=VecDeque::new();
                    let mut pending=Vec::new();if let Some(frame)=frame{pending.push(frame);}
                    let backlog=self.events.len();
                    for _ in 0..backlog {if let Ok(frame)=self.events.try_recv(){pending.push(frame);}else{break;}}
                    for frame in pending {
                        if !frame.event.as_ref().is_some_and(|event|event.topic.starts_with("agent.conversation.")&&desynced.contains(&event.topic)){retained.push_back(frame);}
                    }
                    self.desync_frames.extend(retained);
                    for topic in desynced {
                        let mut ev=if let Some(id)=topic.strip_prefix("agent.conversation."){
                            Event::new(topic.clone(),"hub",json!({"session_id":id,"ready":true}))
                        }else if let Some(id)=topic.strip_prefix("pty.bytes."){
                            Event::new("pty.desync","hub",json!({"sessionId":id}))
                        }else{continue;};
                        ev.time=now();self.desync_frames.push_back(Frame{event:Some(ev),..Frame::op("event")});
                    }
                    self.desync_frames.pop_front()
                }
            }
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.close_done.send_replace(true);
        let _ = self.release.send(self.id);
    }
}

#[cfg(test)]
mod machine_stop_tests {
    use super::*;
    use futures_util::StreamExt;
    #[tokio::test]
    async fn stop_drains_interactive_sockets_with_4001_but_keeps_infrastructure() {
        let mut options = Options::default();
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "fixture".into();
        let hub = Hub::start(options).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        let handle = hub.handle();
        let mut internal = handle.connect_service().await.unwrap();
        internal.recv().await.unwrap();
        let mut provider = handle.connect().await.unwrap();
        provider.recv().await.unwrap();
        provider
            .send(Frame {
                methods: vec!["fixture.provider".into()],
                ..Frame::op("register")
            })
            .unwrap();
        assert_eq!(provider.recv().await.unwrap().op, "registered");
        handle
            .register_plugin(
                "plugin-secret".into(),
                "fixture".into(),
                vec!["fixture.*".into()],
            )
            .await
            .unwrap();
        let mut plugin = handle
            .connect_authenticated("plugin-secret".into(), false)
            .await
            .unwrap();
        plugin.recv().await.unwrap();
        let mut ordinary = handle.connect().await.unwrap();
        ordinary.recv().await.unwrap();
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{address}/bus?token=fixture"))
                .await
                .unwrap();
        socket.next().await.unwrap().unwrap();
        handle.disconnect_for_machine_stop().await.unwrap();
        let close = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let tokio_tungstenite::tungstenite::Message::Close(Some(close)) = close else {
            panic!("expected close frame")
        };
        assert_eq!(u16::from(close.code), 4001);
        assert_eq!(close.reason, "machine stopping; reconnect only to wake");
        assert!(ordinary.recv().await.is_none());
        assert_eq!(ordinary.close_code(), Some(4001));
        for (index, connection) in [&mut internal, &mut provider, &mut plugin]
            .into_iter()
            .enumerate()
        {
            let method = format!("fixture.alive{index}");
            connection
                .send(Frame {
                    methods: vec![method.clone()],
                    ..Frame::op("register")
                })
                .unwrap();
            let response = connection.recv().await.unwrap();
            assert_eq!(response.op, "registered");
            assert_eq!(response.methods, vec![method]);
            assert_eq!(connection.close_code(), None);
        }
        hub.shutdown().unwrap();
    }
}

#[cfg(test)]
mod provider_eviction_tests {
    use super::*;
    #[tokio::test]
    async fn stale_probe_cannot_evict_replacement_or_builtin_handler() {
        let hub = Hub::start(
            Options::default().handler("fixture.builtin", |_, _| async { Ok(Value::Null) }),
        )
        .unwrap();
        hub.ready().await.unwrap();
        let handle = hub.handle();
        assert_eq!(
            handle.provider_connection("fixture.builtin").await.unwrap(),
            None
        );
        let mut old = handle.connect().await.unwrap();
        old.recv().await.unwrap();
        old.send(Frame {
            methods: vec!["brain.info".into()],
            ..Frame::op("register")
        })
        .unwrap();
        old.recv().await.unwrap();
        let generation = handle
            .provider_connection("brain.info")
            .await
            .unwrap()
            .unwrap();
        assert!(
            handle
                .evict_provider("brain.info", generation)
                .await
                .unwrap()
        );
        assert!(old.recv().await.is_none());
        let mut new = handle.connect().await.unwrap();
        new.recv().await.unwrap();
        new.send(Frame {
            methods: vec!["brain.info".into()],
            ..Frame::op("register")
        })
        .unwrap();
        assert_eq!(new.recv().await.unwrap().methods, vec!["brain.info"]);
        let replacement = handle
            .provider_connection("brain.info")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(generation, replacement);
        assert!(
            !handle
                .evict_provider("brain.info", generation)
                .await
                .unwrap()
        );
        assert_eq!(
            handle.provider_connection("brain.info").await.unwrap(),
            Some(replacement)
        );
        assert!(
            !handle
                .evict_provider("fixture.builtin", replacement)
                .await
                .unwrap()
        );
        assert_eq!(new.close_code(), None);
        hub.shutdown().unwrap();
    }
}

#[cfg(test)]
mod provider_context_tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_is_caller_owned_and_broker_deadline_notifies_only_negotiated_provider() {
        for negotiated in [false, true] {
            let mut options = Options::default();
            options.call_timeout = Duration::from_millis(300);
            let hub = Hub::start(options).unwrap();
            hub.ready().await.unwrap();
            let handle = hub.handle();
            let mut provider = handle.connect().await.unwrap();
            provider.recv().await.unwrap();
            provider
                .send(Frame {
                    methods: vec!["fixture.wait".into()],
                    wants_caller_context: negotiated,
                    ..Frame::op("register")
                })
                .unwrap();
            provider.recv().await.unwrap();
            let mut caller = handle.connect().await.unwrap();
            caller.recv().await.unwrap();
            let mut stranger = handle.connect().await.unwrap();
            stranger.recv().await.unwrap();
            caller
                .send(Frame {
                    id: "owned".into(),
                    method: "fixture.wait".into(),
                    ..Frame::op("call")
                })
                .unwrap();
            let first = provider.recv().await.unwrap();
            stranger
                .send(Frame {
                    id: "owned".into(),
                    provider_caller: Some(Identity::host("forgery").provider_caller(caller.id)),
                    ..Frame::op("cancel")
                })
                .unwrap();
            // FIFO command admission guarantees the forgery is processed first.
            provider
                .send(Frame {
                    id: first.id,
                    result: Some(json!({"retained":true})),
                    ..Frame::op("result")
                })
                .unwrap();
            assert_eq!(
                caller.recv().await.unwrap().result,
                Some(json!({"retained":true}))
            );
            caller
                .send(Frame {
                    id: "expires".into(),
                    method: "fixture.wait".into(),
                    ..Frame::op("call")
                })
                .unwrap();
            let second = provider.recv().await.unwrap();
            let expired = tokio::time::timeout(Duration::from_secs(2), caller.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(expired.error.contains("timed out"));
            if negotiated {
                let canceled = provider.recv().await.unwrap();
                assert_eq!(canceled.op, "cancel");
                assert_eq!(canceled.id, second.id);
                assert_eq!(canceled.caller_context_version, 1);
            } else {
                assert!(
                    tokio::time::timeout(Duration::from_millis(40), provider.recv())
                        .await
                        .is_err()
                );
            }
            hub.shutdown().unwrap();
        }
    }
    #[tokio::test]
    async fn negotiated_provider_observes_idle_caller_release() {
        let hub = Hub::start(Options::default()).unwrap();
        hub.ready().await.unwrap();
        let handle = hub.handle();
        let mut provider = handle.connect().await.unwrap();
        provider.recv().await.unwrap();
        provider
            .send(Frame {
                methods: vec!["fixture.echo".into()],
                wants_caller_context: true,
                ..Frame::op("register")
            })
            .unwrap();
        provider.recv().await.unwrap();
        let mut caller = handle.connect().await.unwrap();
        caller.recv().await.unwrap();
        let id = caller.id;
        drop(caller);
        let closed = tokio::time::timeout(Duration::from_secs(2), provider.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(closed.op, "callerClosed");
        assert_eq!(closed.id, id.to_string());
        assert_eq!(closed.caller_context_version, 1);
        assert!(closed.provider_caller.is_none());
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn negotiated_context_replaces_forgery_and_preserves_scope_and_lease_identity() {
        let directory = tempfile::tempdir().unwrap();
        let tokens = directory.path().join("tokens.json");
        let view = crate::auth::mint(&tokens, crate::auth::Scope::View, "viewer").unwrap();
        let operator =
            crate::auth::mint(&tokens, crate::auth::Scope::Operator, "session:manager").unwrap();
        let mut options = Options::default();
        options.token = "central-owner".into();
        options.scoped_tokens = Some(tokens);
        options.control_plane_only = true;
        let central = Hub::start(options).unwrap();
        central.ready().await.unwrap();
        let handle = central.handle();
        let mut provider = handle.connect().await.unwrap();
        provider.recv().await.unwrap();
        provider
            .send(Frame {
                methods: vec!["config.get".into()],
                wants_caller_context: true,
                ..Frame::op("register")
            })
            .unwrap();
        assert_eq!(provider.recv().await.unwrap().caller_context_version, 1);
        let mut viewer = handle
            .connect_authenticated(view.token.clone(), false)
            .await
            .unwrap();
        viewer.recv().await.unwrap();
        let fake = Identity::host("forged-owner").provider_caller(999);
        viewer
            .send(Frame {
                id: "view-call".into(),
                method: "config.get".into(),
                provider_caller: Some(fake),
                ..Frame::op("call")
            })
            .unwrap();
        let forwarded = provider.recv().await.unwrap();
        let proof = forwarded.provider_caller.unwrap();
        assert_eq!(proof.connection_id, viewer.id);
        assert_eq!(proof.scope, "view");
        assert!(!proof.authenticated_host);
        assert!(!proof.may_assert_session);
        assert_eq!(proof.token_id, crate::auth::fingerprint(&view.token));
        let downstream=Hub::start(Options::default().handler("config.get",|caller,_|async move{Ok(json!({"scope":caller.scope,"owner":caller.authenticated_host,"fingerprint":caller.token_id}))}).handler("desktop.fixture",|_,_|async{Ok(Value::Null)})).unwrap();
        downstream.ready().await.unwrap();
        let local = crate::client::Client::from_connection(
            downstream
                .handle()
                .connect_provider_caller(proof.clone())
                .await
                .unwrap(),
        );
        assert_eq!(
            local.call("config.get", Value::Null).await.unwrap(),
            json!({"scope":"view","owner":false,"fingerprint":proof.token_id})
        );
        assert!(local.call("desktop.fixture", Value::Null).await.is_err());
        provider
            .send(Frame {
                id: forwarded.id,
                result: Some(json!({"ok":true})),
                ..Frame::op("result")
            })
            .unwrap();
        assert_eq!(viewer.recv().await.unwrap().id, "view-call");
        let mut facade = handle.connect_facade(operator.token.clone()).await.unwrap();
        facade.recv().await.unwrap();
        facade
            .send(Frame {
                id: "facade-call".into(),
                method: "config.get".into(),
                ..Frame::op("call")
            })
            .unwrap();
        let proof = provider.recv().await.unwrap().provider_caller.unwrap();
        assert_eq!(proof.scope, "operator");
        assert!(!proof.authenticated_host);
        assert!(proof.may_assert_session);
        let identity = Identity::from_provider_caller(proof.clone()).unwrap();
        assert!(identity.may_assert_session());
        assert!(!identity.authenticated_host());
        let sanitized = crate::admission::sanitize(
            &identity,
            "agents.dispatchReplay",
            json!({"dispatchId":"d","originKey":"forged"}),
        )
        .unwrap();
        assert_eq!(
            sanitized["originKey"],
            crate::auth::fingerprint(&operator.token)
        );
        local.close();
        downstream.shutdown().unwrap();
        central.shutdown().unwrap();
    }
    #[tokio::test]
    async fn legacy_provider_frames_stay_unchanged_until_explicit_negotiation() {
        let hub = Hub::start(Options::default()).unwrap();
        hub.ready().await.unwrap();
        let handle = hub.handle();
        let mut provider = handle.connect().await.unwrap();
        provider.recv().await.unwrap();
        provider
            .send(Frame {
                methods: vec!["fixture.echo".into()],
                ..Frame::op("register")
            })
            .unwrap();
        assert_eq!(provider.recv().await.unwrap().caller_context_version, 0);
        let mut caller = handle.connect().await.unwrap();
        caller.recv().await.unwrap();
        caller
            .send(Frame {
                id: "request".into(),
                method: "fixture.echo".into(),
                ..Frame::op("call")
            })
            .unwrap();
        let wire = serde_json::to_value(provider.recv().await.unwrap()).unwrap();
        assert!(wire.get("providerCaller").is_none());
        assert!(wire.get("callerContextVersion").is_none());
        assert!(wire.get("wantsCallerContext").is_none());
        hub.shutdown().unwrap();
    }
}

#[derive(Clone)]
enum ConnectMode {
    User,
    UserIdentity,
    Service,
    Facade,
    Delegated(Identity),
    EphemeralFacade(crate::mcp::access::Lease),
}
enum Command {
    #[cfg(feature = "test-support")]
    TestReceiver(oneshot::Sender<Option<Arc<crate::services::remote_dispatch::Receiver>>>),
    LiveStream(crate::services::live_streams::Delivery),
    ProviderConnection(String, oneshot::Sender<Option<u64>>),
    EvictProvider(String, u64, oneshot::Sender<bool>),
    MachineStop(oneshot::Sender<Vec<watch::Receiver<bool>>>),
    QuiescenceClients(oneshot::Sender<Vec<crate::services::quiescence::ClientInfo>>),
    BeginLaunch(
        Caller,
        String,
        String,
        oneshot::Sender<Result<LaunchPermit>>,
    ),
    CheckLaunch(LaunchPermit, bool, oneshot::Sender<Result<()>>),
    ReplacePeers(Vec<crate::federation::Peer>, oneshot::Sender<Result<()>>),
    PluginTokenMatches(String, String, oneshot::Sender<bool>),
    Plugin {
        token: String,
        identity: Option<(String, Vec<String>)>,
        reply: oneshot::Sender<Result<()>>,
    },
    Publish(Event),
    Connect(
        Option<(String, bool)>,
        ConnectMode,
        oneshot::Sender<Result<Connection>>,
    ),
    Health(oneshot::Sender<Value>),
    Frame(u64, Box<Frame>),
}
struct Peer {
    facade_access: Option<crate::mcp::access::Lease>,
    delegated: bool,
    wants_caller_context: bool,
    close_code: Arc<AtomicU16>,
    network: Arc<AtomicBool>,
    close_done: watch::Receiver<bool>,
    local_facade: bool,
    internal: bool,
    activity_seq: u64,
    last_active_ms: i64,
    last_interaction_ms: i64,
    reports_interaction: bool,
    identity: Identity,
    credential: Option<String>,
    reliable: mpsc::Sender<Frame>,
    events: mpsc::Sender<Frame>,
    closed: watch::Sender<bool>,
    topics: Vec<String>,
    demand: Vec<String>,
    held_demand: BTreeSet<String>,
    desynced: Arc<Mutex<BTreeSet<String>>>,
}
struct Pending {
    caller: u64,
    provider: Option<u64>,
    correlation: String,
    method: String,
    deadline: Instant,
    selected_integration: Option<String>,
    launch_permit: Option<LaunchPermit>,
    launch_prepared: bool,
}
struct Core {
    federation: crate::federation::Routes,
    peers: HashMap<u64, Peer>,
    providers: BTreeMap<String, u64>,
    missing_reported: BTreeSet<String>,
    pending: HashMap<u64, Pending>,
    demand_counts: BTreeMap<String, usize>,
    seq: u64,
    event_seq: u64,
    options: Options,
    jobs: tokio::task::JoinSet<(u64, Result<Value>)>,
    last_revalidation: Instant,
}

impl Core {
    fn launch_ready(&self) -> bool {
        self.options.spawn_coordinator.is_some()
            && self.options.wakes.is_some()
            && (self.options.mcp_listen.is_none() || self.options.mcp_ready.load(Ordering::Acquire))
            && self.options.engine.as_ref().is_some_and(|engine| {
                matches!(
                    *engine.status().borrow(),
                    claudemon::daemon::embedded::Status::Ready(_)
                )
            })
            && crate::mcp::supports_owned_launch()
    }
    fn begin_launch(
        &mut self,
        caller: &Caller,
        session_id: String,
        plugin_id: String,
    ) -> Result<LaunchPermit> {
        let peer = self
            .peers
            .get(&caller.connection_id)
            .ok_or_else(|| anyhow!("launch caller disconnected"))?;
        anyhow::ensure!(!*peer.closed.borrow(), "launch caller disconnected");
        anyhow::ensure!(
            peer.identity.authenticated_host(),
            "launch preparation requires authenticated host authority"
        );
        let pending = self
            .pending
            .get_mut(&caller.call_id)
            .ok_or_else(|| anyhow!("launch call is no longer pending"))?;
        anyhow::ensure!(
            pending.caller == caller.connection_id
                && pending.provider.is_none()
                && pending.method == "agents.spawn"
                && pending.deadline > Instant::now(),
            "launch preparation does not belong to this pending local spawn"
        );
        anyhow::ensure!(
            pending.selected_integration.as_deref() == Some(plugin_id.as_str()),
            "launch integration was not selected by this caller"
        );
        anyhow::ensure!(
            !session_id.is_empty() && session_id.len() <= 256,
            "invalid launch session identity"
        );
        anyhow::ensure!(
            pending.launch_permit.is_none() && !pending.launch_prepared,
            "launch preparation already used for this call"
        );
        let permit = LaunchPermit {
            call_id: caller.call_id,
            connection_id: caller.connection_id,
            session_id,
            plugin_id,
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        pending.launch_permit = Some(permit.clone());
        Ok(permit)
    }
    fn check_launch(&mut self, permit: &LaunchPermit, finish: bool) -> Result<()> {
        let peer = self
            .peers
            .get(&permit.connection_id)
            .ok_or_else(|| anyhow!("launch caller disconnected"))?;
        anyhow::ensure!(!*peer.closed.borrow(), "launch caller disconnected");
        anyhow::ensure!(
            peer.identity.authenticated_host(),
            "launch caller no longer has host authority"
        );
        let pending = self
            .pending
            .get_mut(&permit.call_id)
            .ok_or_else(|| anyhow!("launch call is no longer pending"))?;
        anyhow::ensure!(
            pending.caller == permit.connection_id
                && pending.method == "agents.spawn"
                && pending.deadline > Instant::now()
                && pending.launch_permit.as_ref() == Some(permit),
            "launch preparation proof is stale or mismatched"
        );
        if finish {
            pending.launch_permit = None;
            pending.launch_prepared = true;
        }
        Ok(())
    }
    fn publish(&mut self, mut event: Event) {
        if event.id.is_empty() {
            self.event_seq += 1;
            event.id = format!("ev-{}", self.event_seq);
        }
        if event.time.is_empty() || event.time == "0001-01-01T00:00:00Z" {
            event.time = now();
        }
        for peer in self.peers.values() {
            // Revocation/overflow marks a peer closed before the actor removes
            // it. Do not enqueue protected data or desync metadata in that gap.
            if *peer.closed.borrow()
                || !peer.identity.may_consume(&event.topic)
                || !peer.topics.iter().any(|t| matches(t, &event.topic))
            {
                continue;
            }
            if peer
                .events
                .try_send(Frame {
                    event: Some(event.clone()),
                    ..Frame::op("event")
                })
                .is_err()
                && (event.topic.starts_with("pty.bytes.")
                    || (event.hub.is_empty() && event.topic.starts_with("agent.conversation.")))
            {
                let mut topics = peer.desynced.lock().unwrap();
                if topics.len() < 64 {
                    topics.insert(event.topic.clone());
                }
            }
        }
    }
    fn send(&mut self, id: u64, frame: Frame) {
        if let Some(peer) = self.peers.get(&id)
            && peer.reliable.try_send(frame).is_err()
        {
            peer.closed.send_replace(true);
        }
    }
    fn count_demand(&self, topic: &str) -> usize {
        self.demand_counts.get(topic).copied().unwrap_or(0)
    }
    fn release_demand(&mut self, topic: &str) {
        if let Some(count) = self.demand_counts.get_mut(topic) {
            *count -= 1;
            if *count == 0 {
                self.demand_counts.remove(topic);
                self.demand_change(topic, false);
            }
        }
    }
    fn demand_change(&mut self, topic: &str, wanted: bool) {
        if let Some(streams) = &self.options.live_streams {
            streams.set_demand(topic, wanted);
        }
        let ids: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                p.identity.may_publish(topic)
                    && p.demand
                        .iter()
                        .any(|prefix| !prefix.is_empty() && topic.starts_with(prefix))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.send(
                id,
                Frame {
                    topic: topic.into(),
                    demand: wanted,
                    ..Frame::op("demand")
                },
            );
        }
    }
    fn disconnect(&mut self, id: u64) {
        let Some(peer) = self.peers.remove(&id) else {
            return;
        };
        peer.closed.send_replace(true);
        // Negotiated relays retain per-caller local connections for terminal
        // leases. Release that identity even when no RPC is currently pending.
        let relays: Vec<_> = self
            .peers
            .iter()
            .filter(|(owner, peer)| {
                peer.wants_caller_context
                    && self.providers.values().any(|provider| provider == *owner)
            })
            .map(|(owner, _)| *owner)
            .collect();
        for relay in relays {
            self.send(
                relay,
                Frame {
                    id: id.to_string(),
                    caller_context_version: 1,
                    ..Frame::op("callerClosed")
                },
            );
        }
        self.providers.retain(|_, owner| *owner != id);
        let lost: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.caller == id || p.provider == Some(id))
            .map(|(key, _)| *key)
            .collect();
        for key in lost {
            if let Some(p) = self.pending.get(&key) {
                if p.caller == id {
                    self.cancel_pending(key, None);
                } else {
                    self.finish(key, Err(anyhow!("provider for {} disconnected", p.method)));
                }
            }
        }
        for topic in peer.held_demand {
            self.release_demand(&topic);
        }
    }
    fn sweep(&mut self) {
        if self.last_revalidation.elapsed() >= Duration::from_secs(5) {
            self.last_revalidation = Instant::now();
            if let Some(path) = &self.options.scoped_tokens {
                let store = Store { path: path.clone() };
                for peer in self.peers.values() {
                    if let (Kind::Scoped(original), Some(token)) =
                        (&peer.identity.kind, &peer.credential)
                    {
                        let valid = store.lookup(token).is_some_and(|r| {
                            r.scope == original.scope
                                && r.facade_authority == original.facade_authority
                                && r.provides() == original.provides()
                        });
                        if !valid {
                            peer.closed.send_replace(true);
                        }
                    }
                }
            }
            for peer in self.peers.values() {
                if peer
                    .facade_access
                    .as_ref()
                    .is_some_and(|lease| !lease.valid())
                {
                    peer.closed.send_replace(true);
                }
            }
        }
        let closed: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| *p.closed.borrow() || p.reliable.is_closed())
            .map(|(id, _)| *id)
            .collect();
        for id in closed {
            self.disconnect(id);
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= Instant::now())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.cancel_pending(
                id,
                Some("call timed out; outcome is unknown, do not automatically retry"),
            );
        }
    }
    // Cancellation revokes pending admission/proofs, not an already accepted
    // engine effect. Local owned handlers finish their bookkeeping normally.
    fn cancel_pending(&mut self, id: u64, error: Option<&str>) {
        if let Some(p) = self.pending.remove(&id) {
            if let Some(provider) = p.provider {
                if self
                    .peers
                    .get(&provider)
                    .is_some_and(|peer| peer.wants_caller_context)
                {
                    self.send(
                        provider,
                        Frame {
                            id: id.to_string(),
                            caller_context_version: 1,
                            ..Frame::op("cancel")
                        },
                    );
                }
            }
            if let Some(error) = error {
                self.send(p.caller, Frame::error(p.correlation, error));
            }
        }
    }
    fn cancel_call(&mut self, caller: u64, correlation: &str) {
        // The numeric broker ID and any supplied providerCaller are irrelevant
        // on caller ingress: only this connection's own original ID can cancel.
        let matching: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.caller == caller && p.correlation == correlation)
            .map(|(id, _)| *id)
            .collect();
        for id in matching {
            self.cancel_pending(id, None);
        }
    }
    fn finish(&mut self, id: u64, result: Result<Value>) {
        if let Some(p) = self.pending.remove(&id) {
            let frame = match result {
                Ok(value) => Frame {
                    id: p.correlation,
                    result: Some(value),
                    ..Frame::op("result")
                },
                Err(error) => Frame::error(p.correlation, error.to_string()),
            };
            self.send(p.caller, frame);
        }
    }
    fn frame(&mut self, id: u64, frame: Frame) {
        if !self.peers.contains_key(&id) || *self.peers[&id].closed.borrow() {
            return;
        }
        if self.peers[&id]
            .facade_access
            .as_ref()
            .is_some_and(|lease| !lease.valid())
        {
            self.disconnect(id);
            return;
        }
        if matches!(frame.op.as_str(), "call" | "publish" | "activity") {
            let peer = self.peers.get_mut(&id).unwrap();
            let now = chrono::Utc::now().timestamp_millis();
            peer.activity_seq = peer.activity_seq.saturating_add(1);
            peer.last_active_ms = now;
            if frame.op == "activity" {
                peer.reports_interaction = true;
            }
            if frame.op != "call"
                || !crate::services::quiescence::passive_request(
                    &frame.method,
                    frame.params.as_ref(),
                )
            {
                peer.last_interaction_ms = now;
            }
        }
        match frame.op.as_str() {
            "subscribe" | "unsubscribe" | "demand" => self.topics(id, frame),
            "activity" => (),
            "register" => {
                let mut accepted = Vec::new();
                for method in frame.methods {
                    if method.is_empty()
                        || !self.peers[&id].identity.may_provide(&method)
                        || self.options.handlers.contains_key(&method)
                    {
                        continue;
                    }
                    if self
                        .providers
                        .get(&method)
                        .is_some_and(|owner| *owner != id)
                    {
                        continue;
                    }
                    self.providers.insert(method.clone(), id);
                    self.missing_reported.remove(&method);
                    accepted.push(method);
                }
                if frame.wants_caller_context {
                    self.peers.get_mut(&id).unwrap().wants_caller_context = true;
                }
                self.send(
                    id,
                    Frame {
                        caller_context_version: if self.peers[&id].wants_caller_context {
                            1
                        } else {
                            0
                        },
                        methods: accepted,
                        ..Frame::op("registered")
                    },
                );
            }
            "call" => self.call(id, frame),
            "cancel" => self.cancel_call(id, &frame.id),
            "result" | "error" => {
                if let Ok(key) = frame.id.parse::<u64>()
                    && self
                        .pending
                        .get(&key)
                        .is_some_and(|p| p.provider == Some(id))
                {
                    let result = if frame.op == "error" {
                        Err(anyhow!(frame.error))
                    } else {
                        Ok(frame.result.unwrap_or(Value::Null))
                    };
                    self.finish(key, result);
                }
            }
            "publish" => {
                if let Some(mut event) = frame.event {
                    // Only the trusted federation adapter can establish the
                    // owning peer; a wire publisher cannot stamp its own hub.
                    if event.topic.starts_with("agent.dispatch.") && !event.hub.is_empty() {
                        self.send(
                            id,
                            Frame::error("", "dispatch events cannot assert a hub identity"),
                        );
                        return;
                    }
                    event.hub.clear();
                    if !self.peers[&id].identity.may_publish(&event.topic) {
                        self.send(id, Frame::error("", format!("not authorized: publishing events is outside this token's {:?} scope", self.peers[&id].identity.scope())));
                        return;
                    }
                    // Receipt observations require the opted-in current execution
                    // provider, in addition to the credential's publish grant.
                    if event.topic.starts_with("agent.dispatch.")
                        && !(event.topic == "agent.dispatch.update"
                            && self.peers[&id].wants_caller_context
                            && self.providers.get("agents.spawn") == Some(&id))
                    {
                        self.send(
                            id,
                            Frame::error(
                                "",
                                "dispatch updates require the current negotiated spawn provider",
                            ),
                        );
                        return;
                    }
                    self.publish(event);
                } else {
                    self.send(id, Frame::error("", "publish missing event"));
                }
            }
            _ => self.send(id, Frame::error("", format!("unknown op: {}", frame.op))),
        }
    }
    fn topics(&mut self, id: u64, frame: Frame) {
        if frame.topics.len() > MAX_FRAME_TOPICS {
            self.send(
                id,
                Frame::error(
                    "",
                    format!(
                        "{} carries {} topics, over the {} limit for one frame",
                        frame.op,
                        frame.topics.len(),
                        MAX_FRAME_TOPICS
                    ),
                ),
            );
            return;
        }
        if frame.op == "demand" {
            self.peers.get_mut(&id).unwrap().demand = frame.topics;
            let demanded: Vec<_> = self.demand_counts.keys().cloned().collect();
            for topic in demanded {
                if self.count_demand(&topic) > 0
                    && self.peers[&id].identity.may_publish(&topic)
                    && self.peers[&id]
                        .demand
                        .iter()
                        .any(|p| !p.is_empty() && topic.starts_with(p))
                {
                    self.send(
                        id,
                        Frame {
                            topic,
                            demand: true,
                            ..Frame::op("demand")
                        },
                    );
                }
            }
            return;
        }
        let peer = self.peers.get_mut(&id).unwrap();
        if frame.op == "subscribe" {
            for t in &frame.topics {
                if peer.topics.len() == MAX_TOPICS {
                    break;
                }
                if !peer.topics.contains(t) {
                    peer.topics.push(t.clone());
                }
            }
        } else {
            peer.topics.retain(|t| !frame.topics.contains(t));
        }
        let topics = peer.topics.clone();
        for topic in frame.topics {
            if frame.op == "subscribe" {
                let peer = self.peers.get_mut(&id).unwrap();
                if topic.is_empty()
                    || topic.contains('*')
                    || !peer.topics.contains(&topic)
                    || !peer.identity.may_consume(&topic)
                    || peer.held_demand.contains(&topic)
                {
                    continue;
                }
                if !self.demand_counts.contains_key(&topic)
                    && self.demand_counts.len() >= MAX_DEMAND_TOPICS
                {
                    continue;
                }
                peer.held_demand.insert(topic.clone());
                let count = self.demand_counts.entry(topic.clone()).or_default();
                *count += 1;
                if *count == 1 {
                    self.demand_change(&topic, true);
                }
            } else if self.peers.get_mut(&id).unwrap().held_demand.remove(&topic) {
                self.release_demand(&topic);
            }
        }
        self.send(
            id,
            Frame {
                topics,
                ..Frame::op(if frame.op == "subscribe" {
                    "subscribed"
                } else {
                    "unsubscribed"
                })
            },
        );
    }
    fn call(&mut self, caller: u64, mut frame: Frame) {
        if frame.method.is_empty() {
            self.send(caller, Frame::error(frame.id, "call missing method"));
            return;
        }
        let qualified = if let Some(rest) = frame.method.strip_prefix("hub:") {
            match rest.split_once('/') {
                Some((peer, bare))
                    if !peer.is_empty() && !bare.is_empty() && !bare.starts_with("hub:") =>
                {
                    Some((peer.to_owned(), bare.to_owned()))
                }
                _ => {
                    self.send(caller, Frame::error(frame.id, "invalid qualified method"));
                    return;
                }
            }
        } else {
            None
        };
        let bare = qualified
            .as_ref()
            .map(|(_, bare)| bare.as_str())
            .unwrap_or(&frame.method);
        if qualified.is_some() && !self.peers[&caller].identity.plugin_id().is_empty() {
            self.send(
                caller,
                Frame::error(frame.id, "plugins may not call federated capabilities"),
            );
            return;
        }
        if !self.peers[&caller].identity.may_call(bare)
            && !(qualified.is_none()
                && self.peers[&caller]
                    .facade_access
                    .as_ref()
                    .is_some_and(|lease| lease.allows_plugin(bare)))
        {
            let error = if frame.method.starts_with("desktop.") {
                "desktop services require the authenticated server owner's connection".into()
            } else {
                format!(
                    "not authorized: method {:?} is outside this token's {:?} scope (mint a broader token with `workspacer token create`)",
                    frame.method,
                    self.peers[&caller].identity.scope()
                )
            };
            self.send(caller, Frame::error(frame.id, error));
            return;
        }
        let external_provider = qualified.is_none()
            && self.options.control_plane_only
            && self.providers.contains_key(bare)
            && !self.options.handlers.contains_key(bare);
        let upstream_scrubbed = if self.peers[&caller].delegated && bare == "agents.spawn" {
            frame
                .params
                .as_ref()
                .and_then(|params| params.get("escalationScrubbed"))
                .filter(|value| {
                    value.as_array().is_some_and(|fields| {
                        fields.len() <= 64
                            && fields
                                .iter()
                                .all(|field| field.as_str().is_some_and(|field| field.len() <= 256))
                    })
                })
                .cloned()
        } else {
            None
        };
        let had_params = frame.params.is_some();
        let mut admission_identity = self.peers[&caller].identity.clone();
        if self.peers[&caller].local_facade {
            if let Kind::Scoped(record) = &mut admission_identity.kind {
                record.facade_authority = true;
            }
        }
        match crate::admission::sanitize(
            &admission_identity,
            bare,
            frame.params.take().unwrap_or(Value::Null),
        ) {
            Ok(mut params) => {
                if let Some(scrubbed) = upstream_scrubbed {
                    if params.is_object() {
                        params["escalationScrubbed"] = scrubbed;
                    }
                }
                frame.params = (had_params || !params.is_null()).then_some(params);
            }
            Err(error) => {
                self.send(caller, Frame::error(frame.id, error.to_string()));
                return;
            }
        }
        if external_provider
            && bare == "agents.spawn"
            && let Some(routing) = self.options.routing.clone()
        {
            let params = frame.params.get_or_insert(Value::Null);
            let identity = &self.peers[&caller].identity;
            let audit_caller = Caller {
                call_id: 0,
                activity_seq: self.peers[&caller].activity_seq,
                connection_id: caller,
                authenticated_host: identity.authenticated_host(),
                trusted: identity.trusted(),
                scope: identity.scope().into(),
                plugin_id: identity.plugin_id().into(),
                token_id: identity.token_id.clone(),
                federated: identity.federated,
            };
            let mut audit = routing.begin_spawn_audit(Some(&audit_caller));
            match audit.check(params) {
                Ok(mut scrubbed) => {
                    if scrubbed
                        .iter()
                        .any(|key| matches!(key.as_str(), "model" | "provider"))
                    {
                        if let Some(params) = params.as_object_mut() {
                            for key in ["modelIdentity", "contextWindow"] {
                                if params.remove(key).is_some() {
                                    scrubbed.push(key.into());
                                }
                            }
                        }
                    }
                    audit.extend_scrubbed(&scrubbed);
                    if !scrubbed.is_empty() {
                        params["escalationScrubbed"] = json!(scrubbed);
                    }
                }
                Err(error) => {
                    self.send(caller, Frame::error(frame.id, error.to_string()));
                    return;
                }
            }
        }
        // These operations require the remaining admission and routing stages. Do
        // not expose an unsafe transparent substitute during the migration.
        let paired_spawn = qualified.is_none()
            && bare == "agents.spawn"
            && self.options.paired_dispatch.is_some()
            && frame
                .params
                .as_ref()
                .is_some_and(|params| params["executionTarget"] == "paired");
        let remote_forward =
            qualified.is_some() && bare == "agents.spawn" && self.options.remote_origin.is_some();
        let remote_spawn = qualified.is_none()
            && bare == "agents.spawn"
            && admission_identity.federated
            && self.options.remote_receiver.is_some()
            && frame
                .params
                .as_ref()
                .is_some_and(|params| params.get("remoteOrigin").is_some());
        if ((remote_spawn
            || paired_spawn
            || (qualified.is_none() && bare == "agents.dispatchPrepare"))
            && self.options.spawn_coordinator.is_some()
            && !self.launch_ready())
            || (matches!(bare, "agents.dispatchPrepare" | "agents.dispatchReplay")
                && if qualified.is_some() {
                    bare != "agents.dispatchReplay"
                } else {
                    self.options.remote_receiver.is_none() && !external_provider
                })
            || (bare == "fleetWorkflows.request"
                && (qualified.is_some()
                    || (self.options.workflow_runtime.is_none() && !external_provider)))
            || (bare == "agents.spawn"
                && !remote_spawn
                && !remote_forward
                && !paired_spawn
                && !external_provider
                && (qualified.is_some() || !self.launch_ready()))
        {
            self.send(
                caller,
                Frame::error(
                    frame.id,
                    if bare == "fleetWorkflows.request" {
                        "fleet workflows require the local desktop service"
                    } else {
                        "operation requires a configured execution service"
                    },
                ),
            );
            return;
        }
        let handler: Option<Handler> = if paired_spawn {
            let paired = self.options.paired_dispatch.clone().unwrap();
            Some(Arc::new(move |caller, params| {
                let paired = paired.clone();
                Box::pin(async move { paired.spawn(caller, params).await })
            }))
        } else if remote_spawn {
            let receiver = self.options.remote_receiver.clone().unwrap();
            Some(Arc::new(move |caller, params| {
                let receiver = receiver.clone();
                Box::pin(async move { receiver.spawn(caller, params).await })
            }))
        } else if remote_forward {
            let origin = self.options.remote_origin.clone().unwrap();
            let peer = qualified.as_ref().unwrap().0.clone();
            Some(Arc::new(move |caller, params| {
                let (origin, peer) = (origin.clone(), peer.clone());
                Box::pin(async move { origin.forward_sanitized(&caller, &peer, params).await })
            }))
        } else if let Some((peer, method)) = qualified {
            let routes = self.federation.clone();
            Some(Arc::new(move |_, params| {
                let (routes, peer, method) = (routes.clone(), peer.clone(), method.clone());
                Box::pin(async move { routes.forward(&peer, &method, params).await })
            }))
        } else {
            self.options.handlers.get(&frame.method).cloned()
        };
        let provider = if handler.is_some() {
            None
        } else {
            self.providers.get(&frame.method).copied()
        };
        if handler.is_none() && provider.is_none() {
            // One diagnostic per ordinary method per outage, reset by an
            // accepted registration. Never log request params or credentials.
            // Bound retained diagnostic keys independently of RPC admission.
            if frame.method.len() <= 512
                && self.missing_reported.len() < 4096
                && self.missing_reported.insert(frame.method.clone())
            {
                eprintln!("hub: NO PROVIDER for {:?}", frame.method);
            }
            self.send(
                caller,
                Frame::error(frame.id, format!("no provider for {}", frame.method)),
            );
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.send(
                caller,
                Frame::error(frame.id, "too many pending calls; call was not submitted"),
            );
            return;
        }
        self.seq += 1;
        let key = self.seq;
        let budget = crate::protocol::provider_timeout(&frame.method, self.options.call_timeout);
        self.pending.insert(
            key,
            Pending {
                caller,
                provider,
                correlation: frame.id,
                method: frame.method.clone(),
                deadline: Instant::now() + budget,
                selected_integration: frame
                    .params
                    .as_ref()
                    .and_then(|params| params["launchIntegrationId"].as_str())
                    .map(str::to_owned),
                launch_permit: None,
                launch_prepared: false,
            },
        );
        if let Some(handler) = handler {
            let timeout = budget;
            let identity = &self.peers[&caller].identity;
            let caller = Caller {
                call_id: key,
                activity_seq: self.peers[&caller].activity_seq,
                federated: identity.federated,
                connection_id: caller,
                authenticated_host: identity.authenticated_host(),
                trusted: identity.trusted(),
                scope: identity.scope().into(),
                plugin_id: identity.plugin_id().into(),
                token_id: identity.token_id.clone(),
            };
            self.jobs.spawn(async move {
                let result = tokio::time::timeout(
                    timeout,
                    handler(caller, frame.params.unwrap_or(Value::Null)),
                )
                .await
                .unwrap_or_else(|_| Err(anyhow!("call timed out")));
                (key, result)
            });
        } else {
            self.send(
                provider.unwrap(),
                Frame {
                    id: key.to_string(),
                    method: frame.method,
                    params: frame.params,
                    provider_caller: self.peers[&provider.unwrap()]
                        .wants_caller_context
                        .then(|| admission_identity.provider_caller(caller)),
                    ..Frame::op("call")
                },
            );
        }
    }
}

async fn run(
    mut options: Options,
    handle: Handle,
    mut commands: mpsc::Receiver<Command>,
    mut stop: oneshot::Receiver<()>,
    status: &watch::Sender<Status>,
) -> Result<()> {
    if options.plugins_dir.is_some() && options.listen.is_none() {
        bail!("external plugins require a configured bus listener");
    }
    let listener = match options.listen {
        Some(addr) => Some(tokio::net::TcpListener::bind(addr).await?),
        None => None,
    };
    let address = listener.as_ref().map(|l| l.local_addr()).transpose()?;
    let mcp_listener = match options.mcp_listen {
        Some(address) => Some(tokio::net::TcpListener::bind(address).await?),
        None => None,
    };
    let mcp_address = mcp_listener.as_ref().map(|l| l.local_addr()).transpose()?;
    if options.launch_lifecycle.is_none() {
        if let (Some(engine), Some(data), Some(config), Some(home), Some(tokens)) = (
            options.engine.clone(),
            options.data_dir.as_ref(),
            options.config_dir.as_ref(),
            options.home_dir.as_ref(),
            options.scoped_tokens.as_ref(),
        ) {
            let facade = crate::services::session_facade::SessionFacade {
                readiness: if mcp_address.is_some() {
                    crate::services::session_facade::Readiness::OwnedRust(handle.clone())
                } else {
                    crate::services::session_facade::Readiness::Disabled
                },
                endpoint: mcp_address
                    .map(|address| {
                        format!("http://{}/mcp", crate::net_address::dial_addr(address)).parse()
                    })
                    .transpose()?,
                expected_hub: "in-process".into(),
                tokens: tokens.clone(),
                directory: config.join("session-mcp"),
                home: home.clone(),
                instructions: String::new(),
            };
            let preparation =
                crate::plugins::launch::Preparation::new(Arc::new(facade), handle.clone());
            options.launch_preparation = Some(preparation.clone());
            options.launch_lifecycle = Some(crate::services::agent_lifecycle::Lifecycle::open(
                data.join("agent-launches.json"),
                Arc::new(engine),
                preparation,
            )?);
        }
    }
    if options.routing.is_none() {
        if let Some(directory) = options.config_dir.clone() {
            options.routing = Some(Arc::new(crate::services::routing::RoutingService::open(
                directory,
            )?));
        }
    }
    if let Some(routing) = options.routing.clone() {
        let engine = options.engine.clone();
        options = crate::services::routing::install(options, routing, engine, handle.clone());
    }
    options =
        crate::services::remote_admin::install(options, address.map(|address| address.port()));
    options = crate::services::uploads::install_front(options, handle.clone());
    if let Some(directory) = options.data_dir.clone() {
        options = crate::services::install(options, handle.clone(), directory)?;
    }
    if let Some(directory) = options.config_dir.clone() {
        if !options.control_plane_only {
            options = crate::services::install_config(options, directory, handle.clone())?;
        }
    }
    if let Some(home) = options
        .home_dir
        .clone()
        .filter(|_| !options.control_plane_only)
    {
        options = crate::services::files::install(options, home);
        options = crate::services::git::install(options);
        options = crate::services::search::install(options);
    }
    if let Some(execution) = options.remote_dispatch_execution.clone() {
        let directory = options
            .config_dir
            .clone()
            .ok_or_else(|| anyhow!("remote execution requires a config directory"))?;
        let receiver =
            crate::services::remote_dispatch::Receiver::open(directory, handle.clone(), execution)?;
        options = receiver.handlers(options);
        options.remote_receiver = Some(receiver);
    }
    let (configured, file_watches, mut filewatch_task) =
        crate::services::filewatch::install(options, handle.clone());
    options = configured;
    options = crate::services::terminals::install(options, handle.clone());
    options = crate::services::wakes::install(options);
    if options.spawn_coordinator.is_some() && options.scoped_tokens.is_some() {
        if let Some(wakes) = options.wakes.clone() {
            let (configured, service) = crate::services::manager_replacements::native::install(
                options,
                wakes,
                handle.clone(),
            )?;
            options = configured;
            options.replacement_service = Some(service);
        }
    }
    if options.engine.is_some() {
        options = crate::services::workflow_artifacts::prepare(options, handle.clone());
    }
    let (configured, mut session_task) =
        crate::services::sessions::install(options, handle.clone()).await?;
    if let Some(service) = &configured.workflow_artifacts {
        service.start();
    }
    options = crate::services::desktop_workflows::install(configured);
    options = crate::services::html_card::install(options);
    options = crate::services::live_controls::install(options, handle.clone());
    options = crate::services::live_streams::install(options);
    let live_streams = options.live_streams.clone();
    let mut live_stream_task = live_streams.clone().map(|streams| {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle.ready().await?;
            streams.run(handle).await
        })
    });
    if let Some(service) = &options.replacement_service {
        if let Err(error) = service.initialize().await {
            eprintln!("manager handoff recovery remains fenced for inspection: {error}");
        }
    }
    let mut wake_task = options.wakes.clone().map(|wakes| {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle.ready().await?;
            wakes.run().await
        })
    });
    if let Some(coordinator) = &options.spawn_coordinator {
        if let Err(error) = coordinator.recover_tracking().await {
            eprintln!("acknowledged launch bookkeeping remains pending: {error}");
        }
    }
    let remote_receiver = options.remote_receiver.clone();
    let remote_sweeper = remote_receiver
        .clone()
        .map(|receiver| tokio::spawn(receiver.run()));
    let analytics_watcher = options.analytics_watcher.clone();
    let mut analytics_task = analytics_watcher.clone().map(|watcher| {
        let hub = handle.clone();
        tokio::spawn(async move { watcher.run(hub).await })
    });
    options = crate::services::progress::install(options);
    let (configured, thresholds, mut threshold_task) =
        crate::services::thresholds::install(options, handle.clone());
    options = configured;
    let (configured, mut jobs_task) = crate::services::jobs::install(options, handle.clone());
    options = configured;
    let (configured, mut nodes_owner) =
        crate::services::nodes::install(options, handle.clone()).await?;
    options = configured;
    let (configured, mut push_observer) = crate::services::push::install(options, handle.clone());
    options = configured;
    let (release, mut releases) = mpsc::unbounded_channel();
    let mut federation_peers = match &options.peers_file {
        Some(path) => crate::federation::load_peers(path)?,
        None => Vec::new(),
    };
    if options.paired_target.is_none() {
        if let Some(directory) = &options.config_dir {
            options.paired_target = crate::services::remote_dispatch::paired::target(directory)?;
        }
    }
    if let Some(target) = &options.paired_target {
        options.federation_peers.push(target.peer.clone());
    }
    let fixed_peers = std::mem::take(&mut options.federation_peers);
    federation_peers.extend(fixed_peers.clone());
    let mut federation = crate::federation::Manager::start(handle.clone(), federation_peers)?;
    let (configured, mut remote_observer) = crate::services::remote_dispatch::runtime::install(
        options,
        handle.clone(),
        federation.routes(),
    )?;
    options = configured;
    let peers_file = options.peers_file.clone();
    options = crate::federation::config::install(options, handle.clone(), peers_file, fixed_peers);
    options = crate::federation::install_resume(options, federation.routes());
    let routes = federation.routes();
    options = options.handler("federation.peers", move |_, _| {
        let peers = routes.peers();
        async move { Ok(serde_json::to_value(peers)?) }
    });
    let (configured, quiescence, sampler_task) =
        crate::services::quiescence::install(options, handle.clone(), federation.routes());
    options = configured;
    let mut quiescence_task = Some(sampler_task);
    #[cfg(feature = "test-support")]
    if let Some(configure) = options.test_after_services.take() {
        options = configure(options);
    }
    let plugin_manager = options.plugins_dir.clone().map(|directory| {
        let mut manager = crate::plugins::Manager::new(
            directory,
            handle.clone(),
            format!(
                "ws://{}/bus",
                crate::net_address::dial_addr(address.unwrap())
            ),
        );
        manager.set_stream_logs(options.plugins_stream_logs);
        manager.set_sidecar_node(options.sidecar_node.clone());
        Arc::new(tokio::sync::Mutex::new(manager))
    });
    let plugin_routes = plugin_manager
        .as_ref()
        .map(|manager| {
            crate::plugins::http::router_with_policy(
                manager.clone(),
                crate::plugins::http::HttpOptions {
                    host_token: options.token.clone(),
                    scoped_tokens: options.scoped_tokens.clone(),
                    plugin_origin: options.plugin_origin.clone(),
                    examples_dir: options.plugin_examples_dir.clone(),
                },
                crate::server::policy::Policy::new(address.unwrap().ip(), &options.trusted_hosts)?,
            )
        })
        .transpose()?;
    if let (Some(destination), Some(examples)) =
        (&options.plugins_dir, &options.plugin_examples_dir)
    {
        crate::plugins::install::seed_bundled(destination, examples)?;
    }
    let (plugin_catalog_ready, plugin_catalog_wait) = watch::channel(false);
    if let Some(manager) = &plugin_manager {
        options = crate::plugins::handlers(options, manager.clone());
        let manager = manager.clone();
        options = options.handler("plugins.tools", move |_, _| {
            let manager = manager.clone();
            let mut ready = plugin_catalog_wait.clone();
            async move {
                ready
                    .wait_for(|ready| *ready)
                    .await
                    .map_err(|_| anyhow!("plugin catalog initialization stopped"))?;
                Ok(manager.lock().await.tools())
            }
        });
    } else {
        options = options.handler("plugins.tools", |_, _| async { Ok(json!([])) });
    }
    let mut plugin_boot = plugin_manager.clone().map(|manager| {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle.ready().await?;
            for error in manager.lock().await.load().await {
                eprintln!("plugin load: {error}");
            }
            plugin_catalog_ready.send_replace(true);
            anyhow::Ok(())
        })
    });
    let (configured, mut relay_owner) = crate::provider_relay::install(options, handle.clone())?;
    options = configured;
    let mut mcp_task = mcp_listener.map(|listener| {
        let hub = handle.clone();
        let token = options.token.clone();
        let store = options.scoped_tokens.clone();
        let ready = options.mcp_ready.clone();
        let trusted_hosts = options.trusted_hosts.clone();
        let upstream = options.upstream_caller.clone();
        let access = options
            .mcp_access
            .clone()
            .expect("MCP policy initialized before runtime");
        tokio::spawn(async move {
            crate::mcp::serve(
                listener,
                hub,
                token,
                store,
                ready,
                trusted_hosts,
                upstream,
                access,
            )
            .await
        })
    });
    let webapp_dir = options.webapp_dir.clone();
    let trusted_hosts = options.trusted_hosts.clone();
    let mut server = listener.map(|listener| {
        let token = options.token.clone();
        let handle = handle.clone();
        let scoped_tokens = options.scoped_tokens.clone();
        let plugins = plugin_routes;
        tokio::spawn(async move {
            crate::server::serve(
                listener,
                handle,
                token,
                scoped_tokens,
                plugins,
                webapp_dir,
                trusted_hosts,
            )
            .await
        })
    });
    let provider_utilities = options.provider_utilities.clone();
    let mut provider_utilities_task = provider_utilities.clone().map(|service| {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle.ready().await?;
            service.run().await
        })
    });
    let external_claudemon = options.external_claudemon.clone();
    let mut external_task = external_claudemon.clone().map(|daemon| {
        let handle = handle.clone();
        tokio::spawn(async move { daemon.run(handle).await })
    });
    let mut core = Core {
        federation: federation.routes(),
        peers: HashMap::new(),
        providers: BTreeMap::new(),
        missing_reported: BTreeSet::new(),
        pending: HashMap::new(),
        demand_counts: BTreeMap::new(),
        seq: 0,
        event_seq: 0,
        options,
        jobs: tokio::task::JoinSet::new(),
        last_revalidation: Instant::now(),
    };
    status.send_replace(Status::Ready {
        address,
        mcp_address,
    });
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut result;
    let mut stopping = false;
    let mut plugin_stop: Option<tokio::task::JoinHandle<Result<()>>> = None;
    loop {
        tokio::select! {
            biased;
            _ = &mut stop, if !stopping => {
                stopping = true;
                let manager = plugin_manager.clone();
                let receiver = remote_receiver.clone();
                let paired = core.options.paired_dispatch.clone();
                plugin_stop = Some(tokio::spawn(async move {
                    if let Some(paired) = paired { paired.close().await; }
                    if let Some(receiver) = receiver { receiver.close().await; }
                    if let Some(manager) = manager { manager.lock().await.stop().await?; }
                    Ok(())
                }));
            }
            relayed = async { (&mut relay_owner.as_mut().unwrap().task).await }, if relay_owner.is_some() => {
                result=match relayed{Ok(Ok(()))=>Err(anyhow!("provider relay stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                if let Some(owner)=relay_owner.take(){owner.stop();if let Some(task)=owner.caller_task{let _=task.await;}}break;
            }
            nodes = async { (&mut nodes_owner.as_mut().unwrap().task).await }, if nodes_owner.is_some() => {
                result=match nodes{Ok(Ok(()))=>Err(anyhow!("node supervisor stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};nodes_owner=None;break;
            }
            pushed = async { (&mut push_observer.as_mut().unwrap().task).await }, if push_observer.is_some() => {
                eprintln!("push: observer disabled after exit: {:?}",pushed);push_observer=None;
            }
            observed = async { (&mut remote_observer.as_mut().unwrap().task).await }, if remote_observer.is_some() => {
                result = match observed { Ok(Ok(())) => Err(anyhow!("remote dispatch observer stopped unexpectedly")), Ok(Err(error)) => Err(error), Err(error) => Err(error.into()) };
                remote_observer = None; break;
            }
            provider_utilities_result=async{provider_utilities_task.as_mut().unwrap().await},if provider_utilities_task.is_some()=>{
                result=match provider_utilities_result{Ok(Ok(()))=>Err(anyhow!("provider readiness observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};provider_utilities_task=None;break;
            }
            external_result=async{external_task.as_mut().unwrap().await},if external_task.is_some()=>{
                result=match external_result{Ok(Ok(()))=>Err(anyhow!("external daemon observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};external_task=None;break;
            }
            stream_result=async{live_stream_task.as_mut().unwrap().await},if live_stream_task.is_some()=>{
                result=match stream_result{Ok(Ok(()))=>Err(anyhow!("live stream observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                live_stream_task=None;break;
            }
            wake_result=async{wake_task.as_mut().unwrap().await},if wake_task.is_some()=>{
                result=match wake_result{Ok(Ok(()))=>Err(anyhow!("fleet wake observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                wake_task=None;break;
            }
            analytics_result=async{analytics_task.as_mut().unwrap().await},if analytics_task.is_some()=>{
                result=match analytics_result{Ok(Ok(()))=>Err(anyhow!("analytics observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                analytics_task=None;break;
            }
            filewatch_result=async{filewatch_task.as_mut().unwrap().await},if filewatch_task.is_some()=>{
                result=match filewatch_result{Ok(Ok(()))=>Err(anyhow!("file watcher stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                filewatch_task=None;break;
            }
            threshold_result=async{threshold_task.as_mut().unwrap().await},if threshold_task.is_some()=>{
                result=match threshold_result{Ok(Ok(()))=>Err(anyhow!("threshold observer stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                threshold_task=None;break;
            }
            quiescence_result=async{quiescence_task.as_mut().unwrap().await},if quiescence_task.is_some()=>{
                result=match quiescence_result{Ok(Ok(()))=>Err(anyhow!("quiescence sampler stopped unexpectedly")),Ok(Err(error))=>Err(error),Err(error)=>Err(error.into())};
                quiescence_task=None;break;
            }
            stopped = async { plugin_stop.as_mut().unwrap().await }, if plugin_stop.is_some() => {
                result = stopped.map_err(anyhow::Error::from).and_then(|result| result);
                plugin_stop = None;
                break;
            }
            loaded = async { plugin_boot.as_mut().unwrap().await }, if plugin_boot.is_some() => {
                plugin_boot = None;
                if let Err(error) = loaded.map_err(anyhow::Error::from).and_then(|result| result) { result = Err(error); break; }
            }
            jobs_result = async { jobs_task.as_mut().unwrap().await }, if jobs_task.is_some() => {
                result = match jobs_result { Ok(Ok(())) => Err(anyhow!("job scheduler stopped unexpectedly")), Ok(Err(e)) => Err(e), Err(e) => Err(e.into()) };
                jobs_task = None; break;
            }
            mcp_result = async { mcp_task.as_mut().unwrap().await }, if mcp_task.is_some() => {
                result = match mcp_result { Ok(Ok(())) => Err(anyhow!("MCP listener stopped unexpectedly")), Ok(Err(e)) => Err(e), Err(e) => Err(e.into()) };
                mcp_task = None; break;
            }
            session_result = async { session_task.as_mut().unwrap().await }, if session_task.is_some() => {
                result = match session_result { Ok(Ok(())) => Err(anyhow!("session observer stopped unexpectedly")),
                    Ok(Err(e)) => Err(e), Err(e) => Err(e.into()) };
                session_task = None; break;
            }
            Some(id) = releases.recv() => core.disconnect(id),
            _ = interval.tick() => core.sweep(),
            Some(completed) = core.jobs.join_next() => {
                if let Ok((key, result)) = completed { core.finish(key, result); }
                // A panicked handler's caller expires at the bounded deadline.
            }
            server_result = async { server.as_mut().unwrap().await }, if server.is_some() => {
                result = match server_result { Ok(Ok(())) => Err(anyhow!("hub listener stopped unexpectedly")),
                    Ok(Err(e)) => Err(e), Err(e) => Err(e.into()) };
                server = None; break;
            }
            Some(command) = commands.recv() => match command {
                Command::BeginLaunch(caller,session,plugin,reply)=>{let _=reply.send(core.begin_launch(&caller,session,plugin));}
                Command::CheckLaunch(permit,finish,reply)=>{let _=reply.send(core.check_launch(&permit,finish));}
                Command::ReplacePeers(peers, reply) => { let _ = reply.send(federation.replace(peers)); }
                Command::PluginTokenMatches(token,id,reply) => {
                    let matches=core.options.plugins.get(&token).is_some_and(|(plugin,_)| plugin==&id);
                    let _=reply.send(matches);
                }
                Command::Plugin {token, identity, reply} => {
                    let active:Vec<_>=core.peers.iter().filter(|(_,p)|matches!(p.identity.kind,Kind::Plugin {..}) && p.credential.as_ref()==Some(&token)).map(|(id,_)|*id).collect();
                    for id in active{core.disconnect(id);}
                    core.options.plugins.remove(&token);
                    if let Some((id,provides))=identity {
                        let prefix=format!("{id}.");let provides=provides.into_iter().filter(|p|p.starts_with(&prefix)&&p!=&prefix&&(!p[prefix.len()..].contains('*')||p==&(prefix.clone()+"*"))).collect();
                        core.options.plugins.insert(token,(id,provides));
                    }
                    let _=reply.send(Ok(()));
                }
                Command::LiveStream(delivery) => {
                    if let Some(event)=core.options.live_streams.as_ref().and_then(|streams|streams.accept_delivery(delivery)){core.publish(event);}
                }
                Command::Publish(event) => core.publish(event),
                Command::Connect(credentials, mode, reply) => {
                    let facade_access=match &mode{ConnectMode::EphemeralFacade(lease)=>Some(lease.clone()),_=>None};
                    let local_facade=matches!(&mode,ConnectMode::Facade);
                    let internal=matches!(&mode,ConnectMode::Service|ConnectMode::Delegated(_));
                    let delegated=matches!(&mode,ConnectMode::Delegated(_));
                    let hello_identity=matches!(&mode,ConnectMode::UserIdentity);
                    let now=chrono::Utc::now().timestamp_millis();
                    if core.peers.len() >= MAX_CONNECTIONS { let _ = reply.send(Err(anyhow!("too many connections"))); continue; }
                    let mut identity = match mode{ConnectMode::Delegated(identity)=>identity,ConnectMode::EphemeralFacade(lease)=>match lease.identity(){Ok(identity)=>identity,Err(error)=>{let _=reply.send(Err(error));continue;}},_=>Identity::host(&core.options.token)};
                    let mut credential = None;
                    if let Some((token, federated)) = credentials {
                        if let Some((plugin_id, provides)) = core.options.plugins.get(&token) {
                            identity.kind = Kind::Plugin { id:plugin_id.clone(), provides:provides.clone() };
                            credential = Some(token.clone());
                        } else if !crate::auth::credential_eq(&core.options.token,&token) || token.is_empty() {
                            let record = core.options.scoped_tokens.as_ref().and_then(|path| Store { path: path.clone() }.lookup(&token));
                            match record { Some(record) => { identity.kind = Kind::Scoped(record); credential = Some(token.clone()); },
                                None => { let _ = reply.send(Err(anyhow!("unauthorized"))); continue; } }
                        }
                        identity.token_id = crate::auth::fingerprint(&token); identity.federated = federated;
                    }
                    if local_facade && (identity.scope()!="operator"||!identity.plugin_id().is_empty()||identity.federated){let _=reply.send(Err(anyhow!("facade delegation requires an operator credential")));continue;}
                    core.seq += 1; let id = core.seq;
                    let (reliable, rx) = mpsc::channel(256);
                    let (events, erx) = mpsc::channel(core.options.event_buffer);
                    let (closed, closing) = watch::channel(false);
                    let close_code=Arc::new(AtomicU16::new(0));let network=Arc::new(AtomicBool::new(false));let (close_done,done)=watch::channel(false);
                    let desynced = Arc::new(Mutex::new(BTreeSet::new()));
                    let hello = Frame { provider_caller:hello_identity.then(||identity.provider_caller(id)),caller_context_version:if hello_identity{1}else{0},scope:identity.scope().into(), methods:identity.methods(), spawn_full_access:identity.may_call("agents.spawn"), ..Frame::op("hello") };
                    core.peers.insert(id, Peer { facade_access,identity, credential, local_facade,delegated,wants_caller_context:false, close_code:close_code.clone(),network:network.clone(),close_done:done,internal, activity_seq:1, last_active_ms:now, last_interaction_ms:now, reports_interaction:false, reliable, events, closed, topics: vec![], demand: vec![], held_demand:BTreeSet::new(), desynced: desynced.clone() });
                    core.send(id, hello);
                    let _ = reply.send(Ok(Connection { close_code,network,close_done,id, handle: handle.clone(), reliable: rx, events: erx, desynced,
                        desync_frames: VecDeque::new(), closed: closing, release: release.clone() }));
                }
                #[cfg(feature = "test-support")]
                Command::TestReceiver(reply)=>{let _=reply.send(core.options.remote_receiver.clone());}
                Command::ProviderConnection(method,reply)=>{let _=reply.send(core.providers.get(&method).copied());}
                Command::EvictProvider(method,expected,reply)=>{
                    let matched=core.providers.get(&method).copied()==Some(expected);
                    if matched{core.disconnect(expected);}
                    let _=reply.send(matched);
                }
                Command::MachineStop(reply) => {
                    let ids:Vec<_>=core.peers.iter().filter(|(id,peer)|!peer.internal&&peer.identity.plugin_id().is_empty()&&!core.providers.values().any(|owner|owner==*id)).map(|(id,_)|*id).collect();
                    let mut waiting=Vec::new();
                    for id in ids {
                        if let Some(peer)=core.peers.get(&id){peer.close_code.store(4001,Ordering::Release);if peer.network.load(Ordering::Acquire){waiting.push(peer.close_done.clone());}}
                        core.disconnect(id);
                    }
                    let _=reply.send(waiting);
                }
                Command::QuiescenceClients(reply) => {
                    let mut clients:Vec<_>=core.peers.iter().map(|(id,peer)|crate::services::quiescence::ClientInfo {
                        connection_id:*id,
                        label:if !peer.identity.plugin_id().is_empty(){format!("plugin {}",peer.identity.plugin_id())}else if peer.internal{"the hub's own loopback client".into()}else if peer.identity.authenticated_host(){"operator client".into()}else{format!("{}-tier client",peer.identity.scope())},
                        activity_seq:peer.activity_seq,
                        idle_active_ms:if peer.reports_interaction{peer.last_interaction_ms}else{peer.last_active_ms},
                        provider:core.providers.values().any(|owner|owner==id),
                        plugin:!peer.identity.plugin_id().is_empty(),internal:peer.internal,
                    }).collect();
                    clients.sort_by_key(|client|client.connection_id);
                    let _=reply.send(clients);
                }
                Command::Health(reply) => {
                    let names: BTreeSet<_> = core.providers.keys().chain(core.options.handlers.keys()).cloned().collect();
                    let _ = reply.send(json!({"status":"ok", "subscribers":core.peers.len(), "methods":names.len(), "methodNames":names,"launchReady":core.launch_ready(),"mcpReady":core.options.mcp_ready.load(Ordering::Acquire)}));
                }
                Command::Frame(id, frame) => core.frame(id, *frame),
            }
        }
    }
    if let Some(service) = &core.options.workflow_artifacts {
        service.close().await;
    }
    if let Some(service) = provider_utilities {
        service.close();
    }
    if let Some(task) = provider_utilities_task {
        let _ = task.await;
    }
    if let Some(daemon) = external_claudemon {
        daemon.close();
    }
    if let Some(task) = external_task {
        let _ = task.await;
    }
    if let Some(streams) = live_streams {
        streams.close();
    }
    if let Some(task) = live_stream_task {
        let _ = task.await;
    }
    if let Some(terminals) = &core.options.terminals {
        terminals.close().await;
    }
    if let Some(watcher) = analytics_watcher {
        watcher.close();
    }
    if let Some(task) = analytics_task {
        let _ = task.await;
    }
    if let Some(watches) = file_watches {
        watches.close();
    }
    if let Some(task) = filewatch_task {
        let _ = task.await;
    }
    if let Some(thresholds) = thresholds {
        thresholds.close();
    }
    if let Some(task) = threshold_task {
        let _ = task.await;
    }
    quiescence.close();
    if let Some(task) = quiescence_task {
        let _ = task.await;
    }
    if let Some(owner) = relay_owner {
        owner.stop();
        let _ = owner.task.await;
        if let Some(task) = owner.caller_task {
            let _ = task.await;
        }
    }
    if let Some(owner) = nodes_owner {
        owner.stop();
        let _ = owner.task.await;
    }
    if let Some(observer) = push_observer {
        observer.stop();
        let _ = observer.task.await;
    }
    if let Some(observer) = remote_observer {
        observer.stop();
        observer.task.abort();
        let _ = observer.task.await;
    }
    if let Some(receiver) = remote_receiver {
        receiver.begin_close();
    }
    if let Some(task) = remote_sweeper {
        let _ = task.await;
    }
    commands.close();
    if let Some(task) = plugin_boot {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = plugin_stop {
        task.abort();
        let _ = task.await;
    }
    for peer in core.peers.values() {
        peer.closed.send_replace(true);
    }
    core.jobs.abort_all();
    while core.jobs.join_next().await.is_some() {}
    if let Some(service) = &core.options.replacement_service {
        if let Err(error) = service.close().await {
            if result.is_ok() {
                result = Err(error);
            }
        }
    }
    if let Some(wakes) = &core.options.wakes {
        wakes.close();
    }
    if let Some(task) = wake_task {
        let _ = task.await;
    }
    federation.shutdown().await;
    if let Some(task) = session_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(coordinator) = &core.options.spawn_coordinator {
        coordinator.close().await;
    }
    if let Some(lifecycle) = &core.options.launch_lifecycle {
        lifecycle.close().await;
    }
    if let Some(task) = mcp_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = jobs_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(server) = server {
        server.abort();
        let _ = server.await;
    }
    result
}

#[cfg(test)]
mod launch_proof_tests {
    use super::*;
    fn fixture() -> (Core, Caller) {
        let (reliable, _) = mpsc::channel(4);
        let (events, _) = mpsc::channel(4);
        let (closed, _) = watch::channel(false);
        let peer = Peer {
            facade_access: None,
            delegated: false,
            wants_caller_context: false,
            close_code: Arc::new(AtomicU16::new(0)),
            network: Arc::new(AtomicBool::new(false)),
            close_done: watch::channel(false).1,
            local_facade: false,
            internal: false,
            activity_seq: 1,
            last_active_ms: 0,
            last_interaction_ms: 0,
            reports_interaction: false,
            identity: Identity::host("fixture"),
            credential: None,
            reliable,
            events,
            closed,
            topics: Vec::new(),
            demand: Vec::new(),
            held_demand: BTreeSet::new(),
            desynced: Arc::new(Mutex::new(BTreeSet::new())),
        };
        let pending = Pending {
            caller: 1,
            provider: None,
            correlation: "caller-request".into(),
            method: "agents.spawn".into(),
            deadline: Instant::now() + Duration::from_secs(10),
            selected_integration: Some("fixture".into()),
            launch_permit: None,
            launch_prepared: false,
        };
        let core = Core {
            federation: Default::default(),
            peers: HashMap::from([(1, peer)]),
            providers: BTreeMap::new(),
            missing_reported: BTreeSet::new(),
            pending: HashMap::from([(7, pending)]),
            demand_counts: BTreeMap::new(),
            seq: 7,
            event_seq: 0,
            options: Options::default(),
            jobs: tokio::task::JoinSet::new(),
            last_revalidation: Instant::now(),
        };
        let caller = Caller {
            call_id: 7,
            activity_seq: 0,
            federated: false,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        };
        (core, caller)
    }
    #[test]
    fn proof_binds_selected_plugin_session_nonce_and_one_pending_call() {
        let (mut core, caller) = fixture();
        assert!(
            core.begin_launch(&caller, "session".into(), "unselected".into())
                .is_err()
        );
        let permit = core
            .begin_launch(&caller, "session".into(), "fixture".into())
            .unwrap();
        assert!(core.check_launch(&permit, false).is_ok());
        let mut forged = permit.clone();
        forged.session_id = "another-session".into();
        assert!(core.check_launch(&forged, false).is_err());
        let mut forged = permit.clone();
        forged.nonce = "forged".into();
        assert!(core.check_launch(&forged, false).is_err());
        assert!(
            core.begin_launch(&caller, "session".into(), "fixture".into())
                .is_err()
        );
        core.check_launch(&permit, true).unwrap();
        assert!(core.check_launch(&permit, false).is_err());
        assert!(
            core.begin_launch(&caller, "another-session".into(), "fixture".into())
                .is_err()
        );
    }
    #[test]
    fn proof_cannot_outlive_connection_deadline_or_owner_authority() {
        let (mut closed, caller) = fixture();
        closed.peers[&caller.connection_id]
            .closed
            .send_replace(true);
        assert!(
            closed
                .begin_launch(&caller, "session".into(), "fixture".into())
                .is_err()
        );
        let (mut core, caller) = fixture();
        let permit = core
            .begin_launch(&caller, "session".into(), "fixture".into())
            .unwrap();
        core.peers[&1].closed.send_replace(true);
        assert!(core.check_launch(&permit, false).is_err());
        core.peers[&1].closed.send_replace(false);
        core.peers.get_mut(&1).unwrap().identity.federated = true;
        assert!(core.check_launch(&permit, false).is_err());
        core.peers.get_mut(&1).unwrap().identity.federated = false;
        core.pending.get_mut(&7).unwrap().deadline = Instant::now() - Duration::from_secs(1);
        assert!(core.check_launch(&permit, false).is_err());
        core.pending.get_mut(&7).unwrap().deadline = Instant::now() + Duration::from_secs(10);
        core.peers.remove(&1);
        assert!(core.check_launch(&permit, false).is_err());
        let (mut core, caller) = fixture();
        core.peers.get_mut(&1).unwrap().identity.kind = Kind::Scoped(crate::auth::Record {
            scope: "operator".into(),
            ..Default::default()
        });
        assert!(
            core.begin_launch(&caller, "session".into(), "fixture".into())
                .is_err()
        );
        let (mut core, caller) = fixture();
        core.pending.get_mut(&7).unwrap().method = "hub:peer/agents.spawn".into();
        assert!(
            core.begin_launch(&caller, "session".into(), "fixture".into())
                .is_err()
        );
    }
    #[test]
    fn cancellation_uses_original_caller_correlation_and_revokes_launch_proof() {
        let (mut core, caller) = fixture();
        let permit = core
            .begin_launch(&caller, "session".into(), "fixture".into())
            .unwrap();
        core.pending.insert(
            8,
            Pending {
                caller: 1,
                provider: None,
                correlation: "sibling".into(),
                method: "config.get".into(),
                deadline: Instant::now() + Duration::from_secs(30),
                selected_integration: None,
                launch_permit: None,
                launch_prepared: false,
            },
        );
        core.cancel_call(2, "caller-request");
        core.cancel_call(1, "7");
        assert!(core.check_launch(&permit, false).is_ok());
        core.cancel_call(1, "caller-request");
        assert!(core.check_launch(&permit, false).is_err());
        assert!(!core.pending.contains_key(&7));
        assert!(core.pending.contains_key(&8));
    }
}

#[cfg(test)]
mod quiescence_activity_tests {
    use super::*;
    #[tokio::test]
    async fn passive_polling_preserves_interaction_and_internal_identity_is_host_owned() {
        let options = Options::default()
            .handler("sessions.snapshots", |_, _| async {
                Ok(Value::Array(vec![]))
            })
            .handler("fleet.quiescence", |caller, _| async move {
                Ok(serde_json::json!({"sequence":caller.activity_seq}))
            });
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let handle = hub.handle();
        let mut native = handle.connect().await.unwrap();
        let internal = handle.connect_service().await.unwrap();
        native
            .send(Frame {
                op: "activity".into(),
                ..Default::default()
            })
            .unwrap();
        let before = handle
            .quiescence_clients()
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.connection_id == native.id)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(3)).await;
        native
            .send(Frame {
                op: "call".into(),
                id: "q1".into(),
                method: "sessions.snapshots".into(),
                ..Default::default()
            })
            .unwrap();
        while native.recv().await.unwrap().id != "q1" {}
        let rows = handle.quiescence_clients().await.unwrap();
        let after = rows.iter().find(|c| c.connection_id == native.id).unwrap();
        assert_eq!(before.idle_active_ms, after.idle_active_ms);
        assert!(after.activity_seq > before.activity_seq);
        assert!(!after.internal);
        assert!(
            rows.iter()
                .find(|c| c.connection_id == internal.id)
                .unwrap()
                .internal
        );
        native
            .send(Frame {
                op: "call".into(),
                id: "q2".into(),
                method: "fleet.quiescence".into(),
                ..Default::default()
            })
            .unwrap();
        let sequence = loop {
            let reply = native.recv().await.unwrap();
            if reply.id == "q2" {
                break reply.result.unwrap()["sequence"].as_u64().unwrap();
            }
        };
        native
            .send(Frame {
                op: "activity".into(),
                ..Default::default()
            })
            .unwrap();
        let later = handle
            .quiescence_clients()
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.connection_id == native.id)
            .unwrap();
        assert!(
            later.activity_seq > sequence,
            "later input invalidates query self-exclusion even within one clock tick"
        );
        drop(native);
        drop(internal);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(test)]
mod conversation_overflow_tests {
    use super::*;
    #[tokio::test]
    async fn slow_peer_gets_ready_after_old_fragments_are_discarded_and_other_events_retained() {
        let mut options = Options::default();
        options.event_buffer = 3;
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let handle = hub.handle();
        let mut peer = handle.connect().await.unwrap();
        assert_eq!(peer.recv().await.unwrap().op, "hello");
        peer.send(Frame {
            op: "subscribe".into(),
            topics: vec!["agent.conversation.one".into(), "fixture.other".into()],
            ..Default::default()
        })
        .unwrap();
        handle.health().await.unwrap();
        peer.recv().await.unwrap();
        handle
            .publish_wait(Event::new("fixture.other", "fixture", json!({"keep":true})))
            .await
            .unwrap();
        for seq in 1..=3 {
            handle.publish_wait(Event::new("agent.conversation.one","fixture",json!({"session_id":"one","seq":seq,"reset":false,"items":[{"kind":"assistant_text","text":format!("old-{seq}")}]}))).await.unwrap();
        }
        handle.health().await.unwrap();
        let other = peer.recv().await.unwrap().event.unwrap();
        assert_eq!(other.topic, "fixture.other");
        assert_eq!(other.data.unwrap()["keep"], true);
        let ready = peer.recv().await.unwrap().event.unwrap();
        assert_eq!(ready.topic, "agent.conversation.one");
        assert_eq!(
            ready.data.unwrap(),
            json!({"session_id":"one","ready":true})
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(40), peer.recv())
                .await
                .is_err(),
            "an older queued fragment followed ready"
        );
        let next = json!({"session_id":"one","seq":4,"reset":false,"items":[{"kind":"assistant_text","text":"new"}]});
        handle
            .publish_wait(Event::new(
                "agent.conversation.one",
                "fixture",
                next.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(
            peer.recv().await.unwrap().event.unwrap().data.unwrap(),
            next
        );
        drop(peer);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(test)]
#[path = "runtime/bus_audit_tests.rs"]
mod bus_audit_tests;

#[cfg(test)]
#[path = "runtime/routing_admission_tests.rs"]
mod routing_admission_tests;
