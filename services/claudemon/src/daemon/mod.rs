pub mod api;
pub mod embedded;
pub mod heartbeat;
pub mod hook;
pub mod init;
pub mod mcp_ask;
pub mod oneshot;
// Test-only: derives contracts/claudemon-routes.json from the two routers above
// and fails when the fixture drifts. See the module doc.
#[cfg(test)]
mod routes_contract;
pub mod spawn;
mod worktree_admission;
pub use worktree_admission::{WorktreeAdmission, WorktreeMaintenance};
pub mod wrapper_ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::session::{conversation, ConversationStore, HookEvent, SessionStore};
use crate::store::Db;

/// How many recent sessions to restore into the live list on startup. Newest
/// first; the rest stay in the DB. Generous enough to cover any realistic set
/// of open agents without flooding the UI with stale history.
const SESSION_HYDRATE_LIMIT: usize = 100;

/// How often the maintenance sweep runs: evict archived sessions from the
/// in-memory map and GC old rows from SQLite. These bound slow growth over days
/// of uptime, not a hot path, so hourly is ample.
const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

pub struct ServeConfig {
    pub host: String,
    pub hook_port: u16,
    pub api_port: u16,
    pub db_path: PathBuf,
}

/// Callback address for the single active daemon. Resettable so an embedded
/// daemon can be stopped and restarted without stale provider callbacks.
pub static API_BASE: CallbackBase = CallbackBase(std::sync::RwLock::new(None));
pub struct CallbackBase(std::sync::RwLock<Option<String>>);
impl CallbackBase {
    pub fn get(&self) -> Option<String> {
        self.0.read().unwrap().clone()
    }
    pub fn set(&self, value: String) -> Result<(), String> {
        *self.0.write().unwrap() = Some(value);
        Ok(())
    }
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct DaemonLease;
impl DaemonLease {
    fn acquire() -> Result<Self> {
        anyhow::ensure!(
            !RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel),
            "a claudemon runtime is already active in this process"
        );
        Ok(Self)
    }
}
impl Drop for DaemonLease {
    fn drop(&mut self) {
        *API_BASE.0.write().unwrap() = None;
        RUNNING.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub async fn run(cfg: ServeConfig) -> Result<()> {
    let _lease = DaemonLease::acquire()?;
    #[cfg(windows)]
    confine_to_job();
    run_controlled(cfg, process_shutdown(), None).await
}

async fn process_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
            _ = wait_for_parent_exit() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = wait_for_parent_exit() => {},
    }
}

