pub mod api;
pub mod hook;
pub mod init;
pub mod mcp_ask;
pub mod spawn;
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

pub struct ServeConfig {
    pub host: String,
    pub hook_port: u16,
    pub api_port: u16,
    pub db_path: PathBuf,
}

/// The daemon's own API base URL (`http://host:api_port`), set once at startup.
/// Provider adapters need it to hand agents callback endpoints on this daemon —
/// e.g. registering the `/mcp/ask/:session_id` AskUserQuestion MCP server with
/// a Codex spawn. `None` until `run` is called (unit tests).
pub static API_BASE: once_cell::sync::OnceCell<String> = once_cell::sync::OnceCell::new();

pub async fn run(cfg: ServeConfig) -> Result<()> {
    let _ = API_BASE.set(format!(
        "http://{}:{}",
        if cfg.host == "0.0.0.0" {
            "127.0.0.1"
        } else {
            cfg.host.as_str()
        },
        cfg.api_port
    ));
    // Windows: confine the daemon — and every PTY child it spawns (claude.exe,
    // conhost, shells) — in a kill-on-job-close job object, so the whole tree
    // dies with the daemon no matter how the daemon dies (clean exit, crash,
    // Task-Manager kill). Without this a hard-killed daemon skips
    // kill_all_ptys and orphans its agents. No-op elsewhere.
    #[cfg(windows)]
    confine_to_job();

    let store = SessionStore::new();
    let db = Db::open(&cfg.db_path)
        .with_context(|| format!("opening db at {}", cfg.db_path.display()))?;
    tracing::info!(db = %cfg.db_path.display(), "sqlite store ready");

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
            tracing::info!(count, "hydrated prior sessions from db");
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(?err, "hydrating sessions from db failed"),
    }

    // Persistence runs out-of-band: subscribe to the raw-hook broadcast and
    // write each event to SQLite without blocking the hook handler's response.
    spawn_persistence_task(db.clone(), store.subscribe_hooks());

    // Transcript tailer: daemon-owned conversation parsing. Streams structured
    // deltas to clients so they never re-read the JSONL themselves.
    let conv = ConversationStore::new();
    conversation::spawn_tailer(store.clone(), conv.clone());

    let hook_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.hook_port).parse()?;
    let api_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.api_port).parse()?;

    let hook_listener = TcpListener::bind(hook_addr)
        .await
        .with_context(|| format!("binding hook server to {hook_addr}"))?;
    let api_listener = TcpListener::bind(api_addr)
        .await
        .with_context(|| format!("binding api server to {api_addr}"))?;

    tracing::info!(%hook_addr, "hook server listening");
    tracing::info!(%api_addr, "api server listening");

    let hook_app = hook::router(store.clone());
    // Retained past the `store` move into ApiState so shutdown can kill the PTY
    // children the daemon spawned (they have no kill-on-drop).
    let store_for_shutdown = store.clone();
    let api_app = api::router_with_host(
        api::ApiState { store, db, conv },
        // Accept the daemon's own bind address as a valid Host (loopback is
        // always accepted); wildcard binds add nothing (see `AllowedHosts`).
        Some(cfg.host.clone()),
    );

    let hook_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(hook_listener, hook_app).await {
            tracing::error!(?err, "hook server crashed");
        }
    });
    let api_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(api_listener, api_app).await {
            tracing::error!(?err, "api server crashed");
        }
    });

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = hook_task => {},
            _ = api_task => {},
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, shutting down");
            }
            _ = wait_for_parent_exit() => {
                tracing::info!("parent process exited; shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = hook_task => {},
            _ = api_task => {},
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
            }
            _ = wait_for_parent_exit() => {
                tracing::info!("parent process exited; shutting down");
            }
        }
    }

    // Kill the PTY children we spawned so they don't outlive the daemon (and the
    // launcher). Managed-provider children use kill_on_drop and are reaped as the
    // runtime tears down; the portable-pty children are not, so kill them here.
    store_for_shutdown.kill_all_ptys();

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

fn spawn_persistence_task(db: Db, mut rx: tokio::sync::broadcast::Receiver<HookEvent>) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let db_inner = db.clone();
                    // Run the synchronous sqlite write on the blocking pool so
                    // we don't tie up an async worker on file I/O.
                    let result = tokio::task::spawn_blocking(move || db_inner.record_event(&event))
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
