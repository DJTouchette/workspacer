//! Federated fleet state: remote sessions that live on peer hubs.
//!
//! The local hub republishes peer hubs' `agent.*` events with the peer's name
//! stamped on the event ENVELOPE (`hub: "<peer>"`; payload untouched, absent =
//! local), publishes `hub.peer.connected` / `hub.peer.disconnected` lifecycle
//! events, and routes qualified capability calls (`hub:<peer>/<method>`) over
//! the federation link. This module holds the client half of that:
//!
//! - [`RemoteFleet`]: the per-hub session store the app merges into its agent
//!   list. Fed by hub-stamped `agent.snapshot` events, seeded (and re-seeded on
//!   `hub.peer.connected`) via `federation.peers` + `hub:<peer>/sessions.snapshots`.
//! - [`agent_from_snapshot`]: the camelCase hub-snapshot row → [`Agent`]
//!   translation. The TUI's `Agent` is claudemon's snake_case REST shape; the
//!   hub serves the desktop's enriched camelCase view, so remote rows need this
//!   explicit mapping (`sessionId`/`ambientState`/`pendingApproval`/…).
//! - The async seeding tasks. Events alone can't fix restart-blindness — a TUI
//!   started after a peer's sessions went quiet would never see them — so the
//!   call plane seeds the view, exactly as the desktop main process does.
//!
//! Local sessions are deliberately untouched: they keep flowing from claudemon,
//! and UNSTAMPED bus events never reach this store — double-sourcing the local
//! fleet is the failure mode this file is written around.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppMsg;
use crate::bus::BusClient;
use crate::types::{Agent, AgentMode, CacheSplit, Pending, Question, Usage};

// ── the per-hub session store ───────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct HubState {
    /// Last-known sessions on this hub, keyed by session id. Kept (as
    /// tombstones) while the hub is offline; replaced wholesale on reseed.
    sessions: BTreeMap<String, Agent>,
    /// True between `hub.peer.disconnected` and the next sign of life.
    offline: bool,
}

/// Remote sessions across every peer hub. `BTreeMap`s keep iteration
/// deterministic, so merged rows don't jitter between polls.
#[derive(Debug, Default, Clone)]
pub struct RemoteFleet {
    hubs: BTreeMap<String, HubState>,
}

impl RemoteFleet {
    /// Fold one hub-stamped snapshot in (see [`fold_row`] — a sparse row
    /// overlays what we hold rather than clobbering it). Returns `true` when
    /// this hub needs a (re)seed — it was unknown, or believed offline: a
    /// stamped event is proof of life, and the event stream alone can't
    /// backfill what was missed.
    pub fn upsert(&mut self, hub: &str, agent: Agent) -> bool {
        let state = self.hubs.entry(hub.to_string()).or_default();
        let needs_seed = state.offline || state.sessions.is_empty();
        state.offline = false;
        let agent = match state.sessions.get(&agent.session_id) {
            Some(prev) => fold_row(prev, agent),
            None => agent,
        };
        state.sessions.insert(agent.session_id.clone(), agent);
        needs_seed
    }

    /// Adopt a hub's roster from a `sessions.snapshots` seed. The seed is the
    /// authoritative ROSTER — a session absent from it is gone — but each row
    /// folds over what we already hold ([`fold_row`]), so a brain-answered
    /// reseed refreshes state without stripping the enrichment an earlier rich
    /// desktop row carried. Marks the hub online.
    pub fn seed(&mut self, hub: &str, agents: Vec<Agent>) {
        let state = self.hubs.entry(hub.to_string()).or_default();
        state.offline = false;
        let prev = std::mem::take(&mut state.sessions);
        state.sessions = agents
            .into_iter()
            .map(|a| {
                let a = match prev.get(&a.session_id) {
                    Some(p) => fold_row(p, a),
                    None => a,
                };
                (a.session_id.clone(), a)
            })
            .collect();
    }