async fn run_controlled(
    cfg: ServeConfig,
    shutdown: impl std::future::Future<Output = ()>,
    control: Option<embedded::Control>,
) -> Result<()> {
    // Bind both ports before starting workers. A partial startup failure drops
    // the first listener and cannot leak background tasks into a host runtime.
    let hook_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.hook_port).parse()?;
    let api_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.api_port).parse()?;
    let hook_listener = TcpListener::bind(hook_addr)
        .await
        .with_context(|| format!("binding hook server to {hook_addr}"))?;
    let api_listener = TcpListener::bind(api_addr)
        .await
        .with_context(|| format!("binding api server to {api_addr}"))?;
    let hook_addr = hook_listener.local_addr()?;
    let api_addr = api_listener.local_addr()?;
    let callback_host = if api_addr.ip().is_unspecified() {
        if api_addr.is_ipv4() {
            "127.0.0.1".to_owned()
        } else {
            "[::1]".to_owned()
        }
    } else if api_addr.is_ipv6() {
        format!("[{}]", api_addr.ip())
    } else {
        api_addr.ip().to_string()
    };
    let _ = API_BASE.set(format!("http://{callback_host}:{}", api_addr.port()));
    let store = SessionStore::new();
    if let Some(control) = &control {
        *control.cleanup.lock().unwrap() = Some(store.clone());
    }
    let db = Db::open(&cfg.db_path)
        .with_context(|| format!("opening db at {}", cfg.db_path.display()))?;
    tracing::info!(db = %cfg.db_path.display(), "sqlite store ready");
    // Usage folds beside the database, so a restart reads only what each
    // transcript gained instead of every transcript from its first line.
    crate::session::usage::persist_usage_under(cfg.db_path.with_file_name("usage-cache"));

    // Repopulate the in-memory list from the DB so sessions survive a daemon
    // restart: prior agents reappear (as stopped — no process is attached, so
    // they show as resumable, not live) and can be resumed with
    // `claude --resume <id>`. Bounded to the most-recent window. Nothing is
    // deleted; stale ones come back archived (see `SessionState::is_archived`)
    // so they stay out of the default list but remain reachable.
    match db.load_recent_sessions(SESSION_HYDRATE_LIMIT) {
        Ok(sessions) if !sessions.is_empty() => {
            let count = sessions.len();
            store.hydrate(sessions);
            store.hydrate_execution(&db);
            tracing::info!(count, "hydrated prior sessions from db");
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(?err, "hydrating sessions from db failed"),
    }

    // Persistence runs out-of-band: subscribe to the raw-hook broadcast and
    // write each event to SQLite without blocking the hook handler's response.
    spawn_persistence_task(db.clone(), store.clone(), store.subscribe_hooks());

    // Periodic durability sweep: bound the in-memory session map and GC old
    // SQLite rows so neither grows without limit over long uptime. The FIRST
    // sweep runs immediately inside this task (off the boot path, on a blocking
    // thread) so a long-lived DB is still trimmed on a daemon that rarely stays
    // up — WITHOUT the synchronous startup prune that used to run here and could
    // delay the api-server bind past the client's health-check window.
    spawn_maintenance_task(store.clone(), db.clone());

    // Transcript tailer: daemon-owned conversation parsing. Streams structured
    // deltas to clients so they never re-read the JSONL themselves.
    let conv = ConversationStore::new();
    if let Some(control) = &control {
        *control.conversations.lock().unwrap() = Some(conv.clone());
    }
    conversation::spawn_tailer(store.clone(), conv.clone());

    // Account-usage poller: fills the 5h/7d/monthly gauges for stream-transport
    // Claude sessions, whose wire events carry reset times but (in practice) no
    // utilization %. Costs zero tokens. See session::account_usage.
    //
    // By default every CONFIGURED account is polled, so the gauges are right on
    // a daemon with nothing running. The user can restrict that to accounts
    // with a live session (`usage.pollOnBoot: false` in the workspacer config,
    // passed by process launchers as WORKSPACER_USAGE_POLL_ON_BOOT or by an
    // embedded host as an explicit option). Read once here rather than inside
    // the loop: this is a boot decision.
    let poll_idle_accounts = control
        .as_ref()
        .and_then(|control| control.options.usage_poll_on_boot)
        .unwrap_or_else(crate::session::account_usage::poll_on_boot_enabled);
    tracing::info!(
        poll_on_boot = poll_idle_accounts,
        "account usage poller starting",
    );
    crate::session::account_usage::spawn_poller(store.clone(), poll_idle_accounts);

    // Ghost sweep: a session row whose teardown escaped (superseded generation,
    // crash between spawn and register) advertises a live-looking mode forever
    // for a process that is gone. Every 5 minutes, rows with no live plumbing
    // and 15+ minutes of silence flip to Stopped (hook-adopted sessions revive
    // on their next hook event, so a false positive self-heals; a ghost never
    // does). See SessionStore::sweep_ghost_sessions.
    {
        let store = store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                store.sweep_ghost_sessions(time::Duration::minutes(15));
            }
        });
    }

    tracing::info!(%hook_addr, "hook server listening");
    tracing::info!(%api_addr, "api server listening");

    // Same Host allowlist as the API router below — the hook port is an
    // unauthenticated write surface and was the one router that never got it.
    let hook_app = hook::router_with_host(store.clone(), Some(cfg.host.clone()));
    // Retained past the `store` move into ApiState so shutdown can kill the PTY
    // children the daemon spawned (they have no kill-on-drop).
    let store_for_shutdown = store.clone();
    let api_app = api::router_with_host(
        api::ApiState { store, db, conv },
        // Accept the daemon's own bind address as a valid Host (loopback is
        // always accepted); wildcard binds add nothing (see `AllowedHosts`).
        Some(cfg.host.clone()),
    );

    // Fence only spawn handlers: parked MCP questions must not delay shutdown.
    // The read guard spans admission through PTY registration, so no HTTP
    // handler can publish a child after shutdown takes its cleanup snapshot.
    let spawn_closed = std::sync::Arc::new(tokio::sync::RwLock::new(false));
    let spawn_gate = spawn_closed.clone();
    let api_app = api_app.layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let gate = spawn_gate.clone();
            async move {
                let path = request.uri().path();
                if matches!(path, "/sessions/spawn" | "/sessions/spawn-managed") {
                    let closed = gate.read().await;
                    if *closed {
                        use axum::response::IntoResponse;
                        return (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            "daemon is shutting down",
                        )
                            .into_response();
                    }
                    let response = next.run(request).await;
                    drop(closed);
                    response
                } else {
                    next.run(request).await
                }
            }
        },
    ));

    let command_task = control
        .map(|control| embedded::serve_commands(control, api_app.clone(), hook_addr, api_addr));
    let mut hook_task =
        tokio::spawn(async move { axum::serve(hook_listener, hook_app).tcp_nodelay(true).await });
    let mut api_task =
        tokio::spawn(async move { axum::serve(api_listener, api_app).tcp_nodelay(true).await });
    let result = tokio::select! {
        result = &mut hook_task => result.context("hook listener task failed").and_then(|r| r.context("hook listener failed")),
        result = &mut api_task => result.context("API listener task failed").and_then(|r| r.context("API listener failed")),
        _ = shutdown => Ok(()),
    };
    // Stop ingress first, then release managed drivers and kill/reap PTYs.
    // The embedded owner drops its dedicated runtime after this returns,
    // cancelling all daemon workers and dropping kill-on-drop provider children.
    hook_task.abort();
    api_task.abort();
    if let Some(task) = command_task {
        task.abort();
        let _ = task.await;
    }
    // A spawn may still be waiting for an incomplete HTTP request body. Do not
    // let that client prevent the embedded owner from cancelling its runtime.
    // On timeout the owner still sweeps PTYs during/after runtime destruction.
    let drained =
        tokio::time::timeout(std::time::Duration::from_secs(2), spawn_closed.write()).await;
    let admission_result = match drained {
        Ok(mut closed) => {
            *closed = true;
            Ok(())
        }
        Err(_) => Err(anyhow::anyhow!(
            "timed out draining active spawn requests during shutdown"
        )),
    };
    store_for_shutdown.shutdown_children().await;
    // Let managed drivers observe closed input channels and run their teardown.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    admission_result?;
    result?;

    Ok(())
}

