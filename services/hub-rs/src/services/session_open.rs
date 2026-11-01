//! Hub-owned sidebar set: which sessions every client shows.
//!
//! A session joins `open` when this hub sees it live (or starting). Nothing
//! leaves on its own: when this hub run sees a session end (the user or the
//! agent ended it) it stays in `open` with an `ended` mark. A session stopped
//! while no hub was watching (the app or host closed) is never seen ending, so
//! it stays unmarked. Every client renders a stopped, open, unmarked session as
//! **Paused** (resumable), and a stopped, open, marked one as **Ended**. The
//! roster ([`super::roster`]) always lists `open` sessions, however old.
//!
//! The user's **Hide** is the shared archive: archiving removes the id from
//! `open` and `ended`, and restoring adds it back (marked ended unless it is
//! known to be live). `sessionOpen.set` is a low-level pin/unpin; UIs use the
//! archive.
//!
//! Like `sessionArchive` this is shared VIEW state: it never starts, stops or
//! forgets a session. It replaces native's per-device `kept_open` settings,
//! which the hub used to read off the host disk for `sessions.kept`.
//!
//! The document is `{version, open: {sessionId: sinceMs}, ended: {sessionId:
//! endedAtMs}}` (`ended` ⊆ `open`). Every accepted change bumps `version`,
//! persists, and publishes the whole document as `sessionOpen.changed`, so a
//! client converges from either the event or a `sessionOpen.get`.
//!
//! Observation reads `agent.snapshot` on this hub's own bus, so it works the
//! same whether rows come from an embedded engine or from desktop main as the
//! session provider; the embedded sessions service also feeds every row it is
//! about to publish, so its publish gate never races the bus. `ended` marks are
//! fenced once hub shutdown begins: what a closing app takes down stays Paused.
//!
//! Cleanup prompt: because nothing leaves on its own, the document carries a
//! derived `cleanup: {ended, threshold, suggested, snoozedAt}` block, recomputed
//! on every change and on every read. `ended` counts sidebar sessions that are
//! stopped, marked ended and not archived; Paused and live ones never count.
//! `suggested = threshold > 0 && ended >= max(threshold, snoozedAt + 10)`;
//! `threshold` is hub config `sessions.cleanupThreshold` (default 20, 0 off).
//! `snoozedAt` is persisted; it clears once `ended` drops under the threshold.
//!
//! Upgrade: with no persisted document, the set is seeded once from native's
//! `kept_open` ids (unmarked), every live row, and every row stopped within the
//! retired 24h window (marked ended unless kept), so nothing a user currently
//! sees disappears.
use super::session_archive::SessionArchive;
use crate::{Handle, protocol::Event};
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Upper bound on sidebar ids; past it the oldest ended entries go first,
/// then the oldest overall.
pub const MAX_OPEN: usize = 10_000;
/// `sessions.kept` answered at most this many ids (newest first).
const MAX_KEPT: usize = 200;
const MAX_ID_BYTES: usize = 256;
/// Bound on the in-memory "seen live this run" set.
const MAX_SEEN: usize = 20_000;
/// `sessions.cleanupThreshold` when hub config does not set it.
pub const DEFAULT_CLEANUP_THRESHOLD: u64 = 20;
/// A snoozed prompt returns after this many more sessions end.
pub const SNOOZE_STEP: u64 = 10;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.chars().any(char::is_control)
        && !id.contains(['/', '\\'])
}

fn entries(map: &Value) -> Map<String, Value> {
    map.as_object()
        .into_iter()
        .flatten()
        .filter(|(id, at)| valid_id(id) && at.as_i64().is_some())
        .map(|(id, at)| (id.clone(), at.clone()))
        .collect()
}

/// Well-formed `open`, `ended ⊆ open`, capped.
fn normalize(open: Map<String, Value>, ended: Map<String, Value>) -> Value {
    let ended: Map<String, Value> = ended
        .into_iter()
        .filter(|(id, _)| open.contains_key(id))
        .collect();
    let mut document = json!({
        "version":0,
        "open":open,
        "ended":ended,
        "cleanup":{"ended":0,"threshold":DEFAULT_CLEANUP_THRESHOLD,"suggested":false,"snoozedAt":null},
    });
    cap(&mut document);
    document
}