    /// `hub.peer.disconnected`: keep the sessions, flag them as tombstones.
    pub fn set_offline(&mut self, hub: &str) {
        self.hubs.entry(hub.to_string()).or_default().offline = true;
    }

    /// `hub.peer.connected`: back online. Sessions stay as-is until the reseed
    /// (triggered by the caller) replaces them — a gap with an empty sidebar
    /// would read as the fleet dying on reconnect.
    pub fn set_online(&mut self, hub: &str) {
        self.hubs.entry(hub.to_string()).or_default().offline = false;
    }

    /// Every remote session, hub-tagged and with the tombstone flag stamped
    /// from its hub's connectivity. The app merges this after the local list.
    pub fn agents(&self) -> Vec<Agent> {
        self.hubs
            .values()
            .flat_map(|state| {
                state.sessions.values().map(move |a| {
                    let mut a = a.clone();
                    a.hub_offline = state.offline;
                    a
                })
            })
            .collect()
    }

    /// `(name, online, LIVE session count)` per known hub, for the dashboard.
    /// Stopped rows don't count: a brain re-serves claudemon's whole resumable
    /// history as sparse ended rows, and "work ● 100" would read as a hundred
    /// agents running on the peer.
    pub fn summary(&self) -> Vec<(String, bool, usize)> {
        self.hubs
            .iter()
            .map(|(name, s)| {
                let live = s.sessions.values().filter(|a| !a.is_stopped()).count();
                (name.clone(), !s.offline, live)
            })
            .collect()
    }
}

/// Fold an incoming row over the one already held for the same session — the
/// /m PWA's `Object.assign({}, prev, snap)` fold, translated onto the mapped
/// [`Agent`]:
///
/// - A RICH row replaces wholesale: it is the enriched superset, exactly the
///   pre-federation-polish behaviour.
/// - A SPARSE row carries live state — `mode` and `pending` are taken
///   verbatim (both sparse producers set explicit nulls precisely so a stale
///   approval/question clears on the client) — but only part of the
///   identity/enrichment fields, which keep their known values where the
///   sparse row has none. `tool_calls` never regresses: the counter is
///   monotonic, and a desktop layout-ghost carries none at all.
///
/// The folded row keeps `prev`'s sparseness: state folded onto a rich base is
/// still a rich row, so later sparse rows keep overlaying it instead of
/// inheriting a false "sparse" mark.
fn fold_row(prev: &Agent, mut next: Agent) -> Agent {
    if !next.sparse {
        return next;
    }
    next.sparse = prev.sparse;
    // A session's cwd and provider never change; a sparse row missing them
    // (the desktop's ghosts carry no provider, so mapping defaulted it) must
    // not overwrite what a richer row already established.
    if next.cwd.is_none() {
        next.cwd = prev.cwd.clone();
    }
    next.provider = prev.provider.clone();
    next.label = next.label.or_else(|| prev.label.clone());
    next.usage = next.usage.or_else(|| prev.usage.clone());
    next.last_event = next.last_event.or_else(|| prev.last_event.clone());
    next.tool_calls = next.tool_calls.max(prev.tool_calls);
    next
}

// ── snapshot row → Agent ────────────────────────────────────────────────────