/// Resolves when our parent process exits, so a daemon launched by the desktop
/// app never outlives it (no orphaned listeners holding ports 7890/7891).
///
/// Two independent triggers race, and whichever fires first wins:
///
///  1. **stdin EOF** — the launcher hands us a stdin pipe and holds the write
///     end open for its whole life; when it dies the kernel closes the pipe and
///     our read returns EOF. Fastest path when it works.
///  2. **parent-pid poll** — the safety net for Windows, where libuv marks the
///     stdio pipe handles inheritable, so a sibling daemon inherits a duplicate
///     of *our* stdin write handle. That duplicate keeps the pipe open after the
///     launcher dies, so EOF never arrives and the daemons keep each other's
///     ports hostage. Polling `WORKSPACER_PARENT_PID` for the launcher's death
///     doesn't depend on the pipe, so it frees the ports regardless.
///
/// Gated on `WORKSPACER_PARENT_PID` (set by the launcher): when it's unset — a
/// manual `claudemon serve` from a terminal — neither trigger resolves, so the
/// daemon keeps running.
async fn wait_for_parent_exit() {
    let Some(pid_os) = std::env::var_os("WORKSPACER_PARENT_PID") else {
        std::future::pending::<()>().await;
        return;
    };
    let parent_pid: Option<u32> = pid_os.to_str().and_then(|s| s.trim().parse().ok());

    // Path 1: stdin EOF. We discard any bytes the parent writes; only EOF matters.
    let eof = async {
        use tokio::io::AsyncReadExt;
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 256];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => break, // parent closed the pipe (exited)
                Ok(_) => {}              // ignore anything the parent writes
            }
        }
    };

    // Path 2: watch the launcher pid — a pinned process handle on Windows
    // (immune to PID reuse, fires the instant the launcher exits), a liveness
    // poll elsewhere. If the pid didn't parse, this arm never resolves and we
    // fall back to the EOF path alone.
    let poll = async {
        match parent_pid {
            Some(pid) => parent_exit_signal(pid).await,
            None => std::future::pending::<()>().await,
        }
    };

    tokio::select! {
        _ = eof => {}
        _ = poll => {}
    }
}

