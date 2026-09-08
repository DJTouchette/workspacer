//! Internal execution composition. Only the compiled claudemon implementation
//! is installed in production. There is deliberately no wire selection field.
//!
//! API 1 describes this Rust contract. Implementation `1` is the compatible
//! claudemon persistence lineage, not a binary/build hash. Incompatible state
//! requires a new lineage and explicit migration; unknown pins never fall back.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::{
    daemon::spawn::{SpawnManagedPayload, SpawnPayload},
    session::{
        store::MessageOutcome, windows::PersistedModelSelection, ConversationStore, SessionStore,
    },
    store::Db,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineRef {
    pub id: String,
    pub api_version: u32,
    pub implementation_version: String,
}
impl Default for EngineRef {
    fn default() -> Self {
        Self {
            id: "claudemon-v1".into(),
            api_version: 1,
            implementation_version: "1".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineLease {
    #[serde(flatten)]
    pub engine: EngineRef,
    pub generation: u64,
    /// Availability on the current host, never inferred from the provider name.
    pub readiness: String,
}

/// Trusted first-party code only. Authority is the already-admitted launch
/// payload: engines cannot register capabilities or obtain a token from this API.
/// Native controls retain their existing delivery/retry semantics (no invented
/// end-to-end command deduplication for routes without request identities).
pub(crate) trait ExecutionEngineV1: Send + Sync {
    fn describe(&self) -> EngineRef;
    fn preflight(&self, request: &SpawnManagedPayload, bin: &str) -> Result<()>;
    // Keep the admitted launch context explicit at the engine boundary.
    #[allow(clippy::too_many_arguments)]
    fn start(
        &self,
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        id: &str,
        request: SpawnManagedPayload,
        bin: String,
        selection: Option<PersistedModelSelection>,
    ) -> bool;
    fn send(&self, store: &SessionStore, id: &str, text: String) -> MessageOutcome;
    fn decide(&self, store: &SessionStore, id: &str, approve: bool) -> bool;
    fn cancel(&self, store: &SessionStore, id: &str) -> bool;
    fn stop(&self, store: &SessionStore, id: &str) -> bool;
    fn start_pty(
        &self,
        _store: &SessionStore,
        _conv: &ConversationStore,
        _db: &Db,
        _id: &str,
        _request: SpawnPayload,
        _selection: Option<PersistedModelSelection>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "execution engine does not support PTY starts",
        )
            .into_response()
    }
    fn snapshot(
        &self,
        store: &SessionStore,
        id: &str,
    ) -> Option<crate::session::state::SessionState> {
        store.native_get(id)
    }
    fn answer(
        &self,
        store: &SessionStore,
        id: &str,
        answer: crate::session::store::ManagedAnswer,
    ) -> bool {
        store.native_submit_managed_answer(id, answer)
    }
    fn resolve(&self, store: &SessionStore, id: &str, decision: serde_json::Value) -> bool {
        store.native_resolve_decision(id, decision)
    }
    fn signal(&self, store: &SessionStore, id: &str, signal: crate::protocol::Signal) -> bool {
        store.wrapper(id).is_some_and(|handle| {
            handle
                .tx
                .send(crate::protocol::WrapperMessage::Signal { signal })
                .is_ok()
        })
    }
    fn dispose(&self, _id: &str) {}
}
struct ClaudemonEngine;
impl ExecutionEngineV1 for ClaudemonEngine {
    fn describe(&self) -> EngineRef {
        EngineRef::default()
    }
    fn preflight(&self, request: &SpawnManagedPayload, bin: &str) -> Result<()> {
        // No processes, auth probes or configuration writes. Process spawn can
        // still fail after this check; the existing driver reports that failure.
        let executable = |path: &std::path::Path| {
            let Ok(meta) = path.metadata() else {
                return false;
            };
            if !meta.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        };
        let path = std::path::Path::new(bin);
        let found = if path.is_absolute() {
            executable(path)
        } else if path.components().count() > 1 {
            executable(&std::path::Path::new(&request.cwd).join(path))
        } else {
            let search = matches!(request.provider.as_str(), "claude" | "codex" | "copilot")
                .then(|| request.env.get("PATH").cloned())
                .flatten()
                .or_else(|| std::env::var("PATH").ok())
                .unwrap_or_default();
            std::env::split_paths(&search).any(|dir| {
                if executable(&dir.join(bin)) {
                    return true;
                }
                #[cfg(windows)]
                {
                    return executable(&dir.join(format!("{bin}.exe")));
                }
                #[cfg(not(windows))]
                {
                    false
                }
            })
        };
        if !found {
            bail!("execution engine unavailable: provider executable not found or not executable");
        }
        Ok(())
    }
    fn start(
        &self,
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        id: &str,
        request: SpawnManagedPayload,
        bin: String,
        selection: Option<PersistedModelSelection>,
    ) -> bool {
        crate::daemon::spawn::start_native_managed(store, conv, db, id, request, bin, selection)
    }
    fn start_pty(
        &self,
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        id: &str,
        request: SpawnPayload,
        selection: Option<PersistedModelSelection>,
    ) -> axum::response::Response {
        crate::daemon::spawn::start_native_pty(store, conv, db, id, request, selection)
    }
    fn send(&self, store: &SessionStore, id: &str, text: String) -> MessageOutcome {
        store.native_submit_message(id, text)
    }
    fn decide(&self, store: &SessionStore, id: &str, approve: bool) -> bool {
        store.native_submit_managed_decision(id, approve)
    }
    fn cancel(&self, store: &SessionStore, id: &str) -> bool {
        store.native_interrupt_managed(id)
    }
    fn stop(&self, store: &SessionStore, id: &str) -> bool {
        store.native_terminate_managed(id)
    }
}
struct Registration {
    engine: Arc<dyn ExecutionEngineV1>,
    enabled: bool,
}
#[derive(Clone)]
struct Binding {
    engine: Arc<dyn ExecutionEngineV1>,
    lease: EngineLease,
    fence: Arc<Mutex<EventFence>>,
    commands: Arc<Mutex<()>>,
}
// Every command captures a binding before dispatch. Retirement waits for an
// in-flight command; a delayed captured call refuses instead of targeting the
// next lifetime's native channels. Events use a separate lock because a test
// engine may synchronously emit a result while handling a command.
struct BoundEngine(Binding);
impl BoundEngine {
    fn call<T>(&self, fallback: T, call: impl FnOnce(&dyn ExecutionEngineV1) -> T) -> T {
        let _command = self.0.commands.lock().unwrap();
        {
            let fence = self.0.fence.lock().unwrap();
            if fence.retired || fence.disposed {
                return fallback;
            }
        }
        call(self.0.engine.as_ref())
    }
}
impl ExecutionEngineV1 for BoundEngine {
    fn snapshot(
        &self,
        store: &SessionStore,
        id: &str,
    ) -> Option<crate::session::state::SessionState> {
        // A stopped current lease remains observable and resumable.
        let _commands = self.0.commands.lock().unwrap();
        if self.0.fence.lock().unwrap().retired {
            return None;
        }
        self.0.engine.snapshot(store, id)
    }
    fn answer(
        &self,
        store: &SessionStore,
        id: &str,
        answer: crate::session::store::ManagedAnswer,
    ) -> bool {
        self.call(false, |engine| engine.answer(store, id, answer))
    }
    fn resolve(&self, store: &SessionStore, id: &str, decision: serde_json::Value) -> bool {
        self.call(false, |engine| engine.resolve(store, id, decision))
    }
    fn signal(&self, store: &SessionStore, id: &str, signal: crate::protocol::Signal) -> bool {
        self.call(false, |engine| {
            if matches!(
                signal,
                crate::protocol::Signal::Sigterm | crate::protocol::Signal::Sigkill
            ) {
                self.0.fence.lock().unwrap().draining = true;
            }
            engine.signal(store, id, signal)
        })
    }
    fn describe(&self) -> EngineRef {
        self.0.lease.engine.clone()
    }
    fn preflight(&self, _: &SpawnManagedPayload, _: &str) -> Result<()> {
        bail!("bound handle cannot admit a session")
    }
    fn start(
        &self,
        _: &SessionStore,
        _: &ConversationStore,
        _: &Db,
        _: &str,
        _: SpawnManagedPayload,
        _: String,
        _: Option<PersistedModelSelection>,
    ) -> bool {
        false
    }
    fn send(&self, store: &SessionStore, id: &str, text: String) -> MessageOutcome {
        self.call(MessageOutcome::WrapperGone, |engine| {
            engine.send(store, id, text)
        })
    }
    fn decide(&self, store: &SessionStore, id: &str, approve: bool) -> bool {
        self.call(false, |engine| engine.decide(store, id, approve))
    }
    fn cancel(&self, store: &SessionStore, id: &str) -> bool {
        self.call(false, |engine| engine.cancel(store, id))
    }
    fn stop(&self, store: &SessionStore, id: &str) -> bool {
        self.call(false, |engine| {
            self.0.fence.lock().unwrap().draining = true;
            engine.stop(store, id)
        })
    }
}
#[derive(Default)]
struct EventFence {
    retired: bool,
    disposed: bool,
    draining: bool,
    sequence: u64,
}
#[derive(Clone)]
pub(crate) struct ExecutionScope {
    id: String,
    binding: Binding,
}
impl ExecutionScope {
    pub(crate) fn collect<T>(
        &self,
        id: &str,
        sequence: Option<u64>,
        apply: impl FnOnce() -> T,
    ) -> Option<T> {
        if id != self.id {
            return None;
        }
        let mut fence = self.binding.fence.lock().unwrap();
        if fence.retired || fence.disposed {
            return None;
        }
        if let Some(sequence) = sequence {
            if sequence <= fence.sequence {
                return None;
            }
            fence.sequence = sequence;
        }
        Some(apply())
    }
    pub(crate) fn finish(&self) {
        let mut fence = self.binding.fence.lock().unwrap();
        if fence.disposed {
            return;
        }
        fence.disposed = true;
        drop(fence);
        self.binding.engine.dispose(&self.id);
    }
}
#[derive(Clone)]
pub(crate) struct EngineRegistry {
    registrations: Arc<Mutex<HashMap<String, Registration>>>,
    bindings: Arc<Mutex<HashMap<String, Binding>>>,
    starts: Arc<Mutex<()>>,
}
pub(crate) struct Prepared {
    engine: Arc<dyn ExecutionEngineV1>,
    previous: Option<EngineLease>,
}
impl Default for EngineRegistry {
    fn default() -> Self {
        let registry = Self {
            registrations: Arc::default(),
            bindings: Arc::default(),
            starts: Arc::default(),
        };
        registry
            .register(Arc::new(ClaudemonEngine))
            .expect("compiled engine descriptor");
        registry
    }
}
impl EngineRegistry {
    fn register(&self, engine: Arc<dyn ExecutionEngineV1>) -> Result<()> {
        let descriptor = engine.describe();
        if descriptor.api_version != 1
            || descriptor.id.is_empty()
            || !descriptor
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || descriptor.implementation_version.trim().is_empty()
        {
            bail!("invalid execution engine descriptor");
        }
        let mut registrations = self.registrations.lock().unwrap();
        if registrations.contains_key(&descriptor.id) {
            bail!("execution engine already registered");
        }
        registrations.insert(
            descriptor.id.clone(),
            Registration {
                engine,
                enabled: true,
            },
        );
        Ok(())
    }
    #[cfg(test)]
    fn prepare(
        &self,
        previous: Option<&EngineLease>,
        request: &SpawnManagedPayload,
        bin: &str,
    ) -> Result<Prepared> {
        let reference = previous.map(|p| p.engine.clone()).unwrap_or_default();
        self.prepare_selected(&reference, previous, request, bin)
    }
    pub(crate) fn prepare_resume(
        &self,
        previous: Option<&EngineLease>,
        source: Option<&EngineLease>,
        request: &SpawnManagedPayload,
        bin: &str,
    ) -> Result<Prepared> {
        if let (Some(previous), Some(source)) = (previous, source) {
            if previous.engine != source.engine {
                bail!("execution resume identity conflicts with target pin");
            }
        }
        let reference = source
            .or(previous)
            .map(|lease| lease.engine.clone())
            .unwrap_or_default();
        self.prepare_selected(&reference, previous, request, bin)
    }
    fn prepare_selected(
        &self,
        reference: &EngineRef,
        previous: Option<&EngineLease>,
        request: &SpawnManagedPayload,
        bin: &str,
    ) -> Result<Prepared> {
        let engine = {
            let registrations = self.registrations.lock().unwrap();
            let Some(registration) = registrations.get(&reference.id) else {
                bail!("execution engine unavailable: unknown identity");
            };
            if !registration.enabled {
                bail!("execution engine unavailable: disabled");
            }
            if registration.engine.describe() != *reference {
                bail!("execution engine unavailable: incompatible pinned version");
            }
            registration.engine.clone()
        };
        engine.preflight(request, bin)?;
        Ok(Prepared {
            engine,
            previous: previous.cloned(),
        })
    }
    // Mirrors ExecutionEngineV1::start plus the validated engine selection.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        &self,
        prepared: Prepared,
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        id: &str,
        request: SpawnManagedPayload,
        bin: String,
        selection: Option<PersistedModelSelection>,
    ) -> Result<bool> {
        self.admit(prepared, store, db, id, |engine, scoped| {
            engine.start(scoped, conv, db, id, request, bin, selection)
        })
    }
    pub(crate) fn start_pty(
        &self,
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        id: &str,
        request: SpawnPayload,
        selection: Option<PersistedModelSelection>,
    ) -> Result<axum::response::Response> {
        let previous = db.execution_lease(id)?;
        // PTY launch has its own native argv contract. This temporary request
        // is only an executable/cwd preflight, never the launch payload.
        let probe: SpawnManagedPayload = serde_json::from_value(serde_json::json!({
            "provider": "claude", "cwd": request.cwd, "env": request.env
        }))?;
        let resume_id = request
            .argv
            .windows(2)
            .find(|pair| pair[0] == "--resume")
            .map(|pair| pair[1].as_str())
            .or_else(|| {
                request
                    .argv
                    .iter()
                    .find_map(|arg| arg.strip_prefix("--resume="))
            });
        let source = resume_id
            .map(|id| db.execution_lease(id))
            .transpose()?
            .flatten();
        let prepared =
            self.prepare_resume(previous.as_ref(), source.as_ref(), &probe, &request.argv[0])?;
        if prepared.engine.describe() != EngineRef::default() {
            bail!("execution engine unavailable for PTY resume");
        }
        self.admit(prepared, store, db, id, |engine, scoped| {
            let response = engine.start_pty(scoped, conv, db, id, request, selection);
            if !response.status().is_success() {
                if let Some(scope) = &scoped.execution_scope {
                    scope.finish();
                }
            }
            response
        })
    }
    fn admit<T>(
        &self,
        prepared: Prepared,
        store: &SessionStore,
        db: &Db,
        id: &str,
        start: impl FnOnce(&dyn ExecutionEngineV1, &SessionStore) -> T,
    ) -> Result<T> {
        let _start = self.starts.lock().unwrap();
        // Recheck availability at lease admission; later disable cannot retarget
        // this captured Arc. Publication happens only after durable pinning.
        {
            let registrations = self.registrations.lock().unwrap();
            let descriptor = prepared.engine.describe();
            let Some(registration) = registrations.get(&descriptor.id) else {
                bail!("execution engine unavailable");
            };
            if !registration.enabled || !Arc::ptr_eq(&registration.engine, &prepared.engine) {
                bail!("execution engine unavailable");
            }
        }
        let old = self.bindings.lock().unwrap().get(id).cloned();
        if let Some(old) = &old {
            let fence = old.fence.lock().unwrap();
            if !fence.disposed && !fence.draining {
                bail!("execution session already has an active lease");
            }
        }
        let lease =
            db.claim_execution_lease(id, prepared.previous.as_ref(), &prepared.engine.describe())?;
        if let Some(old) = old {
            let _commands = old.commands.lock().unwrap();
            old.fence.lock().unwrap().retired = true;
        }
        let binding = Binding {
            engine: prepared.engine.clone(),
            lease,
            fence: Arc::default(),
            commands: Arc::default(),
        };
        self.bindings
            .lock()
            .unwrap()
            .insert(id.into(), binding.clone());
        let mut scoped = store.clone();
        scoped.execution_scope = Some(ExecutionScope {
            id: id.into(),
            binding,
        });
        Ok(start(prepared.engine.as_ref(), &scoped))
    }

    pub(crate) fn bound(&self, id: &str) -> Option<Arc<dyn ExecutionEngineV1>> {
        self.bindings
            .lock()
            .unwrap()
            .get(id)
            .map(|b| Arc::new(BoundEngine(b.clone())) as Arc<dyn ExecutionEngineV1>)
    }
    pub(crate) fn metadata(&self, id: &str) -> Option<EngineLease> {
        self.bindings
            .lock()
            .unwrap()
            .get(id)
            .map(|b| b.lease.clone())
    }
    pub(crate) fn readiness(&self, lease: &mut EngineLease) {
        let registrations = self.registrations.lock().unwrap();
        lease.readiness = if registrations
            .get(&lease.engine.id)
            .is_some_and(|r| r.enabled && r.engine.describe() == lease.engine)
        {
            "ready"
        } else {
            "unavailable"
        }
        .into();
    }
    /// Internal bounded drain. Timeout is explicit and retains the pinned
    /// handle: a timeout does not prove a provider process died. Native adapters
    /// remain responsible for child/group termination before their finalizer.
    #[allow(dead_code)]
    pub(crate) async fn drain(
        &self,
        store: &SessionStore,
        id: &str,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let binding = self
            .bindings
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("execution lease not found"))?;
        BoundEngine(binding.clone()).stop(store, id);
        loop {
            if binding.fence.lock().unwrap().disposed {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("execution cancel deadline exceeded; pinned handle retained until native cleanup");
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + std::time::Duration::from_millis(10)),
            )
            .await;
        }
    }
    #[cfg(test)]
    fn disable(&self, id: &str) {
        let _starts = self.starts.lock().unwrap();
        if let Some(registration) = self.registrations.lock().unwrap().get_mut(id) {
            registration.enabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{apply_updates, AgentUpdate, UsageAcc};
    use crate::session::state::SessionMode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Replay {
        descriptor: Option<EngineRef>,
        lives: Mutex<HashMap<String, (SessionStore, ConversationStore, u64)>>,
        disposals: AtomicUsize,
        decisions: Mutex<Vec<bool>>,
    }
    impl ExecutionEngineV1 for Replay {
        fn describe(&self) -> EngineRef {
            self.descriptor.clone().unwrap_or_else(|| EngineRef {
                id: "replay-test-v1".into(),
                ..Default::default()
            })
        }
        fn preflight(&self, request: &SpawnManagedPayload, _: &str) -> Result<()> {
            // This test implementation has no credentials, process or facade.
            if request.yolo || request.mcp.is_some() || !request.env.is_empty() {
                bail!("replay accepts no authority");
            }
            Ok(())
        }
        fn start(
            &self,
            store: &SessionStore,
            conv: &ConversationStore,
            _: &Db,
            id: &str,
            request: SpawnManagedPayload,
            _: String,
            _: Option<PersistedModelSelection>,
        ) -> bool {
            let generation = store.claim_generation(id);
            store.register_managed(id, &request.cwd, &request.provider);
            self.lives
                .lock()
                .unwrap()
                .insert(id.into(), (store.clone(), conv.clone(), generation));
            false
        }
        fn send(&self, _: &SessionStore, id: &str, text: String) -> MessageOutcome {
            let lives = self.lives.lock().unwrap();
            let (store, conv, _) = &lives[id];
            apply_updates(
                store,
                conv,
                id,
                vec![AgentUpdate::AssistantText(text), AgentUpdate::Idle],
                &mut SessionMode::Unknown,
                &mut UsageAcc::default(),
            );
            MessageOutcome::Sent
        }
        fn decide(&self, _: &SessionStore, _: &str, approve: bool) -> bool {
            self.decisions.lock().unwrap().push(approve);
            true
        }
        fn cancel(&self, _: &SessionStore, _: &str) -> bool {
            true
        }
        fn stop(&self, _: &SessionStore, id: &str) -> bool {
            let life = self.lives.lock().unwrap().get(id).cloned().unwrap();
            life.0.deregister_managed(id, life.2)
        }
        fn dispose(&self, _: &str) {
            self.disposals.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn request() -> SpawnManagedPayload {
        serde_json::from_value(serde_json::json!({"provider":"claude", "cwd":"/", "yolo":false}))
            .unwrap()
    }
    fn db() -> Db {
        Db::open(crate::testtmp::db_path("execution")).unwrap()
    }
    fn start_replay(
        store: &SessionStore,
        conv: &ConversationStore,
        db: &Db,
        replay: &Arc<Replay>,
        id: &str,
    ) {
        let previous = db.execution_lease(id).unwrap();
        let prepared = store
            .engines
            .prepare_selected(&replay.describe(), previous.as_ref(), &request(), "unused")
            .unwrap();
        store
            .engines
            .start(
                prepared,
                store,
                conv,
                db,
                id,
                request(),
                "unused".into(),
                None,
            )
            .unwrap();
    }
    #[test]
    fn replay_uses_real_start_control_projection_and_disposal() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let db = db();
        let replay = Arc::new(Replay::default());
        store.engines.register(replay.clone()).unwrap();
        let mut events = store.subscribe();
        start_replay(&store, &conv, &db, &replay, "replay");
        assert_eq!(
            store
                .get("replay")
                .unwrap()
                .execution_engine
                .unwrap()
                .engine
                .id,
            "replay-test-v1"
        );
        assert_eq!(
            store.submit_message("replay", "deterministic".into()),
            MessageOutcome::Sent
        );
        assert!(!conv.snapshot("replay").unwrap().1.is_empty());
        assert!(store.submit_managed_decision("replay", false));
        assert_eq!(*replay.decisions.lock().unwrap(), vec![false]);
        assert!(store.interrupt_managed("replay"));
        store.engines.disable("replay-test-v1");
        assert!(store
            .engines
            .prepare_selected(&replay.describe(), None, &request(), "unused")
            .is_err());
        assert_eq!(
            store.submit_message("replay", "still pinned".into()),
            MessageOutcome::Sent
        );
        assert!(store.terminate_managed("replay"));
        assert!(!store.terminate_managed("replay"));
        assert_eq!(replay.disposals.load(Ordering::SeqCst), 1);
        let mut ends = 0;
        while let Ok(event) = events.try_recv() {
            if event.event == "SessionEnd" {
                ends += 1;
            }
        }
        assert_eq!(ends, 1);
    }
    #[test]
    fn persistence_resume_pins_and_stale_prepare_refuses_without_fallback() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let db = db();
        let replay = Arc::new(Replay::default());
        store.engines.register(replay.clone()).unwrap();
        start_replay(&store, &conv, &db, &replay, "resume");
        let first = db.execution_lease("resume").unwrap().unwrap();
        assert_eq!(first.generation, 1);
        assert!(store
            .engines
            .prepare(Some(&first), &request(), "unused")
            .is_ok());
        let stale = store
            .engines
            .prepare(Some(&first), &request(), "unused")
            .unwrap();
        assert!(store.terminate_managed("resume"));
        start_replay(&store, &conv, &db, &replay, "resume");
        assert_eq!(db.execution_lease("resume").unwrap().unwrap().generation, 2);
        assert!(store.terminate_managed("resume"));
        assert!(store
            .engines
            .start(
                stale,
                &store,
                &conv,
                &db,
                "resume",
                request(),
                "unused".into(),
                None
            )
            .is_err());
        let fresh_host = SessionStore::new();
        assert!(fresh_host
            .engines
            .prepare(Some(&first), &request(), "unused")
            .is_err());
        let mut incompatible = first;
        incompatible.engine = EngineRef::default();
        incompatible.engine.implementation_version = "2".into();
        assert!(fresh_host
            .engines
            .prepare(Some(&incompatible), &request(), "unused")
            .is_err());
    }
    #[test]
    fn collector_rejects_duplicate_out_of_order_retired_and_terminal_updates() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let db = db();
        let replay = Arc::new(Replay::default());
        store.engines.register(replay.clone()).unwrap();
        start_replay(&store, &conv, &db, &replay, "fence");
        let scope = replay.lives.lock().unwrap()["fence"]
            .0
            .execution_scope
            .clone()
            .unwrap();
        let count = AtomicUsize::new(0);
        for seq in [2, 2, 1, 3] {
            scope.collect("fence", Some(seq), || count.fetch_add(1, Ordering::SeqCst));
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(scope.collect("other", Some(4), || ()).is_none());
        store.terminate_managed("fence");
        start_replay(&store, &conv, &db, &replay, "fence");
        assert!(scope.collect("fence", Some(5), || ()).is_none());
    }
    #[test]
    fn unavailable_preflight_never_publishes_and_replay_cannot_request_authority() {
        let store = SessionStore::new();
        assert!(store
            .engines
            .prepare(None, &request(), "/definitely-missing-execution-binary")
            .is_err());
        assert!(store.list().is_empty());
        let replay = Arc::new(Replay::default());
        store.engines.register(replay.clone()).unwrap();
        let mut elevated = request();
        elevated.yolo = true;
        assert!(store
            .engines
            .prepare_selected(&replay.describe(), None, &elevated, "unused")
            .is_err());
        assert!(store.engines.register(replay).is_err());
    }
    #[test]
    fn native_controls_keep_the_existing_channels_and_values() {
        let store = SessionStore::new();
        let (input, mut messages) = tokio::sync::mpsc::unbounded_channel();
        let (decision, mut decisions) = tokio::sync::mpsc::unbounded_channel();
        let (interrupt, mut interrupts) = tokio::sync::mpsc::unbounded_channel();
        store.register_managed("native", "/", "codex");
        store.register_managed_input("native", input);
        store.register_managed_decision("native", decision);
        store.register_managed_interrupt("native", interrupt);
        let engine = ClaudemonEngine;
        assert_eq!(
            engine.send(&store, "native", "exact input".into()),
            MessageOutcome::Sent
        );
        assert_eq!(messages.try_recv().unwrap(), "exact input");
        assert!(engine.decide(&store, "native", false));
        assert!(!decisions.try_recv().unwrap());
        assert!(engine.cancel(&store, "native"));
        assert_eq!(interrupts.try_recv(), Ok(()));
        assert!(engine.stop(&store, "native"));
        assert!(messages.try_recv().is_err());
    }
    #[test]
    fn invalid_descriptors_are_rejected_atomically() {
        let registry = EngineRegistry::default();
        for descriptor in [
            EngineRef {
                id: " ".into(),
                ..Default::default()
            },
            EngineRef {
                id: "candidate".into(),
                api_version: 2,
                ..Default::default()
            },
            EngineRef {
                id: "candidate".into(),
                implementation_version: "".into(),
                ..Default::default()
            },
        ] {
            assert!(registry
                .register(Arc::new(Replay {
                    descriptor: Some(descriptor),
                    ..Default::default()
                }))
                .is_err());
        }
        assert_eq!(registry.registrations.lock().unwrap().len(), 1);
    }

    #[test]
    fn legacy_hydration_and_durable_resume_keep_known_lineage() {
        let path = crate::testtmp::db_path("execution-reopen");
        let db = Db::open(&path).unwrap();
        let hook = serde_json::from_value(serde_json::json!({"session_id":"legacy", "event":"SessionStart", "payload":{"cwd":"/isolated"}})).unwrap();
        db.record_event(&hook).unwrap();
        let store = SessionStore::new();
        store.hydrate(db.load_recent_sessions(10).unwrap());
        store.hydrate_execution(&db);
        let legacy = store.get("legacy").unwrap().execution_engine.unwrap();
        assert_eq!(legacy.engine, EngineRef::default());
        assert_eq!(legacy.generation, 0);
        let lease = db
            .claim_execution_lease("legacy", None, &legacy.engine)
            .unwrap();
        drop(db);
        let db = Db::open(path).unwrap();
        assert_eq!(db.execution_lease("legacy").unwrap(), Some(lease));
        let unknown = EngineRef {
            implementation_version: "future".into(),
            ..Default::default()
        };
        let previous = db.execution_lease("legacy").unwrap();
        db.claim_execution_lease("legacy", previous.as_ref(), &unknown)
            .unwrap();
        store.hydrate_execution(&db);
        assert_eq!(
            store
                .get("legacy")
                .unwrap()
                .execution_engine
                .unwrap()
                .readiness,
            "unavailable"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_deadline_is_explicit_and_does_not_claim_process_death() {
        let store = SessionStore::new();
        let binding = Binding {
            engine: Arc::new(ClaudemonEngine),
            lease: EngineLease {
                engine: Default::default(),
                generation: 1,
                readiness: "ready".into(),
            },
            fence: Arc::default(),
            commands: Arc::default(),
        };
        store
            .engines
            .bindings
            .lock()
            .unwrap()
            .insert("hung".into(), binding.clone());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(20);
        assert!(store
            .engines
            .drain(&store, "hung", deadline)
            .await
            .unwrap_err()
            .to_string()
            .contains("deadline"));
        assert!(!binding.fence.lock().unwrap().disposed);
        assert!(store.engines.bound("hung").is_some());
    }
}