/// Build a TUI [`Agent`] from one hub snapshot row (an `agent.snapshot` event
/// payload or a `sessions.snapshots` element), stamped with its hub.
///
/// Two producers publish rows, and both map here:
/// - Rich desktop snapshots (camelCase, enriched).
/// - `sparse:true` state-only rows: the headless brain's compat overlay
///   (claudemon's snake_case row with `sessionId` / `status` / `ambientState`
///   / camelCase `usage` / `pendingApproval` / `pendingQuestions` / `label`
///   layered on — see `cmd/brain/enrich.go`), and the desktop's stopped
///   layout-ghosts. A brain row carries everything the fleet view needs
///   (status, cwd, provider, pending work, usage), so a `workspacer serve`
///   peer contributes real rows — /m always accepted these; skipping them
///   here made a brain-only peer invisible. The `sparse` mark makes
///   [`fold_row`] overlay them onto a richer row instead of clobbering it.
///
/// Returns `None` only for rows without a `sessionId` (nothing to key on).
pub fn agent_from_snapshot(hub: &str, row: &Value) -> Option<Agent> {
    let sparse = row.get("sparse").and_then(Value::as_bool) == Some(true);
    let session_id = row.get("sessionId")?.as_str()?.to_string();

    let str_of = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };

    // Pending work: the question list deserializes verbatim (the snapshot's
    // `pendingQuestions` spells `multi_select`/`options`/`label` exactly like
    // claudemon does); the approval is reshaped from {toolName, toolInput}.
    let questions: Option<Vec<Question>> = row
        .get("pendingQuestions")
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .filter(|q: &Vec<Question>| !q.is_empty());
    let approval = row
        .get("pendingApproval")
        .filter(|v| !v.is_null())
        .map(|v| Pending::Approval {
            tool: v.get("toolName").and_then(Value::as_str).map(String::from),
            raw: v.get("toolInput").cloned().unwrap_or(Value::Null),
        });
    let pending = match (&questions, approval) {
        (Some(qs), _) => Some(Pending::Question {
            questions: qs.clone(),
        }),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    };

    let mode = remote_mode(row, pending.as_ref());

    let usage = row.get("usage").filter(|v| !v.is_null()).map(|u| Usage {
        model: u.get("model").and_then(Value::as_str).map(String::from),
        context_tokens: u.get("contextTokens").and_then(Value::as_u64).unwrap_or(0),
        // Absent (or the 0 an older hub sends for "unknown") stays absent: a
        // peer that could not report a window must not arrive claiming one.
        context_limit: u
            .get("contextLimit")
            .and_then(Value::as_u64)
            .filter(|l| *l > 0),
        cost_usd: u.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0),
        // The hub passes the prompt-cache split through under its own key with
        // its sub-keys unchanged, so it maps straight across. Absent stays
        // absent: a remote peer that reported no cache data must not arrive
        // looking like one whose cache never hit.
        cache: u.get("cache").filter(|v| !v.is_null()).map(|c| CacheSplit {
            fresh: c.get("fresh").and_then(Value::as_u64).unwrap_or(0),
            write: c.get("write").and_then(Value::as_u64).unwrap_or(0),
            read: c.get("read").and_then(Value::as_u64).unwrap_or(0),
        }),
    });

    Some(Agent {
        session_id,
        cwd: str_of("cwd"),
        provider: str_of("provider").unwrap_or_else(|| "claude".to_string()),
        // No local PTY can exist for a remote session whatever its transport
        // is at home, and "stream" is exactly the TUI's transcript-only path.
        transport: "stream".to_string(),
        mode,
        pending,
        // Rich rows spell it `totalToolCalls`; a sparse brain row keeps
        // claudemon's own `tool_calls` underneath its compat overlay.
        tool_calls: row
            .get("totalToolCalls")
            .or_else(|| row.get("tool_calls"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        last_event: None,
        // A peer-local path is meaningless here (and the runs overlay reads
        // the local disk off it) — never carry it across the seam.
        transcript_path: None,
        usage,
        hub: Some(hub.to_string()),
        hub_offline: false,
        // The harness spells the label `name`; the real snapshot says `label`.
        label: str_of("label").or_else(|| str_of("name")),
        sparse,
    })
}

/// Resolve a hub snapshot's `status` + `ambientState` (+ pending work) into
/// the TUI's [`AgentMode`] vocabulary. Pending work wins over the ambient
/// state — an approval card must render whatever the ambient string says.
fn remote_mode(row: &Value, pending: Option<&Pending>) -> AgentMode {
    if row.get("status").and_then(Value::as_str) == Some("ended") {
        return AgentMode::Stopped;
    }
    match pending {
        Some(Pending::Question { .. }) => return AgentMode::Question,
        Some(Pending::Approval { .. }) => return AgentMode::Approval,
        None => {}
    }
    match row
        .get("ambientState")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        // "working" is the harness's spelling; "background" is a busy subagent.
        "thinking" | "streaming" | "working" | "background" => AgentMode::Responding,
        "waiting_approval" => AgentMode::Approval,
        // The desktop folds "at the prompt" into idle/waiting_input; both are
        // the TUI's `input` (a session awaiting its next message counts as
        // needing you, exactly like a local one).
        "waiting_input" | "idle" => AgentMode::Input,
        "" => AgentMode::Unknown,
        _ => AgentMode::Other,
    }
}