/// How often the parent-pid safety net checks whether the launcher is still alive.
#[cfg(unix)]
const PARENT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Resolves once the launcher process has exited (Unix: 1s liveness poll —
/// PIDs recycle slowly there and `kill -0` is cheap).
#[cfg(unix)]
async fn parent_exit_signal(pid: u32) {
    loop {
        tokio::time::sleep(PARENT_POLL_INTERVAL).await;
        if !parent_alive(pid) {
            break; // launcher gone
        }
    }
}

/// Resolves once the launcher process has exited.
///
/// Windows pins a handle to the launcher ONCE and blocks on it. Re-opening the
/// pid per poll (the old approach) races Windows' aggressive PID reuse: if
/// another process claimed the launcher's pid between polls, the watcher
/// believed the launcher was alive forever and the daemon held ports
/// 7890/7891 until killed by hand. A pinned handle references the original
/// process object, which becomes signaled on exit no matter who now owns the
/// pid number.
#[cfg(windows)]
async fn parent_exit_signal(pid: u32) {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const INFINITE: u32 = 0xFFFF_FFFF;
    type Handle = *mut core::ffi::c_void;
    extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return; // can't open the pid → the launcher is already gone
    }
    // Raw pointers aren't Send; carry the handle across the blocking task as a
    // plain integer.
    let handle_bits = handle as usize;
    let _ = tokio::task::spawn_blocking(move || unsafe {
        let h = handle_bits as Handle;
        WaitForSingleObject(h, INFINITE); // blocks until the launcher exits
        CloseHandle(h);
    })
    .await;
}

/// Whether process `pid` is still running. Probes existence without disturbing
/// the process; on any ambiguity it errs toward "alive" so we never shut down a
/// daemon whose launcher is actually still up.
#[cfg(unix)]
fn parent_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Signal 0 delivers nothing; it just checks that the pid is a live process.
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true, // exists, but we may not signal it
        Err(_) => false,           // ESRCH (gone) or anything else → treat as dead
    }
}