fn cap(document: &mut Value) {
    loop {
        let len = document["open"].as_object().map_or(0, Map::len);
        if len <= MAX_OPEN {
            return;
        }
        let at = |v: &Value| v.as_i64().unwrap_or(0);
        let victim = document["ended"]
            .as_object()
            .and_then(|ended| {
                ended
                    .iter()
                    .min_by(|a, b| at(a.1).cmp(&at(b.1)).then_with(|| a.0.cmp(b.0)))
            })
            .or_else(|| {
                document["open"].as_object().and_then(|open| {
                    open.iter()
                        .min_by(|a, b| at(a.1).cmp(&at(b.1)).then_with(|| a.0.cmp(b.0)))
                })
            })
            .map(|(id, _)| id.clone());
        let Some(victim) = victim else { return };
        document["open"].as_object_mut().unwrap().remove(&victim);
        document["ended"].as_object_mut().unwrap().remove(&victim);
    }
}

fn row_id(row: &Value) -> Option<&str> {
    row["sessionId"]
        .as_str()
        .or_else(|| row["session_id"].as_str())
        .filter(|id| valid_id(id))
}

/// What one observed row means for the set.
#[derive(Debug, PartialEq, Eq)]
enum Observation {
    Open,
    Ended,
    Neither,
}

fn classify(row: &Value, shells: &BTreeSet<String>) -> Observation {
    // A federated row belongs to its own hub's set.
    if row["hub"].as_str().is_some_and(|h| !h.is_empty()) {
        return Observation::Neither;
    }
    let ctx = super::roster::Ctx {
        shells: shells.clone(),
        ..Default::default()
    };
    if super::roster::live(row) || row["starting"] == true || super::roster::starting(row, &ctx) {
        Observation::Open
    } else if super::roster::ended(row) {
        Observation::Ended
    } else {
        Observation::Neither
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub struct SessionOpen {
    document: Mutex<Value>,
    path: Option<PathBuf>,
    hub: Option<Handle>,
    archive: Option<Arc<SessionArchive>>,
    /// Ids this hub run observed open; only these get an `ended` mark.
    seen: Mutex<BTreeSet<String>>,
    closing: AtomicBool,
    /// The one-time upgrade seed (live + recently stopped rows) is pending.
    seed_pending: AtomicBool,
    /// Native's legacy kept ids imported at open; they stay unmarked.
    legacy_kept: BTreeSet<String>,
    /// Ids observed live right now; never counted as ended.
    live_now: Mutex<BTreeSet<String>>,
    /// Hub config, for `sessions.cleanupThreshold` (absent before config).
    config: Mutex<Option<Arc<super::config::Config>>>,
    stop: tokio::sync::watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SessionOpen {
    /// Open the persisted set. `legacy` is native's settings file, read only
    /// when no set has been persisted yet (part of the one-time seed).
    pub fn open(
        path: Option<PathBuf>,
        hub: Option<Handle>,
        archive: Option<Arc<SessionArchive>>,
        legacy: Option<PathBuf>,
    ) -> Self {
        let mut document = normalize(Map::new(), Map::new());
        let mut missing = path.is_none();
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) if value.is_object() => {
                        document = normalize(entries(&value["open"]), entries(&value["ended"]));
                        document["version"] = json!(value["version"].as_i64().unwrap_or(0).max(0));
                        if let Some(snoozed) = value["cleanup"]["snoozedAt"].as_u64() {
                            document["cleanup"]["snoozedAt"] = json!(snoozed);
                        }
                    }
                    _ => {
                        eprintln!("session sidebar: persisted document is invalid; starting empty")
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = true,
                Err(e) => eprintln!("session sidebar: could not read persisted document: {e}"),
            }
        }
        let mut legacy_kept = BTreeSet::new();
        if missing && let Some(legacy) = legacy {
            let kept = super::kept_sessions::read(Some(&legacy));
            let mut open = Map::new();
            for row in kept["sessions"].as_array().into_iter().flatten() {
                if let (Some(id), Some(since)) = (row["sessionId"].as_str(), row["since"].as_i64())
                    && valid_id(id)
                {
                    open.insert(id.into(), json!(since));
                    legacy_kept.insert(id.to_owned());
                }
            }
            if !open.is_empty() {
                document = normalize(open, Map::new());
            }
        }
        let (stop, _) = tokio::sync::watch::channel(false);
        Self {
            document: Mutex::new(document),
            path,
            hub,
            archive,
            seen: Mutex::new(BTreeSet::new()),
            closing: AtomicBool::new(false),
            seed_pending: AtomicBool::new(missing),
            legacy_kept,
            live_now: Mutex::new(BTreeSet::new()),
            config: Mutex::new(None),
            stop,
            task: Mutex::new(None),
        }
    }

    pub fn get(&self) -> Value {
        let mut document = self.document.lock().unwrap().clone();
        self.derive(&mut document);
        document
    }

    /// Whether this hub last observed `id` live (or starting).
    pub fn observed_live(&self, id: &str) -> bool {
        self.live_now.lock().unwrap().contains(id)
    }

    /// Read `sessions.cleanupThreshold` from this hub's config from now on.
    pub fn set_config(&self, config: Arc<super::config::Config>) {
        *self.config.lock().unwrap() = Some(config);
    }

    fn threshold(&self) -> u64 {
        self.config
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|config| config.get()["sessions"]["cleanupThreshold"].as_u64())
            .unwrap_or(DEFAULT_CLEANUP_THRESHOLD)
    }

    /// The ids `cleanup.ended` counts, with their ended-mark times: on the
    /// sidebar, marked ended, not archived, not observed live.
    fn counted(&self, document: &Value) -> Vec<(String, i64)> {
        let archived = self
            .archive
            .as_ref()
            .map(|archive| archive.get()["archived"].clone())
            .unwrap_or(Value::Null);
        let live = self.live_now.lock().unwrap();
        document["ended"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, _)| {
                document["open"].get(id.as_str()).is_some()
                    && archived.get(id.as_str()).is_none()
                    && !live.contains(id.as_str())
            })
            .map(|(id, at)| (id.clone(), at.as_i64().unwrap_or(0)))
            .collect()
    }

    /// Recompute the derived `cleanup` block (keeping a persisted snooze).
    fn derive(&self, document: &mut Value) {
        let ended = self.counted(document).len() as u64;
        let threshold = self.threshold();
        let mut snoozed = document["cleanup"]["snoozedAt"].as_u64();
        if threshold == 0 || ended < threshold {
            snoozed = None;
        }
        let bar = threshold.max(snoozed.map_or(0, |at| at + SNOOZE_STEP));
        document["cleanup"] = json!({
            "ended": ended,
            "threshold": threshold,
            "suggested": threshold > 0 && ended >= bar,
            "snoozedAt": snoozed,
        });
    }

    /// Every id on the sidebar (ended or not): what the roster lists.
    pub fn ids(&self) -> BTreeSet<String> {
        self.document.lock().unwrap()["open"]
            .as_object()
            .map(|open| open.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// `sessions.kept`'s historical shape: open, unended ids, newest first.
    pub fn kept(&self) -> Value {
        let document = self.document.lock().unwrap();
        let ended = document["ended"].as_object();
        let mut rows: Vec<(String, i64)> = document["open"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, _)| !ended.is_some_and(|ended| ended.contains_key(*id)))
            .map(|(id, at)| (id.clone(), at.as_i64().unwrap_or(0)))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows.truncate(MAX_KEPT);
        json!({"sessions": rows.into_iter().map(|(id, since)| json!({"sessionId": id, "since": since})).collect::<Vec<_>>()})
    }

    fn archived(&self, id: &str) -> bool {
        self.archive
            .as_ref()
            .is_some_and(|archive| archive.get()["archived"].get(id).is_some())
    }

    /// Apply `edit` to a copy; if it changed anything, bump, persist, commit,
    /// publish. `strict` (an explicit client request) refuses an unpersisted
    /// change; an observation commits in memory and only logs, because what
    /// the hub saw is true either way.
    fn change(&self, strict: bool, edit: impl FnOnce(&mut Value)) -> Result<(Value, bool)> {
        let mut document = self.document.lock().unwrap();
        let mut next = document.clone();
        edit(&mut next);
        cap(&mut next);
        self.derive(&mut next);
        if next["open"] == document["open"]
            && next["ended"] == document["ended"]
            && next["cleanup"] == document["cleanup"]
        {
            return Ok((document.clone(), false));
        }
        next["version"] = json!(document["version"].as_i64().unwrap_or(0) + 1);
        if let Some(path) = &self.path
            && let Err(error) = super::atomic_json(path, &next, false)
        {
            if strict {
                return Err(error);
            }
            eprintln!(
                "session sidebar: version {} could not be persisted; it will be lost on restart: {error}",
                next["version"]
            );
        }
        *document = next;
        if let Some(hub) = &self.hub
            && let Err(error) =
                hub.publish(Event::new("sessionOpen.changed", "hub", document.clone()))
        {
            eprintln!(
                "session sidebar: version {} is committed but its change event could not be queued: {error}",
                document["version"]
            );
        }
        Ok((document.clone(), true))
    }

    /// `{sessionId, open}`: low-level pin (true) or unpin (false, also clears
    /// the ended mark). Idempotent; a no-op keeps the version and publishes
    /// nothing. UIs hide through `sessionArchive` instead.
    pub fn set(&self, params: Value) -> Result<Value> {
        let Some(id) = params["sessionId"].as_str() else {
            bail!("sessionOpen.set requires {{ sessionId, open }}");
        };
        let Some(open) = params["open"].as_bool() else {
            bail!("sessionOpen.set requires a boolean open");
        };
        if !valid_id(id) {
            bail!("sessionOpen.set: invalid sessionId");
        }
        if open && self.archived(id) {
            bail!(
                "sessionOpen.set: a hidden (archived) session cannot be pinned; restore it first"
            );
        }
        Ok(self
            .change(true, |doc| {
                if open {
                    if doc["open"].get(id).is_none() {
                        doc["open"][id] = json!(now_ms());
                    }
                } else {
                    doc["open"].as_object_mut().unwrap().remove(id);
                    doc["ended"].as_object_mut().unwrap().remove(id);
                }
            })?
            .0)
    }

    /// Hide: an archived session leaves the sidebar on every client.
    pub fn forget(&self, id: &str) {
        if valid_id(id) {
            let _ = self.change(false, |doc| {
                doc["open"].as_object_mut().unwrap().remove(id);
                doc["ended"].as_object_mut().unwrap().remove(id);
            });
            self.seen.lock().unwrap().remove(id);
        }
    }

    /// Hide many at once (one sidebar change).
    pub fn forget_many(&self, ids: &[String]) {
        let ids: Vec<&String> = ids.iter().filter(|id| valid_id(id)).collect();
        let _ = self.change(false, |doc| {
            for id in &ids {
                doc["open"].as_object_mut().unwrap().remove(id.as_str());
                doc["ended"].as_object_mut().unwrap().remove(id.as_str());
            }
        });
        let mut seen = self.seen.lock().unwrap();
        for id in ids {
            seen.remove(id);
        }
    }

    /// Unhide many at once (one sidebar change); `live` says which are known
    /// to be running (those come back unmarked).
    pub fn restore_many(&self, ids: &[String], live: impl Fn(&str) -> bool) {
        let at = now_ms();
        let _ = self.change(false, |doc| {
            for id in ids.iter().filter(|id| valid_id(id)) {
                if doc["open"].get(id).is_none() {
                    doc["open"][id] = json!(at);
                }
                if live(id) {
                    doc["ended"].as_object_mut().unwrap().remove(id);
                } else if doc["ended"].get(id).is_none() {
                    doc["ended"][id] = json!(at);
                }
            }
        });
    }

    /// `sessionOpen.snoozeCleanup {}`: the prompt returns only after
    /// [`SNOOZE_STEP`] more sessions end.
    pub fn snooze_cleanup(&self) -> Result<Value> {
        let (document, _) = self.change(true, |doc| {
            let ended = self.counted(doc).len() as u64;
            doc["cleanup"]["snoozedAt"] = json!(ended);
        })?;
        Ok(document)
    }

    /// The counted ended sessions whose mark is at least `older_than_ms` old
    /// (all of them when `None`), oldest mark first. Never Paused or live.
    pub fn cleanup_candidates(&self, older_than_ms: Option<i64>) -> Vec<String> {
        let document = self.document.lock().unwrap().clone();
        let cutoff = older_than_ms.map(|age| now_ms().saturating_sub(age.max(0)));
        let mut rows: Vec<(String, i64)> = self
            .counted(&document)
            .into_iter()
            .filter(|(_, at)| cutoff.is_none_or(|cutoff| *at <= cutoff))
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        rows.into_iter().map(|(id, _)| id).collect()
    }

    /// Unhide: back on the sidebar, marked ended unless known to be live.
    pub fn restore(&self, id: &str, live: bool) {
        if valid_id(id) {
            let _ = self.change(false, |doc| {
                let at = now_ms();
                if doc["open"].get(id).is_none() {
                    doc["open"][id] = json!(at);
                }
                if live {
                    doc["ended"].as_object_mut().unwrap().remove(id);
                } else if doc["ended"].get(id).is_none() {
                    doc["ended"][id] = json!(at);
                }
            });
        }
    }

    /// Fold one observed session row in. Returns whether the set changed.
    pub fn observe(&self, row: &Value, shells: &BTreeSet<String>) -> bool {
        let Some(id) = row_id(row) else {
            return false;
        };
        match classify(row, shells) {
            Observation::Open => {
                self.live_now.lock().unwrap().insert(id.into());
                {
                    let mut seen = self.seen.lock().unwrap();
                    if seen.len() >= MAX_SEEN && !seen.contains(id) {
                        seen.clear();
                    }
                    seen.insert(id.into());
                }
                if self.archived(id) {
                    return false;
                }
                self.change(false, |doc| {
                    if doc["open"].get(id).is_none() {
                        doc["open"][id] = json!(now_ms());
                    }
                    // A resume clears the ended mark.
                    doc["ended"].as_object_mut().unwrap().remove(id);
                })
                .is_ok_and(|(_, changed)| changed)
            }
            Observation::Ended => {
                self.live_now.lock().unwrap().remove(id);
                if self.closing.load(Ordering::Acquire) || !self.seen.lock().unwrap().contains(id) {
                    return false;
                }
                self.change(false, |doc| {
                    if doc["open"].get(id).is_some() && doc["ended"].get(id).is_none() {
                        doc["ended"][id] = json!(now_ms());
                    }
                })
                .is_ok_and(|(_, changed)| changed)
            }
            Observation::Neither => false,
        }
    }

    /// The one-time upgrade seed over the rows a hub currently knows:
    /// live rows join unmarked; rows stopped within the retired 24h window
    /// join marked ended unless native kept them. Persisted even when
    /// nothing was added, so it never runs again.
    pub fn seed(&self, rows: &[Value], shells: &BTreeSet<String>, now: time::OffsetDateTime) {
        if !self.seed_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        let at = now_ms();
        let mut seen = Vec::new();
        let result = self.change(false, |doc| {
            for row in rows {
                let Some(id) = row_id(row) else { continue };
                if self.archived(id) {
                    continue;
                }
                match classify(row, shells) {
                    Observation::Open => {
                        seen.push(id.to_owned());
                        if doc["open"].get(id).is_none() {
                            doc["open"][id] = json!(at);
                        }
                    }
                    Observation::Ended if super::roster::recently_ended(row, now) => {
                        if doc["open"].get(id).is_none() {
                            doc["open"][id] = json!(at);
                        }
                        if !self.legacy_kept.contains(id) && doc["ended"].get(id).is_none() {
                            doc["ended"][id] = json!(at);
                        }
                    }
                    _ => (),
                }
            }
        });
        self.live_now.lock().unwrap().extend(seen.iter().cloned());
        self.seen.lock().unwrap().extend(seen);
        // Make the seed one-time even when it changed nothing.
        if matches!(result, Ok((_, false)))
            && let Some(path) = &self.path
            && !path.exists()
            && let Err(error) = super::atomic_json(path, &self.get(), false)
        {
            eprintln!("session sidebar: seeded document could not be persisted: {error}");
        }
    }

    pub fn seed_pending(&self) -> bool {
        self.seed_pending.load(Ordering::Acquire)
    }

    /// Begin hub shutdown: no observation marks a session ended from now on.
    pub fn begin_close(&self) {
        self.closing.store(true, Ordering::Release);
    }

    /// Fence ended marks and stop the observer.
    pub async fn close(&self) {
        self.begin_close();
        self.stop.send_replace(true);
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// Start observing this hub's `agent.snapshot` stream. `rows` is the
    /// embedded engine's projection (empty when another process provides
    /// sessions), read for the upgrade seed.
    pub(crate) fn start(
        self: &Arc<Self>,
        hub: Handle,
        terminals: Option<Arc<super::terminals::Terminals>>,
        rows: Arc<RwLock<BTreeMap<String, Value>>>,
    ) {
        let open = self.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = open.run(hub, terminals, rows).await {
                eprintln!("session sidebar: observer stopped: {error}");
            }
        });
        *self.task.lock().unwrap() = Some(task);
    }

    async fn run(
        self: Arc<Self>,
        hub: Handle,
        terminals: Option<Arc<super::terminals::Terminals>>,
        raw: Arc<RwLock<BTreeMap<String, Value>>>,
    ) -> Result<()> {
        let mut stopping = self.stop.subscribe();
        if *stopping.borrow() {
            return Ok(());
        }
        tokio::select! {_=stopping.changed()=>return Ok(()),ready=hub.ready()=>{ready?;}}
        let client = crate::client::Client::connect_service(&hub).await?;
        let mut events = client.events();
        client
            .topics(BTreeSet::from(["agent.snapshot".into()]))
            .await?;
        let shells = || {
            terminals
                .as_ref()
                .map(|t| {
                    let mut ids = t.owned_shell_ids();
                    ids.extend(t.uncertain_shell_ids());
                    ids
                })
                .unwrap_or_default()
        };
        // Subscribed first, so nothing between the prime and the stream is lost.
        self.prime(&client, &raw, &shells()).await;
        let mut retry = tokio::time::interval(Duration::from_secs(5));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased;
                _ = stopping.changed() => { client.close(); return Ok(()); }
                event = events.recv() => match event {
                    Ok(event) => {
                        if event.topic == "agent.snapshot" && let Some(row) = event.data {
                            let open = self.clone();
                            let shells = shells();
                            tokio::task::spawn_blocking(move || open.observe(&row, &shells)).await?;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        self.prime(&client, &raw, &shells()).await;
                    }
                    Err(_) => { if self.closing.load(Ordering::Acquire) { return Ok(()); } bail!("session sidebar observer disconnected") }
                },
                // Another process may register `sessions.snapshots` after us.
                _ = retry.tick(), if self.seed_pending() => {
                    self.prime(&client, &raw, &shells()).await;
                }
            }
        }
    }

    /// Mark the current roster's live rows as seen (and on the sidebar) so a
    /// session already running when this hub started is marked when it ends,
    /// and run the upgrade seed once rows are available.
    async fn prime(
        self: &Arc<Self>,
        client: &crate::client::Client,
        raw: &Arc<RwLock<BTreeMap<String, Value>>>,
        shells: &BTreeSet<String>,
    ) {
        let local: Vec<Value> = raw.read().unwrap().values().cloned().collect();
        let listed = match tokio::time::timeout(
            Duration::from_secs(10),
            client.call("sessions.snapshots", json!({})),
        )
        .await
        {
            Ok(Ok(Value::Array(rows))) => Some(rows),
            _ => None,
        };
        if listed.is_none() && local.is_empty() {
            return;
        }
        let open = self.clone();
        let shells = shells.clone();
        let _ = tokio::task::spawn_blocking(move || {
            // The engine projection holds every retained row (the roster
            // would hide recently stopped ones); otherwise the provider's list.
            let rows = if local.is_empty() {
                listed.unwrap_or_default()
            } else {
                local
            };
            open.seed(&rows, &shells, time::OffsetDateTime::now_utc());
            for row in &rows {
                if classify(row, &shells) == Observation::Open {
                    open.observe(row, &shells);
                }
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: &str) -> Value {
        json!({"sessionId":id,"session_id":id,"mode":"input"})
    }
    fn stopped(id: &str) -> Value {
        json!({"sessionId":id,"session_id":id,"mode":"stopped","status":"ended"})
    }
    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn ended(open: &SessionOpen) -> BTreeSet<String> {
        open.get()["ended"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn an_end_seen_this_run_stays_on_the_sidebar_marked_ended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-open.json");
        let open = SessionOpen::open(Some(path.clone()), None, None, None);
        assert!(open.observe(&live("a"), &none()));
        assert!(!open.observe(&live("a"), &none()), "repeat is a no-op");
        assert_eq!(open.get()["version"], 1);
        // Persisted before the next run.
        assert!(
            SessionOpen::open(Some(path.clone()), None, None, None)
                .ids()
                .contains("a")
        );
        assert!(open.observe(&stopped("a"), &none()));
        assert!(open.ids().contains("a"), "nothing leaves on its own");
        assert_eq!(ended(&open), ["a".to_owned()].into());
        assert_eq!(open.kept(), json!({"sessions":[]}));
        // A resume clears the mark.
        assert!(open.observe(&live("a"), &none()));
        assert!(ended(&open).is_empty());
    }

    #[test]
    fn a_session_already_stopped_when_first_seen_stays_paused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-open.json");
        let first = SessionOpen::open(Some(path.clone()), None, None, None);
        first.observe(&live("a"), &none());
        // Next run: the app closing stopped "a" while no hub watched.
        let second = SessionOpen::open(Some(path), None, None, None);
        assert!(!second.observe(&stopped("a"), &none()));
        assert!(second.ids().contains("a"));
        assert!(ended(&second).is_empty());
        assert_eq!(second.kept()["sessions"][0]["sessionId"], "a");
    }

    #[test]
    fn shutdown_fences_ended_marks() {
        let open = SessionOpen::open(None, None, None, None);
        open.observe(&live("a"), &none());
        open.begin_close();
        assert!(!open.observe(&stopped("a"), &none()));
        assert!(open.ids().contains("a"));
        assert!(
            ended(&open).is_empty(),
            "a closing app's sessions stay Paused"
        );
    }

    #[test]
    fn starting_rows_open_but_shells_and_federated_rows_do_not() {
        let open = SessionOpen::open(None, None, None, None);
        assert!(open.observe(
            &json!({"session_id":"s","mode":"unknown","transport":"stream"}),
            &none()
        ));
        assert!(!open.observe(
            &json!({"session_id":"sh","mode":"unknown","transport":"stream"}),
            &["sh".to_owned()].into()
        ));
        assert!(!open.observe(
            &json!({"session_id":"pty","mode":"unknown","transport":"pty"}),
            &none()
        ));
        assert!(!open.observe(
            &json!({"sessionId":"r","mode":"input","hub":"peer"}),
            &none()
        ));
        assert_eq!(open.ids(), ["s".to_owned()].into());
    }

    #[test]
    fn hide_is_archive_and_restore_puts_it_back_marked_ended() {
        let archive = Arc::new(SessionArchive::open(None, None));
        let open = SessionOpen::open(None, None, Some(archive.clone()), None);
        open.observe(&live("a"), &none());
        open.observe(&stopped("a"), &none());
        archive
            .set(json!({"sessionId":"a","archived":true}))
            .unwrap();
        open.forget("a");
        assert!(!open.ids().contains("a"));
        assert!(ended(&open).is_empty());
        assert!(!open.observe(&live("a"), &none()), "hidden stays hidden");
        assert!(open.set(json!({"sessionId":"a","open":true})).is_err());
        archive
            .set(json!({"sessionId":"a","archived":false}))
            .unwrap();
        open.restore("a", false);
        assert!(open.ids().contains("a"));
        assert_eq!(ended(&open), ["a".to_owned()].into());
        open.restore("b", true);
        assert!(open.ids().contains("b") && !ended(&open).contains("b"));
    }

    #[test]
    fn explicit_set_is_idempotent_validated_and_unpin_clears_the_mark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-open.json");
        let open = SessionOpen::open(Some(path.clone()), None, None, None);
        for params in [
            Value::Null,
            json!({}),
            json!({"sessionId":"a"}),
            json!({"sessionId":"a","open":"yes"}),
            json!({"sessionId":"","open":true}),
            json!({"sessionId":"a\nb","open":true}),
            json!({"sessionId":"../x","open":true}),
            json!({"sessionId":"x".repeat(MAX_ID_BYTES + 1),"open":true}),
        ] {
            assert!(open.set(params).is_err());
        }
        assert!(!path.exists());
        let doc = open.set(json!({"sessionId":"a","open":true})).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(open.set(json!({"sessionId":"a","open":true})).unwrap(), doc);
        open.observe(&live("a"), &none());
        open.observe(&stopped("a"), &none());
        let doc = open.set(json!({"sessionId":"a","open":false})).unwrap();
        assert_eq!(doc["open"], json!({}));
        assert_eq!(doc["ended"], json!({}));
    }

    #[test]
    fn the_cap_evicts_ended_entries_first() {
        let mut document = normalize(Map::new(), Map::new());
        for i in 0..(MAX_OPEN + 2) {
            document["open"][format!("s{i}")] = json!(i as i64 + 100);
        }
        // The newest two are ended: they go before older unended ones.
        document["ended"][format!("s{}", MAX_OPEN + 1)] = json!(1);
        document["ended"][format!("s{MAX_OPEN}")] = json!(2);
        cap(&mut document);
        let open = document["open"].as_object().unwrap();
        assert_eq!(open.len(), MAX_OPEN);
        assert!(!open.contains_key(&format!("s{}", MAX_OPEN + 1)));
        assert!(!open.contains_key(&format!("s{MAX_OPEN}")));
        assert!(open.contains_key("s0"));
        document["open"]["extra"] = json!(1_000_000);
        cap(&mut document);
        assert!(!document["open"].as_object().unwrap().contains_key("s0"));
    }

    #[test]
    fn upgrade_seeds_kept_live_and_recent_rows_once() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("native-settings.json");
        std::fs::write(
            &legacy,
            json!({"kept_open": {"ws://h/bus": {"kept": 100, "kept-recent": 300}}}).to_string(),
        )
        .unwrap();
        let path = dir.path().join("session-open.json");
        let open = SessionOpen::open(Some(path.clone()), None, None, Some(legacy.clone()));
        assert!(open.seed_pending());
        let now = time::OffsetDateTime::parse(
            "2026-11-01T12:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        open.seed(
            &[
                live("running"),
                json!({"session_id":"recent","mode":"stopped","updated_at":"2026-11-01T06:00:00Z"}),
                json!({"session_id":"kept-recent","mode":"stopped","updated_at":"2026-11-01T06:00:00Z"}),
                json!({"session_id":"old","mode":"stopped","updated_at":"2026-10-01T00:00:00Z"}),
            ],
            &none(),
            now,
        );
        assert!(!open.seed_pending());
        assert_eq!(
            open.ids(),
            ["kept", "kept-recent", "recent", "running"]
                .map(str::to_owned)
                .into()
        );
        assert_eq!(ended(&open), ["recent".to_owned()].into());
        let kept: BTreeSet<_> = open.kept()["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["sessionId"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            kept,
            ["kept", "kept-recent", "running"].map(str::to_owned).into()
        );
        // The running session is "seen", so its end is marked.
        open.observe(&stopped("running"), &none());
        assert!(ended(&open).contains("running"));
        // Persisted: the legacy file is not read and nothing is re-seeded.
        let reopened = SessionOpen::open(Some(path.clone()), None, None, Some(legacy));
        assert!(!reopened.seed_pending());
        assert_eq!(reopened.ids(), open.ids());
    }

    #[test]
    fn an_empty_seed_still_runs_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-open.json");
        let open = SessionOpen::open(Some(path.clone()), None, None, None);
        open.seed(&[], &none(), time::OffsetDateTime::now_utc());
        assert!(path.exists());
        assert!(!SessionOpen::open(Some(path), None, None, None).seed_pending());
    }

    fn end(open: &SessionOpen, id: &str) {
        open.observe(&live(id), &none());
        open.observe(&stopped(id), &none());
    }

    #[test]
    fn cleanup_counts_only_ended_sidebar_sessions_and_suggests_at_the_threshold() {
        let archive = Arc::new(SessionArchive::open(None, None));
        let open = SessionOpen::open(None, None, Some(archive.clone()), None);
        let cleanup = |open: &SessionOpen| open.get()["cleanup"].clone();
        assert_eq!(
            cleanup(&open),
            json!({"ended":0,"threshold":DEFAULT_CLEANUP_THRESHOLD,"suggested":false,"snoozedAt":null})
        );
        for i in 0..19 {
            end(&open, &format!("e{i}"));
        }
        // Paused (seen stopped first) and live sessions never count.
        open.observe(&stopped("paused"), &none());
        open.set(json!({"sessionId":"paused","open":true})).unwrap();
        open.observe(&live("running"), &none());
        assert_eq!(cleanup(&open)["ended"], 19);
        assert_eq!(cleanup(&open)["suggested"], false);
        end(&open, "e19");
        assert_eq!(cleanup(&open)["ended"], 20);
        assert_eq!(cleanup(&open)["suggested"], true);
        // The change event carries the block too (the stored document).
        assert_eq!(open.document.lock().unwrap()["cleanup"]["suggested"], true);
        // An archived id is not counted even before its sidebar entry goes.
        archive
            .set(json!({"sessionId":"e0","archived":true}))
            .unwrap();
        assert_eq!(cleanup(&open)["ended"], 19);
        assert_eq!(cleanup(&open)["suggested"], false);
    }

    #[test]
    fn snooze_waits_for_ten_more_and_clears_under_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-open.json");
        let open = SessionOpen::open(Some(path.clone()), None, None, None);
        for i in 0..22 {
            end(&open, &format!("e{i}"));
        }
        let doc = open.snooze_cleanup().unwrap();
        assert_eq!(doc["cleanup"]["snoozedAt"], 22);
        assert_eq!(doc["cleanup"]["suggested"], false);
        // Persisted with the document.
        assert_eq!(
            SessionOpen::open(Some(path.clone()), None, None, None).get()["cleanup"]["snoozedAt"],
            22
        );
        for i in 22..31 {
            end(&open, &format!("e{i}"));
        }
        assert_eq!(open.get()["cleanup"]["suggested"], false, "31 < 22 + 10");
        end(&open, "e31");
        assert_eq!(open.get()["cleanup"]["suggested"], true);
        // Hiding back under the threshold clears the snooze.
        let ids: Vec<String> = (0..20).map(|i| format!("e{i}")).collect();
        open.forget_many(&ids);
        let doc = open.get();
        assert_eq!(doc["cleanup"]["ended"], 12);
        assert_eq!(doc["cleanup"]["snoozedAt"], Value::Null);
        assert_eq!(doc["cleanup"]["suggested"], false);
    }

    #[test]
    fn cleanup_candidates_respect_age_and_never_include_paused_or_live() {
        let open = SessionOpen::open(None, None, None, None);
        end(&open, "old");
        end(&open, "new");
        open.observe(&stopped("paused"), &none());
        open.set(json!({"sessionId":"paused","open":true})).unwrap();
        open.observe(&live("running"), &none());
        // Age the "old" mark by a day.
        open.document.lock().unwrap()["ended"]["old"] = json!(now_ms() - 86_400_000);
        assert_eq!(
            open.cleanup_candidates(None),
            vec!["old".to_owned(), "new".to_owned()]
        );
        assert_eq!(
            open.cleanup_candidates(Some(3_600_000)),
            vec!["old".to_owned()]
        );
        // A resumed session leaves the candidates.
        open.observe(&live("new"), &none());
        assert_eq!(open.cleanup_candidates(None), vec!["old".to_owned()]);
    }
}