// ── async seeding (fire-and-forget; results arrive as AppMsg) ───────────────

/// Seed the whole federated view: ask the local hub for its peers, then pull
/// each connected peer's roster. Quiet on every failure — a hub without
/// federation (or none at all) answers with an error and the TUI behaves
/// exactly as before.
pub async fn seed_remote_fleet(bus: BusClient, tx: UnboundedSender<AppMsg>) {
    let Ok(peers) = bus.call("federation.peers", json!({})).await else {
        return;
    };
    let Some(peers) = peers.as_array() else {
        return;
    };
    for peer in peers {
        let Some(name) = peer.get("name").and_then(Value::as_str) else {
            continue;
        };
        if peer.get("connected").and_then(Value::as_bool) != Some(true) {
            let _ = tx.send(AppMsg::HubDown {
                hub: name.to_string(),
            });
            continue;
        }
        seed_peer(&bus, name, &tx).await;
    }
}

/// Pull one peer's roster (`hub:<peer>/sessions.snapshots`) and hand it to the
/// app as the authoritative roster (sparse rows included — a brain-only peer
/// serves nothing else). An empty roster is still sent: stale rows must drop.
pub async fn seed_peer(bus: &BusClient, hub: &str, tx: &UnboundedSender<AppMsg>) {
    let Ok(rows) = bus
        .call(&format!("hub:{hub}/sessions.snapshots"), json!({}))
        .await
    else {
        return;
    };
    let agents = rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| agent_from_snapshot(hub, r))
                .collect()
        })
        .unwrap_or_default();
    let _ = tx.send(AppMsg::RemoteSeed {
        hub: hub.to_string(),
        agents,
    });
}

