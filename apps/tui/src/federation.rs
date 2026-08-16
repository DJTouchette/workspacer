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
use crate::types::{Agent, AgentMode, Pending, Question, Usage};

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
    /// Fold one hub-stamped snapshot in. Returns `true` when this hub needs a
    /// (re)seed — it was unknown, or believed offline: a stamped event is proof
    /// of life, and the event stream alone can't backfill what was missed.
    pub fn upsert(&mut self, hub: &str, agent: Agent) -> bool {
        let state = self.hubs.entry(hub.to_string()).or_default();
        let needs_seed = state.offline || state.sessions.is_empty();
        state.offline = false;
        state.sessions.insert(agent.session_id.clone(), agent);
        needs_seed
    }

    /// Replace a hub's sessions wholesale from a `sessions.snapshots` seed
    /// (which is the authoritative roster — a session absent from it is gone).
    /// Marks the hub online.
    pub fn seed(&mut self, hub: &str, agents: Vec<Agent>) {
        let state = self.hubs.entry(hub.to_string()).or_default();
        state.offline = false;
        state.sessions = agents
            .into_iter()
            .map(|a| (a.session_id.clone(), a))
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
            .iter()
            .flat_map(|(_, state)| {
                state.sessions.values().map(move |a| {
                    let mut a = a.clone();
                    a.hub_offline = state.offline;
                    a
                })
            })
            .collect()
    }

    /// `(name, online, session count)` per known hub, for the dashboard.
    pub fn summary(&self) -> Vec<(String, bool, usize)> {
        self.hubs
            .iter()
            .map(|(name, s)| (name.clone(), !s.offline, s.sessions.len()))
            .collect()
    }
}

// ── snapshot row → Agent ────────────────────────────────────────────────────

/// Build a TUI [`Agent`] from one hub snapshot row (an `agent.snapshot` event
/// payload or a `sessions.snapshots` element), stamped with its hub.
///
/// Returns `None` for rows that must be skipped:
/// - `sparse:true` layout-ghosts (stopped-history stubs the desktop appends,
///   and every headless-brain compat row) — same rule as the desktop's ingest.
/// - Rows without a `sessionId` (nothing to key on).
pub fn agent_from_snapshot(hub: &str, row: &Value) -> Option<Agent> {
    if row.get("sparse").and_then(Value::as_bool) == Some(true) {
        return None;
    }
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
        context_limit: u.get("contextLimit").and_then(Value::as_u64).unwrap_or(0),
        cost_usd: u.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0),
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
        tool_calls: row
            .get("totalToolCalls")
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
/// app as a wholesale replacement. An empty (or all-sparse) roster is still
/// sent: the seed is authoritative, and stale rows must drop.
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
        assert_eq!(u.context_limit, 200);
        assert!((u.cost_usd - 1.25).abs() < f64::EPSILON);
        // A remote session can never have a local PTY, whatever it runs at home.
        assert!(a.is_stream());
    }

    #[test]
    fn sparse_rows_are_skipped() {
        // The desktop's stopped-history stubs — and every headless-brain compat
        // row — carry sparse:true and must never become fleet rows.
        let row = json!({
            "sessionId": "ghost", "cwd": "/x", "status": "ended",
            "ambientState": "idle", "sparse": true
        });
        assert!(agent_from_snapshot("work", &row).is_none());
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
}