/// Put the daemon (and all future children) in a kill-on-job-close Windows
/// job object. When the daemon's last handle to the job closes — which its own
/// death guarantees — the OS terminates every process in the job: all the PTY
/// children (claude.exe, conhost, shells) die with the daemon instead of
/// orphaning. Nested jobs are fine on Win8+; failure is logged and non-fatal
/// (the clean-shutdown path still runs kill_all_ptys).
#[cfg(windows)]
fn confine_to_job() {
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    // JobObjectExtendedLimitInformation
    const JOB_OBJECT_INFO_CLASS_EXTENDED: u32 = 9;
    type Handle = *mut core::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    extern "system" {
        fn CreateJobObjectW(attrs: *mut core::ffi::c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: u32,
            info: *const core::ffi::c_void,
            len: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            tracing::warn!("job object confinement unavailable (CreateJobObject failed)");
            return;
        }
        let mut info = ExtendedLimits::default();
        info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JOB_OBJECT_INFO_CLASS_EXTENDED,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<ExtendedLimits>() as u32,
        ) == 0
        {
            tracing::warn!("job object confinement unavailable (SetInformationJobObject failed)");
            CloseHandle(job);
            return;
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            tracing::warn!("job object confinement unavailable (AssignProcessToJobObject failed)");
            CloseHandle(job);
            return;
        }
        // Intentionally leak `job`: it must stay open for the daemon's whole
        // life — its close (at process death) is exactly the kill trigger.
        tracing::info!("confined to kill-on-close job object (child PTYs die with the daemon)");
    }
}

/// Periodic durability sweep. Evicts archived (Stopped + stale) sessions from
/// the in-memory map and prunes the matching SQLite rows, on a slow interval.
/// Both operations are conservative and resume-safe (see
/// `SessionStore::evict_stale_stopped` and `Db::prune_archived`).
fn spawn_maintenance_task(store: SessionStore, db: Db) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(MAINTENANCE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately, so the startup GC pass happens here
        // in the background (not synchronously before the servers bind), then
        // every MAINTENANCE_INTERVAL after.
        loop {
            interval.tick().await;
            let evicted = store.evict_stale_stopped();
            if evicted > 0 {
                tracing::info!(evicted, "evicted archived sessions from memory");
            }
            let db_inner = db.clone();
            match tokio::task::spawn_blocking(move || {
                db_inner.prune_archived(SESSION_HYDRATE_LIMIT)
            })
            .await
            {
                Ok(Ok(n)) if n > 0 => {
                    tracing::info!(pruned = n, "pruned archived sessions from db")
                }
                Ok(Ok(_)) => {}
                Ok(Err(err)) => tracing::warn!(?err, "db retention prune failed"),
                Err(err) => tracing::warn!(?err, "db retention prune task panicked"),
            }
        }
    });
}

fn spawn_persistence_task(
    db: Db,
    store: SessionStore,
    mut rx: tokio::sync::broadcast::Receiver<HookEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let db_inner = db.clone();
                    // The model this session was ASKED for, carried into the
                    // same statement that CREATES its row. A spawn records it
                    // in memory before any row exists (rows are born from the
                    // first hook event), so writing it separately would UPDATE
                    // nothing and the row would then be inserted with a NULL —
                    // and a daemon restart would revert a 1M session to the
                    // table's guess for its marker-stripped transcript id.
                    let requested_selection = store.requested_model_selection(&event.session_id);
                    // …and WHICH ACCOUNT it bills against, for the same reason
                    // and through the same statement. `None` here is a real
                    // answer — a session the daemon did not spawn genuinely has
                    // no attribution, and NULL says so rather than defaulting
                    // it onto the primary account.
                    let config_root = store.config_root(&event.session_id);
                    // Run the synchronous sqlite write on the blocking pool so
                    // we don't tie up an async worker on file I/O.
                    let result = tokio::task::spawn_blocking(move || {
                        db_inner.record_event_with_spawn_facts(
                            &event,
                            crate::store::SpawnFacts {
                                requested_selection: requested_selection.as_ref(),
                                config_root: config_root.as_deref(),
                            },
                        )
                    })
                    .await
                    .unwrap_or_else(|join_err| Err(anyhow::anyhow!(join_err)));
                    if let Err(err) = result {
                        tracing::warn!(?err, "persisting hook event failed");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "persistence task lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("hook broadcast closed; persistence task exiting");
                    break;
                }
            }
        }
    });
}