/// Fetch a remote session's conversation over the federation link
/// (`hub:<peer>/sessions.conversation` → `{items, seq}` — the same shape the
/// local fold adopts), and deliver it as an ordinary transcript snapshot.
pub async fn fetch_remote_conversation(
    bus: BusClient,
    hub: String,
    session_id: String,
    tx: UnboundedSender<AppMsg>,
) {
    let Ok(v) = bus
        .call(
            &format!("hub:{hub}/sessions.conversation"),
            json!({ "sessionId": session_id }),
        )
        .await
    else {
        return;
    };
    let _ = tx.send(AppMsg::Transcript {
        session_id,
        snapshot: Box::new(v),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich_row() -> Value {
        json!({
            "sessionId": "r1",
            "cwd": "/peer/proj",
            "provider": "claude",
            "transport": "pty",
            "status": "active",
            "ambientState": "streaming",
            "totalToolCalls": 7,
            "label": "api refactor",
            "usage": { "model": "claude-opus-4-8", "contextTokens": 50, "contextLimit": 200, "costUSD": 1.25 },
            "pendingApproval": null,
            "pendingQuestions": null
        })
    }

    #[test]
    fn maps_a_rich_snapshot_row() {
        let a = agent_from_snapshot("work", &rich_row()).expect("mapped");
        assert_eq!(a.session_id, "r1");
        assert_eq!(a.hub.as_deref(), Some("work"));
        assert_eq!(a.cwd_str(), "/peer/proj");
        assert_eq!(a.mode, AgentMode::Responding);
        assert!(a.is_busy());
        assert_eq!(a.tool_calls, 7);
        assert_eq!(a.label.as_deref(), Some("api refactor"));
        let u = a.usage.clone().expect("usage");
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(u.context_tokens, 50);
        assert_eq!(u.context_limit, Some(200));
        assert!((u.cost_usd - 1.25).abs() < f64::EPSILON);
        // A remote session can never have a local PTY, whatever it runs at home.
        assert!(a.is_stream());
    }

    /// A headless brain row as `cmd/brain/enrich.go` actually builds it:
    /// claudemon's snake_case session underneath, camelCase compat on top.
    fn sparse_brain_row() -> Value {
        json!({
            "session_id": "b1", "cwd": "/srv/api", "provider": "codex",
            "transport": "pty", "mode": "responding", "tool_calls": 4,
            "updated_at": "2026-08-16T12:00:00Z",
            "sessionId": "b1", "sparse": true, "status": "active",
            "ambientState": "streaming", "lastActivity": 1755345600000u64,
            "pendingApproval": null, "pendingQuestions": null,
            "usage": { "model": "gpt-5.3-codex", "contextTokens": 10, "contextLimit": 400, "costUSD": 0.5 }
        })
    }

    #[test]
    fn maps_a_sparse_brain_row() {
        // A `workspacer serve` peer publishes ONLY these; skipping them made a
        // brain-only peer invisible while /m showed its agents fine.
        let a = agent_from_snapshot("work", &sparse_brain_row()).expect("mapped");
        assert!(a.sparse, "the mark drives the fold semantics");
        assert_eq!(a.session_id, "b1");
        assert_eq!(a.hub.as_deref(), Some("work"));
        assert_eq!(a.cwd_str(), "/srv/api");
        assert_eq!(a.provider, "codex");
        assert_eq!(a.mode, AgentMode::Responding);
        assert_eq!(a.tool_calls, 4, "claudemon's snake `tool_calls` counts");
        assert_eq!(a.label, None, "no enrichment — the UI derives from cwd");
        let u = a.usage.as_ref().expect("usage");
        assert_eq!(u.model.as_deref(), Some("gpt-5.3-codex"));
        assert!(a.is_stream(), "no local PTY exists whatever runs at home");
    }

    #[test]
    fn sparse_pending_work_maps_like_rich_pending_work() {
        // The brain reshapes claudemon's pending into the same
        // pendingApproval/pendingQuestions the desktop publishes, so needs-you,
        // y/n/a, and the question stepper work unchanged.
        let mut row = sparse_brain_row();
        row["ambientState"] = json!("waiting_approval");
        row["pendingApproval"] =
            json!({ "toolName": "Bash", "toolInput": { "command": "make test" } });
        let a = agent_from_snapshot("work", &row).expect("mapped");
        assert_eq!(a.mode, AgentMode::Approval);
        assert!(a.needs_you());
        let (tool, raw) = a.approval().expect("approval pending");
        assert_eq!(tool, "Bash");
        assert_eq!(raw["command"], json!("make test"));
    }

    #[test]
    fn sparse_layout_ghosts_map_to_stopped_rows() {
        // The desktop's stopped-history stubs are sparse too (status ended).
        // They map — mirroring /m — and the app's seen-live orphan rule decides
        // whether history is shown, exactly as for local stopped sessions.
        let row = json!({
            "sessionId": "ghost", "cwd": "/x", "label": "old run",
            "status": "ended", "ambientState": "idle",
            "pendingApproval": null, "pendingQuestions": null, "sparse": true
        });
        let a = agent_from_snapshot("work", &row).expect("mapped");
        assert!(a.sparse);
        assert!(a.is_stopped());
        assert_eq!(a.label.as_deref(), Some("old run"));
    }

    #[test]
    fn rows_without_a_session_id_are_skipped() {
        assert!(agent_from_snapshot("work", &json!({ "cwd": "/x" })).is_none());
        assert!(agent_from_snapshot("work", &json!("not-an-object")).is_none());
    }

    #[test]
    fn maps_the_harness_shape() {
        // services/hub/scripts/federation-harness.sh publishes `name` (not
        // `label`), ambientState "working", and no status/usage.
        let row = json!({
            "sessionId": "fed-demo-2", "cwd": "/tmp/web", "provider": "codex",
            "ambientState": "waiting_approval", "name": "web tests",
            "pendingApproval": { "toolName": "Bash", "toolInput": { "command": "npm test" }, "timestamp": 1 }
        });
        let a = agent_from_snapshot("work", &row).expect("mapped");
        assert_eq!(a.mode, AgentMode::Approval);
        assert_eq!(a.label.as_deref(), Some("web tests"));
        let (tool, raw) = a.approval().expect("approval pending");
        assert_eq!(tool, "Bash");
        assert_eq!(raw["command"], json!("npm test"));
    }

    #[test]
    fn questions_deserialize_verbatim_and_win_the_mode() {
        let row = json!({
            "sessionId": "q1", "ambientState": "idle",
            "pendingQuestions": [
                { "question": "Which?", "multi_select": true,
                  "options": [{ "label": "A" }, { "label": "B", "description": "b" }] }
            ]
        });
        let a = agent_from_snapshot("work", &row).expect("mapped");
        assert_eq!(a.mode, AgentMode::Question);
        let qs = a.questions().expect("questions");
        assert_eq!(qs.len(), 1);
        assert!(qs[0].multi_select);
        assert_eq!(qs[0].options[1].label, "B");
    }

    #[test]
    fn status_ended_beats_everything() {
        let mut row = rich_row();
        row["status"] = json!("ended");
        let a = agent_from_snapshot("work", &row).expect("mapped");
        assert!(a.is_stopped());
    }

    #[test]
    fn ambient_vocabulary_maps_to_agent_modes() {
        let mode = |ambient: &str| {
            let row = json!({ "sessionId": "s", "ambientState": ambient });
            agent_from_snapshot("h", &row).unwrap().mode
        };
        assert_eq!(mode("thinking"), AgentMode::Responding);
        assert_eq!(mode("streaming"), AgentMode::Responding);
        assert_eq!(mode("working"), AgentMode::Responding);
        assert_eq!(mode("background"), AgentMode::Responding);
        assert_eq!(mode("waiting_approval"), AgentMode::Approval);
        assert_eq!(mode("waiting_input"), AgentMode::Input);
        assert_eq!(mode("idle"), AgentMode::Input);
        assert_eq!(mode("someday-new-state"), AgentMode::Other);
    }

    // ── fleet merge + tombstones ────────────────────────────────────────────

    fn remote(hub: &str, sid: &str) -> Agent {
        agent_from_snapshot(
            hub,
            &json!({ "sessionId": sid, "cwd": format!("/peer/{sid}"), "ambientState": "streaming" }),
        )
        .unwrap()
    }

    #[test]
    fn upsert_reports_when_a_hub_needs_seeding() {
        let mut fleet = RemoteFleet::default();
        // First sign of life from an unknown hub → seed it.
        assert!(fleet.upsert("work", remote("work", "s1")));
        // Steady state → no reseed churn.
        assert!(!fleet.upsert("work", remote("work", "s2")));
        // A stamped event from an offline hub is proof of life → reseed.
        fleet.set_offline("work");
        assert!(fleet.upsert("work", remote("work", "s1")));
        assert!(!fleet.upsert("work", remote("work", "s1")));
    }

    #[test]
    fn disconnect_keeps_sessions_as_tombstones_and_reconnect_reseeds() {
        let mut fleet = RemoteFleet::default();
        fleet.seed("work", vec![remote("work", "s1"), remote("work", "s2")]);
        assert_eq!(fleet.agents().len(), 2);
        assert!(fleet.agents().iter().all(|a| !a.hub_offline));

        // Down: nothing vanishes; every row is flagged offline.
        fleet.set_offline("work");
        let tombstones = fleet.agents();
        assert_eq!(tombstones.len(), 2, "sessions must not silently vanish");
        assert!(tombstones.iter().all(|a| a.hub_offline));
        assert!(
            tombstones.iter().all(|a| !a.needs_you()),
            "tombstones can't be acted on, so they must not count as needing you"
        );

        // Back up: rows go live again even before the reseed lands…
        fleet.set_online("work");
        assert!(fleet.agents().iter().all(|a| !a.hub_offline));
        // …and the reseed is authoritative (s2 ended while the hub was away).
        fleet.seed("work", vec![remote("work", "s1")]);
        let live: Vec<String> = fleet
            .agents()
            .iter()
            .map(|a| a.session_id.clone())
            .collect();
        assert_eq!(live, vec!["s1"]);
    }

    #[test]
    fn hubs_merge_deterministically_and_summarize() {
        let mut fleet = RemoteFleet::default();
        fleet.seed("zeta", vec![remote("zeta", "z1")]);
        fleet.seed("alpha", vec![remote("alpha", "a2"), remote("alpha", "a1")]);
        fleet.set_offline("zeta");

        let ids: Vec<(Option<String>, String)> = fleet
            .agents()
            .iter()
            .map(|a| (a.hub.clone(), a.session_id.clone()))
            .collect();
        assert_eq!(
            ids,
            vec![
                (Some("alpha".into()), "a1".into()),
                (Some("alpha".into()), "a2".into()),
                (Some("zeta".into()), "z1".into()),
            ],
            "iteration order is deterministic (hub, then session id)"
        );
        assert_eq!(
            fleet.summary(),
            vec![
                ("alpha".to_string(), true, 2),
                ("zeta".to_string(), false, 1)
            ]
        );
    }

    // ── the sparse fold (mirrors /m's Object.assign(prev, snap)) ────────────

    #[test]
    fn sparse_rows_overlay_rich_ones_without_clobbering_enrichment() {
        // A desktop peer's hub carries BOTH producers: rich desktop snapshots
        // and the brain's sparse compat rows, interleaved for the SAME session.
        let mut fleet = RemoteFleet::default();
        let mut rich = rich_row();
        rich["provider"] = json!("codex");
        fleet.upsert("work", agent_from_snapshot("work", &rich).unwrap());

        // Brain row: fresher state (now waiting for input, approval cleared by
        // explicit nulls), but no label, no usage, no provider, fewer counters.
        let sparse = json!({
            "sessionId": "r1", "sparse": true, "status": "active",
            "ambientState": "waiting_input", "tool_calls": 2,
            "pendingApproval": null, "pendingQuestions": null
        });
        fleet.upsert("work", agent_from_snapshot("work", &sparse).unwrap());

        let a = &fleet.agents()[0];
        assert_eq!(
            a.mode,
            AgentMode::Input,
            "live state comes from the sparse row"
        );
        assert!(a.pending.is_none());
        assert_eq!(
            a.label.as_deref(),
            Some("api refactor"),
            "rich label survives"
        );
        assert_eq!(
            a.provider, "codex",
            "provider is immutable; the mapping default must not win"
        );
        assert_eq!(a.cwd_str(), "/peer/proj");
        assert_eq!(
            a.usage.as_ref().and_then(|u| u.model.as_deref()),
            Some("claude-opus-4-8"),
            "a sparse row without usage keeps the rich one"
        );
        assert_eq!(a.tool_calls, 7, "the monotonic counter never regresses");
        assert!(!a.sparse, "folded onto a rich base, the row is still rich");
    }

    #[test]
    fn sparse_overlay_clears_stale_pending_decisions() {
        // The brain sets pendingApproval/pendingQuestions explicitly (null when
        // absent) precisely so a decision made elsewhere clears here.
        let mut fleet = RemoteFleet::default();
        let mut rich = rich_row();
        rich["ambientState"] = json!("waiting_approval");
        rich["pendingApproval"] =
            json!({ "toolName": "Bash", "toolInput": { "command": "rm -rf x" } });
        fleet.upsert("work", agent_from_snapshot("work", &rich).unwrap());
        assert_eq!(fleet.agents()[0].mode, AgentMode::Approval);

        let sparse = json!({
            "sessionId": "r1", "sparse": true, "status": "active",
            "ambientState": "streaming",
            "pendingApproval": null, "pendingQuestions": null
        });
        fleet.upsert("work", agent_from_snapshot("work", &sparse).unwrap());
        let a = &fleet.agents()[0];
        assert!(a.pending.is_none(), "the approval was decided at home");
        assert_eq!(a.mode, AgentMode::Responding);
    }

    #[test]
    fn rich_rows_replace_sparse_ones_wholesale() {
        // The desktop coming up on a peer upgrades its sessions in place.
        let mut fleet = RemoteFleet::default();
        let mut sparse = sparse_brain_row();
        sparse["sessionId"] = json!("r1");
        fleet.upsert("work", agent_from_snapshot("work", &sparse).unwrap());
        assert!(fleet.agents()[0].sparse);

        fleet.upsert("work", agent_from_snapshot("work", &rich_row()).unwrap());
        let a = &fleet.agents()[0];
        assert!(!a.sparse);
        assert_eq!(a.label.as_deref(), Some("api refactor"));
        assert_eq!(a.tool_calls, 7);
    }

    #[test]
    fn a_sparse_only_seed_populates_the_fleet_and_reseeds_drop_gone_sessions() {
        // The whole point: a brain-only peer's roster is ALL sparse rows, and
        // seeding/reseeding must treat it as the authoritative roster.
        let sparse = |sid: &str, ambient: &str| {
            let mut row = sparse_brain_row();
            row["sessionId"] = json!(sid);
            row["ambientState"] = json!(ambient);
            agent_from_snapshot("brainy", &row).unwrap()
        };
        let mut fleet = RemoteFleet::default();
        fleet.seed(
            "brainy",
            vec![sparse("s1", "streaming"), sparse("s2", "idle")],
        );
        let agents = fleet.agents();
        assert_eq!(agents.len(), 2);
        assert!(agents
            .iter()
            .all(|a| a.sparse && a.hub.as_deref() == Some("brainy")));

        // Tombstones work for sparse rows like any other…
        fleet.set_offline("brainy");
        assert!(fleet.agents().iter().all(|a| a.hub_offline));

        // …and the reseed on reconnect is authoritative: s2 is gone.
        fleet.set_online("brainy");
        fleet.seed("brainy", vec![sparse("s1", "waiting_input")]);
        let agents = fleet.agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].session_id, "s1");
        assert_eq!(agents[0].mode, AgentMode::Input);
        assert!(!agents[0].hub_offline);
    }

    #[test]
    fn reseeds_fold_sparse_rows_over_rich_ones() {
        // When the brain answers the reseed on a desktop peer's hub, the fold
        // applies there too: state (and present usage) refresh, labels survive.
        let mut fleet = RemoteFleet::default();
        fleet.seed(
            "work",
            vec![agent_from_snapshot("work", &rich_row()).unwrap()],
        );
        let mut sparse = sparse_brain_row();
        sparse["sessionId"] = json!("r1");
        fleet.seed("work", vec![agent_from_snapshot("work", &sparse).unwrap()]);

        let a = &fleet.agents()[0];
        assert_eq!(
            a.label.as_deref(),
            Some("api refactor"),
            "reseed must not regress rich data"
        );
        assert!(!a.sparse);
        assert_eq!(
            a.usage.as_ref().and_then(|u| u.model.as_deref()),
            Some("gpt-5.3-codex"),
            "usage carried by the sparse row is fresher and wins"
        );
    }

    #[test]
    fn summary_counts_only_live_sessions() {
        // A brain re-serves claudemon's resumable stopped history; the
        // dashboard's per-hub count must not read as agents running.
        let ghost = agent_from_snapshot(
            "work",
            &json!({ "sessionId": "old", "status": "ended", "ambientState": "idle", "sparse": true }),
        )
        .unwrap();
        let mut fleet = RemoteFleet::default();
        fleet.seed("work", vec![remote("work", "s1"), ghost]);
        assert_eq!(fleet.summary(), vec![("work".to_string(), true, 1)]);
    }
}
