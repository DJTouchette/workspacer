//! Remote worker nodes: machines that can be OFF ON PURPOSE.
//!
//! A "node" is `claudemon` + `brain --hub …` running somewhere else (today, a
//! Fly machine). The hub owns the registry and the state machine; this module
//! owns everything the TUI needs to read one honestly, and the two async tasks
//! that talk to the bus. The wire contract is
//! `.workspacer/reports/2026-08-24-fly-wake-contract.md`, and this is the Rust
//! twin of the renderer's `apps/desktop/src/renderer/src/lib/remoteNodes.ts` —
//! same states, same words, same refusals.
//!
//! Three things here are load-bearing and none is obvious:
//!
//!  1. **A permission check is NOT a feature check.** `nodes.list` sits in the
//!     bus's VIEW tier, so every token may call it — but the hub only
//!     REGISTERS the method when a `nodes.json` exists, which is to say never
//!     on an ordinary install. `no provider for nodes.list` therefore means
//!     "this hub has no remote nodes", not "something broke"; see
//!     [`is_registry_absent`]. Any OTHER error is a real failure and must not
//!     be folded into it, or a broken hub renders as a hub with no nodes.
//!
//!  2. **`waking` is not `unreachable`, and neither is `stopping`.** A machine
//!     takes real seconds to boot and real seconds to drain, and a state that
//!     reads the same as a hang is what makes someone give up. The five states
//!     get five presentations and the two transitional ones are the only ones
//!     that read as progress.
//!
//!  3. **A wake spends real money, and this client cannot spend it back.** The
//!     hub has a stop verb now (`nodes.sleep`) but the TUI does not offer it,
//!     so [`WAKE_COST_NOTE`] says who can. It is printed beside the action
//!     rather than hidden, the action is never offered where it would be
//!     refused ([`wake_affordance`]) — including on a machine already
//!     shutting down, where a wake would silently CANCEL somebody's stop — and
//!     no single keypress starts one. See `App::request_wake` /
//!     `App::confirm_wake`.
//!
//! Mirrors [`crate::federation`] in shape: a small store the app folds events
//! into, a wire→model adapter, and free async task bodies that post [`AppMsg`]s
//! back to the loop. Seed from the call plane on every (re)connect, patch from
//! `node.state_changed`, never poll.

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppMsg;
use crate::bus::BusClient;

// ── the state machine ───────────────────────────────────────────────────────

/// The five node states. The distinction between the last four is the whole
/// point of the feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    /// Its provider is on the bus and answering.
    Available,
    /// The hub asked the cloud API to start it and is waiting for the provider.
    Waking,
    /// The hub asked the machine to shut down and is waiting for it to drain.
    /// The sleep path's mirror of [`NodeState::Waking`]: rendering a shutdown
    /// somebody deliberately asked for as a failure turns an act into an alarm.
    Stopping,
    /// Switched off, deliberately, and wakeable.
    Stopped,
    /// The hub does not know how to get a working node out of this one.
    Unreachable,
}

impl NodeState {
    /// Read the wire string. An unknown state is [`NodeState::Unreachable`]
    /// rather than rendered raw: a state this client cannot presume to
    /// understand is, by definition, one it does not know how to act on.
    pub fn from_wire(s: &str) -> NodeState {
        match s {
            "available" => NodeState::Available,
            "waking" => NodeState::Waking,
            "stopping" => NodeState::Stopping,
            "stopped" => NodeState::Stopped,
            _ => NodeState::Unreachable,
        }
    }

    /// The state in a word, for the row's right-hand chip.
    pub fn label(self) -> &'static str {
        match self {
            NodeState::Available => "connected",
            NodeState::Waking => "starting…",
            NodeState::Stopping => "shutting down…",
            NodeState::Stopped => "asleep",
            NodeState::Unreachable => "can't reach",
        }
    }

    /// The dot in front of the row. `waking` shares the accent (working) mark
    /// a thinking agent uses — a booting machine that paints like a failure is
    /// the whole bug this feature exists to remove — and `stopping` gets the
    /// mirrored half for the same reason, filling the other way.
    pub fn marker(self) -> &'static str {
        match self {
            NodeState::Available => "●",
            NodeState::Waking => "◐",
            NodeState::Stopping => "◑",
            NodeState::Stopped => "○",
            NodeState::Unreachable => "▲",
        }
    }

    /// The line under the chip when the hub sent no `detail` of its own.
    pub fn fallback_detail(self) -> &'static str {
        match self {
            NodeState::Available => "this machine is on the bus and answering.",
            NodeState::Waking => "the machine is booting — usually ready in about 20 seconds.",
            NodeState::Stopping => {
                "the machine is shutting down cleanly. it stops billing once it is off."
            }
            NodeState::Stopped => "switched off, and nothing is billing. waking will start it.",
            NodeState::Unreachable => "the hub can't get a working machine out of this one.",
        }
    }
}

/// The node's own exit record, read off its volume via `brain.info`. ABSENT
/// means nobody knows — never that it ended cleanly. The hub does not
/// fabricate an empty one, and neither does this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeLastExit {
    /// `signal-TERM` / `signal-INT` = a deliberate stop. Anything else = a crash.
    pub reason: Option<String>,
    pub exit_code: Option<i64>,
    /// RFC3339, on the NODE's clock. Display only — never compute with it.
    pub at: Option<String>,
}

/// `nodes.NodeView` — the one payload `nodes.list`, `nodes.wake` and
/// `node.state_changed` all carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    pub id: String,
    pub label: String,
    pub state: NodeState,
    /// One human sentence, written to be read. Empty is normal for `available`.
    pub detail: Option<String>,
    pub wakeable: bool,
    /// Consecutive failed wakes. The hub stops a machine whose wake never
    /// produced a provider, so these no longer mean money is still burning —
    /// its own `detail` says whether that stop worked.
    pub wake_failures: u64,
    pub last_exit: Option<NodeLastExit>,
    /// THIS hub process issued the stop that put the machine to sleep. Absent
    /// means "this hub did not do it", NEVER "somebody else did": it is
    /// in-memory only, so a restarted hub honestly stops claiming it.
    pub slept_by_hub: bool,
    /// The hub's belief about the MACHINE's power, which is a different
    /// question from whether its provider answers — `unreachable` covers both
    /// "running and providing nothing" (a meter) and "off and broken" (nothing
    /// to switch off). Read verbatim; never inferred from `detail`.
    pub may_be_running: bool,
}

/// Coerce one wire row into a [`NodeView`], or `None` if it is not one — a row
/// without an `id` can't be keyed, let alone woken.
pub fn node_from_row(raw: &Value) -> Option<NodeView> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let label = raw
        .get("label")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    let last_exit = raw.get("lastExit").and_then(|e| {
        let exit = NodeLastExit {
            reason: e
                .get("reason")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from),
            exit_code: e.get("exitCode").and_then(Value::as_i64),
            at: e
                .get("at")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from),
        };
        // An all-empty record is no record: `Clean()` is false for one the hub
        // does not hold, and an empty struct here would read as "ended fine".
        (exit != NodeLastExit::default()).then_some(exit)
    });
    Some(NodeView {
        id: id.to_string(),
        label: label.to_string(),
        state: raw
            .get("state")
            .and_then(Value::as_str)
            .map(NodeState::from_wire)
            .unwrap_or(NodeState::Unreachable),
        detail: raw
            .get("detail")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        wakeable: raw.get("wakeable").and_then(Value::as_bool) == Some(true),
        wake_failures: raw.get("wakeFailures").and_then(Value::as_u64).unwrap_or(0),
        last_exit,
        // Both are positive assertions, like `wakeable`: anything that is not
        // literally `true` means the hub did not say yes.
        slept_by_hub: raw.get("sleptByHub").and_then(Value::as_bool) == Some(true),
        may_be_running: raw.get("mayBeRunning").and_then(Value::as_bool) == Some(true),
    })
}

/// Coerce a whole `nodes.list` answer, dropping rows that aren't nodes.
pub fn nodes_from_rows(raw: &Value) -> Vec<NodeView> {
    raw.as_array()
        .map(|rows| rows.iter().filter_map(node_from_row).collect())
        .unwrap_or_default()
}

// ── the store ───────────────────────────────────────────────────────────────

/// The hub's node registry as this client holds it. Registry order is the
/// hub's order and is preserved: `nodes.list` returns the order the nodes are
/// written in `nodes.json`, and a list that reshuffles under a cursor — with a
/// money button on it — is its own bug.
#[derive(Debug, Default, Clone)]
pub struct NodeRegistry {
    nodes: Vec<NodeView>,
}

impl NodeRegistry {
    pub fn new(nodes: Vec<NodeView>) -> NodeRegistry {
        NodeRegistry { nodes }
    }

    /// Adopt a `nodes.list` answer wholesale — it is the authoritative roster.
    pub fn seed(&mut self, nodes: Vec<NodeView>) {
        self.nodes = nodes;
    }

    /// Patch one row from `node.state_changed` (or a `nodes.wake` answer). An
    /// id we don't hold is appended: the registry is hand-edited and can grow
    /// under a long-lived client.
    pub fn apply_change(&mut self, incoming: NodeView) {
        match self.nodes.iter_mut().find(|n| n.id == incoming.id) {
            Some(slot) => *slot = incoming,
            None => self.nodes.push(incoming),
        }
    }

    pub fn nodes(&self) -> &[NodeView] {
        &self.nodes
    }

    pub fn get(&self, id: &str) -> Option<&NodeView> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Nodes that are NOT quietly fine — the ones worth putting in front of
    /// someone unasked. `available` WITH a crash notice counts: a node back
    /// from a crash is available and carrying the only notice of that crash.
    pub fn needing_attention(&self) -> Vec<&NodeView> {
        self.nodes
            .iter()
            .filter(|n| n.state != NodeState::Available || crash_notice(n).is_some())
            .collect()
    }

    /// One line for the dashboard: `"2 machines · 1 asleep"`-shaped, in the
    /// order that matters most first. Empty when the registry is.
    pub fn summary(&self) -> String {
        if self.nodes.is_empty() {
            return String::new();
        }
        [
            (NodeState::Waking, "starting"),
            (NodeState::Stopping, "shutting down"),
            (NodeState::Unreachable, "unreachable"),
            (NodeState::Stopped, "asleep"),
            (NodeState::Available, "connected"),
        ]
        .iter()
        .filter_map(|(state, word)| {
            let n = self.nodes.iter().filter(|x| x.state == *state).count();
            (n > 0).then(|| format!("{n} {word}"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

// ── the sentences ───────────────────────────────────────────────────────────

/// The cost sentence. A wake starts a meter, and the meter runs until somebody
/// stops the machine. The hub grew a stop verb (`nodes.sleep`) and this client
/// does not offer it yet, so the note names who can rather than claiming — as
/// it used to, and as is no longer true — that nobody can.
pub const WAKE_COST_NOTE: &str = "starts a real machine. it bills from boot until it is put back \
     to sleep, which needs the desktop or the web app — not this one.";

/// The sentence under a node's chip: the hub's own `detail` when it wrote one
/// (it is written to be read by a person), else the state's fallback.
pub fn detail_line(node: &NodeView) -> &str {
    node.detail
        .as_deref()
        .unwrap_or_else(|| node.state.fallback_detail())
}

/// The node telling you its last run crashed — the only notice anyone gets,
/// and it arrives one wake LATE by construction (the file lives on the node's
/// volume, so the hub cannot read it while the node is off).
///
/// `None` for a `signal-` reason (a deliberate stop) and for a missing record,
/// which means NOBODY KNOWS rather than "it ended cleanly".
pub fn crash_notice(node: &NodeView) -> Option<String> {
    let exit = node.last_exit.as_ref()?;
    let reason = exit.reason.as_deref()?;
    if reason.starts_with("signal-") {
        return None;
    }
    let code = exit
        .exit_code
        .map(|c| format!(" (exit {c})"))
        .unwrap_or_default();
    let when = exit
        .at
        .as_deref()
        .map(|a| format!(" at {a}"))
        .unwrap_or_default();
    Some(format!(
        "its previous run did not end cleanly: {reason}{code}{when}."
    ))
}

/// Failed wakes, reported honestly. `None` when there have been none.
///
/// This no longer warns about a machine left billing: the hub now stops one
/// whose wake never produced a provider, and its own `detail` — rendered above
/// this line — is the only thing that knows whether that stop worked.
pub fn wake_failure_notice(node: &NodeView) -> Option<String> {
    let n = node.wake_failures;
    if n == 0 {
        return None;
    }
    let s = if n == 1 { "" } else { "s" };
    Some(format!(
        "{n} wake{s} failed. the machine started and never became usable — check its boot log."
    ))
}

// ── the action ──────────────────────────────────────────────────────────────

/// Whether this node gets a wake affordance, and whether this caller may press
/// it. See [`wake_affordance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeAffordance {
    /// Show anything at all? `false` = no control, not a dead one.
    pub visible: bool,
    /// May it actually fire? A visible-but-refused control still explains itself.
    pub enabled: bool,
    /// What the row offers, e.g. `"w  wake"`.
    pub label: &'static str,
    /// Why it can't fire (or, when it can, what it will cost). Always a
    /// sentence — there is no hover in a terminal, so a tooltip is not an
    /// option and everything has to be on the screen.
    pub reason: &'static str,
}

/// Should this node offer a wake, and may this caller press it?
///
/// The rule that matters: **never offer an action that will be refused.**
/// `nodes.wake` is host-authority-only, so a view- or triage-tier connection
/// gets the STATE and not the verb — that silent-failure class is what this
/// feature exists to remove. Same for `wakeable:false`, which is the hub
/// saying it holds no cloud coordinates or credential for this node: the wake
/// would fail every single time, and each failure leaves money burning.
///
/// `stopping` is refused for a different reason, and it is the one this rule
/// nearly missed: the hub ACCEPTS a wake there and cancels the stop. Offering
/// it means someone reading a row for a shutdown they just asked for can undo
/// it with one key and be billed for the privilege. The transition owns the row
/// until it settles.
pub fn wake_affordance(node: &NodeView, can_wake: bool, pending: bool) -> WakeAffordance {
    if node.state == NodeState::Available {
        return WakeAffordance {
            visible: false,
            enabled: false,
            label: "",
            reason: "",
        };
    }
    // Before the `pending` check: the hub's state is authoritative over our own
    // optimistic flag, and "already shutting down" is the more useful sentence.
    if node.state == NodeState::Stopping {
        return WakeAffordance {
            visible: true,
            enabled: false,
            label: "shutting down…",
            reason: "already shutting down — wait for it to stop, then wake it.",
        };
    }
    if node.state == NodeState::Waking || pending {
        return WakeAffordance {
            visible: true,
            enabled: false,
            label: "starting…",
            reason: "already starting — waking it again would do nothing.",
        };
    }
    if !node.wakeable {
        return WakeAffordance {
            visible: true,
            enabled: false,
            label: "wake",
            reason: "this hub holds no cloud credentials for this machine, so it cannot start it.",
        };
    }
    if !can_wake {
        return WakeAffordance {
            visible: true,
            enabled: false,
            label: "wake",
            reason: "starting a machine spends money, so it needs an operator token.",
        };
    }
    WakeAffordance {
        visible: true,
        enabled: true,
        label: "w  wake",
        reason: WAKE_COST_NOTE,
    }
}

// ── error reading ───────────────────────────────────────────────────────────

/// Is this "the hub has no node registry" rather than a failure?
///
/// The bus router's own words for an unregistered method are `no provider for
/// <method>`, and that is the definitive signal. Anything else (a dropped
/// socket, a timeout, a malformed answer) is a real error and must NOT be
/// swallowed into "feature absent".
pub fn is_registry_absent(err: &str) -> bool {
    err.contains("no provider for nodes.list") || err.contains("no provider for nodes.wake")
}

/// Is this the tier refusal — a view/triage token asking to spend money?
pub fn is_host_authority_refusal(err: &str) -> bool {
    err.contains("requires host authority")
}

/// A `nodes.wake` failure in words a person can act on. The hub already
/// renders cloud-API failures BY CATEGORY rather than quoting the API's
/// response body, so its own text is safe to pass through; these cases are the
/// ones where the hub's wording describes a client bug.
pub fn describe_wake_error(err: &str) -> String {
    let msg = err.trim();
    if is_host_authority_refusal(msg) {
        return "starting a machine needs an operator token.".into();
    }
    if msg.contains("unknown node") || msg.contains("naming a registered node is required") {
        return "this machine is no longer in the registry.".into();
    }
    if msg.contains("has no cloud coordinates or credential") {
        return "this hub holds no cloud credentials for this machine.".into();
    }
    if is_registry_absent(msg) {
        return "this hub no longer has a node registry.".into();
    }
    if msg.is_empty() {
        return "couldn't start the machine.".into();
    }
    msg.to_string()
}

// ── the async tasks ─────────────────────────────────────────────────────────

/// Seed the registry from `nodes.list`. Called on every bus (re)connect and
/// when the overlay is opened — the hub keeps no node state across a restart,
/// so a reconnect is exactly when our copy is most likely wrong.
///
/// Sends [`AppMsg::Nodes`] with `None` ONLY for the feature-absent answer. Any
/// other failure sends nothing at all and leaves whatever we hold in place: a
/// hub outage is not this surface's to own, and blanking the list on a
/// reconnect blip would render a broken hub as a hub with no nodes.
pub async fn seed_nodes(bus: BusClient, tx: UnboundedSender<AppMsg>) {
    match bus.call("nodes.list", json!({})).await {
        Ok(v) => {
            let _ = tx.send(AppMsg::Nodes(Some(nodes_from_rows(&v))));
        }
        Err(e) => {
            if is_registry_absent(&e.to_string()) {
                let _ = tx.send(AppMsg::Nodes(None));
            }
        }
    }
}

/// Fire `nodes.wake` for one node and post the outcome back.
///
/// The call returns as soon as the cloud API has accepted the start —
/// normally `state:"waking"` — and the rest arrives on `node.state_changed`.
/// That is the design, not a missing await: a real `waking` state beats a
/// spinner on a held request, and it is why there is no queue for input typed
/// during a wake.
pub async fn wake_node(bus: BusClient, id: String, tx: UnboundedSender<AppMsg>) {
    let msg = match bus.call("nodes.wake", json!({ "id": id })).await {
        Ok(v) => AppMsg::NodeWake {
            id,
            node: node_from_row(&v).map(Box::new),
            error: None,
        },
        Err(e) => AppMsg::NodeWake {
            id,
            node: None,
            error: Some(describe_wake_error(&e.to_string())),
        },
    };
    let _ = tx.send(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(state: NodeState) -> NodeView {
        NodeView {
            id: "den".into(),
            label: "Fly node (den)".into(),
            state,
            detail: None,
            wakeable: true,
            wake_failures: 0,
            last_exit: None,
            slept_by_hub: false,
            may_be_running: false,
        }
    }

    #[test]
    fn a_minimal_row_parses_and_a_row_without_an_id_does_not() {
        let n = node_from_row(&json!({
            "id": "den", "state": "available", "wakeable": false
        }))
        .expect("a minimal NodeView is a node");
        assert_eq!(n.id, "den");
        // label ALWAYS present on the wire, but falling back to the id keeps a
        // sparse row renderable rather than blank.
        assert_eq!(n.label, "den");
        assert_eq!(n.state, NodeState::Available);
        assert!(!n.wakeable);
        assert!(node_from_row(&json!({ "state": "stopped" })).is_none());
        assert!(node_from_row(&json!({ "id": "" })).is_none());
    }

    /// A state we cannot presume to understand is one we do not know how to get
    /// a working node out of — which is the definition of `unreachable`.
    #[test]
    fn an_unknown_state_reads_as_unreachable_not_as_available() {
        let n =
            node_from_row(&json!({ "id": "x", "state": "hibernating", "wakeable": true })).unwrap();
        assert_eq!(n.state, NodeState::Unreachable);
        // …but a state we DO know is never coerced. `stopping` arrived with the
        // sleep path and reading it as `unreachable` is the bug this arm exists
        // to prevent.
        let n =
            node_from_row(&json!({ "id": "x", "state": "stopping", "wakeable": true })).unwrap();
        assert_eq!(n.state, NodeState::Stopping);
        // …and so does a row that names no state at all.
        let n = node_from_row(&json!({ "id": "x", "wakeable": true })).unwrap();
        assert_eq!(n.state, NodeState::Unreachable);
    }

    /// `wakeable` is a positive assertion. Anything that is not literally
    /// `true` — absent, null, the string "true" — means the hub did not say
    /// yes, and offering a wake on it would fail every time.
    #[test]
    fn wakeable_is_only_true_when_the_hub_said_true() {
        for raw in [json!(null), json!("true"), json!(1), json!(false)] {
            let n =
                node_from_row(&json!({ "id": "x", "state": "stopped", "wakeable": raw })).unwrap();
            assert!(!n.wakeable, "{raw} must not read as wakeable");
        }
        assert!(
            node_from_row(&json!({ "id": "x", "state": "stopped", "wakeable": true }))
                .unwrap()
                .wakeable
        );
    }

    #[test]
    fn an_empty_last_exit_is_no_last_exit() {
        // The hub never fabricates an empty record, and neither may we: a
        // present-but-empty one would read as "it ended cleanly".
        let n = node_from_row(&json!({ "id": "x", "state": "available", "lastExit": {} })).unwrap();
        assert!(n.last_exit.is_none());
        assert!(crash_notice(&n).is_none());
    }

    #[test]
    fn a_crash_notice_is_rendered_and_a_deliberate_stop_is_not() {
        let crashed = node_from_row(&json!({
            "id": "x", "state": "available", "wakeable": true,
            "lastExit": { "reason": "claudemon-died", "exitCode": 1, "at": "2026-08-24T21:00:00Z" }
        }))
        .unwrap();
        let notice = crash_notice(&crashed).expect("a crash is news");
        assert!(notice.contains("did not end cleanly"), "{notice}");
        assert!(notice.contains("claudemon-died"), "{notice}");
        assert!(notice.contains("exit 1"), "{notice}");

        let stopped = node_from_row(&json!({
            "id": "x", "state": "stopped", "wakeable": true,
            "lastExit": { "reason": "signal-TERM", "at": "2026-08-24T21:00:00Z" }
        }))
        .unwrap();
        assert!(
            crash_notice(&stopped).is_none(),
            "a deliberate stop is not news"
        );
    }

    /// The hub STOPS a machine whose wake never produced a provider, so this
    /// notice must not still claim money is burning — the hub's own `detail`,
    /// rendered above it, is the only thing that knows whether that stop worked.
    #[test]
    fn failed_wakes_are_reported_without_claiming_the_machine_is_still_billing() {
        let mut n = node(NodeState::Unreachable);
        assert!(wake_failure_notice(&n).is_none());
        n.wake_failures = 2;
        let notice = wake_failure_notice(&n).unwrap();
        assert!(notice.contains("2 wakes failed"), "{notice}");
        assert!(notice.contains("never became usable"), "{notice}");
        assert!(!notice.contains("running and billing"), "{notice}");
    }

    /// The two fields the sleep path added. Both are positive assertions, and
    /// both are kept rather than dropped: the honest reading of a stop is built
    /// out of them.
    #[test]
    fn the_sleep_paths_two_wire_fields_are_read_and_kept() {
        let n = node_from_row(&json!({
            "id": "den", "state": "stopped", "wakeable": true,
            "sleptByHub": true, "mayBeRunning": false
        }))
        .unwrap();
        assert!(n.slept_by_hub);
        assert!(!n.may_be_running);

        // Absent means "this hub did not say so", never "somebody else did" —
        // and anything that is not literally `true` is absent.
        for raw in [json!(null), json!("true"), json!(1), json!(false)] {
            let n = node_from_row(&json!({
                "id": "den", "state": "unreachable", "wakeable": true,
                "sleptByHub": raw, "mayBeRunning": raw
            }))
            .unwrap();
            assert!(!n.slept_by_hub, "{raw} must not read as slept-by-hub");
            assert!(!n.may_be_running, "{raw} must not read as may-be-running");
        }

        let running = node_from_row(&json!({
            "id": "den", "state": "unreachable", "wakeable": true, "mayBeRunning": true
        }))
        .unwrap();
        assert!(running.may_be_running);
    }

    /// The whole point of the five-state model: a booting machine and a broken
    /// one must not read the same.
    #[test]
    fn waking_is_not_unreachable() {
        assert_ne!(
            NodeState::Waking.label(),
            NodeState::Unreachable.label(),
            "distinct words"
        );
        assert_ne!(
            NodeState::Waking.marker(),
            NodeState::Unreachable.marker(),
            "distinct marks"
        );
        // …and a machine already starting must not offer a second wake.
        let a = wake_affordance(&node(NodeState::Waking), true, false);
        assert!(a.visible && !a.enabled);
        assert!(a.reason.contains("already starting"), "{}", a.reason);
    }

    /// The other half of that model, and the one this client got wrong: a
    /// machine shutting down on purpose is not a machine it cannot reach.
    #[test]
    fn stopping_is_not_unreachable_and_is_not_waking_either() {
        for other in [
            NodeState::Unreachable,
            NodeState::Waking,
            NodeState::Stopped,
        ] {
            assert_ne!(
                NodeState::Stopping.label(),
                other.label(),
                "distinct words from {other:?}"
            );
            assert_ne!(
                NodeState::Stopping.marker(),
                other.marker(),
                "distinct marks from {other:?}"
            );
        }
        // It reads as progress, like `waking` — not as a fault. The words are
        // what carry that here, so pin them rather than the colour.
        assert!(NodeState::Stopping.label().ends_with('…'));
        assert!(
            NodeState::Stopping
                .fallback_detail()
                .contains("shutting down"),
            "{}",
            NodeState::Stopping.fallback_detail()
        );
        // And it is counted, rather than falling through the summary's list.
        let reg = NodeRegistry::new(nodes_from_rows(&json!([
            { "id": "draining", "state": "stopping", "wakeable": true }
        ])));
        assert_eq!(reg.summary(), "1 shutting down");
        assert_eq!(reg.needing_attention().len(), 1);
    }

    /// The bug this task exists for. The hub ACCEPTS a wake on a `stopping`
    /// node and cancels the stop, so the refusal has to live here: someone
    /// reading a row for a shutdown they just asked for must not be able to
    /// undo it with one key and be billed for it.
    #[test]
    fn a_wake_is_refused_on_a_machine_that_is_already_shutting_down() {
        let a = wake_affordance(&node(NodeState::Stopping), true, false);
        assert!(a.visible, "the row still explains itself");
        assert!(!a.enabled, "and it does NOT offer a wake");
        assert!(a.reason.contains("already shutting down"), "{}", a.reason);
        // Not even for an operator with a wake already in flight, and not for
        // a node the hub holds no credential for either — the state decides.
        assert!(!wake_affordance(&node(NodeState::Stopping), true, true).enabled);
        let mut credentialless = node(NodeState::Stopping);
        credentialless.wakeable = false;
        let a = wake_affordance(&credentialless, true, false);
        assert!(!a.enabled);
        assert!(a.reason.contains("already shutting down"), "{}", a.reason);
    }

    #[test]
    fn a_connected_node_offers_nothing_at_all() {
        let a = wake_affordance(&node(NodeState::Available), true, false);
        assert!(!a.visible, "a healthy machine is not news");
    }

    /// Never offer an action that will be refused: the two refusals the hub
    /// would issue are both pre-empted here, WITH the reason on the screen.
    #[test]
    fn a_wake_is_never_offered_where_the_hub_would_refuse_it() {
        let view_tier = wake_affordance(&node(NodeState::Stopped), false, false);
        assert!(view_tier.visible && !view_tier.enabled);
        assert!(
            view_tier.reason.contains("operator token"),
            "{}",
            view_tier.reason
        );

        let mut credentialless = node(NodeState::Unreachable);
        credentialless.wakeable = false;
        let a = wake_affordance(&credentialless, true, false);
        assert!(a.visible && !a.enabled);
        assert!(a.reason.contains("no cloud credentials"), "{}", a.reason);
    }

    /// The cost is beside the action, not hidden behind a hover a terminal
    /// does not have.
    #[test]
    fn an_enabled_wake_prints_what_it_costs() {
        let a = wake_affordance(&node(NodeState::Stopped), true, false);
        assert!(a.enabled);
        assert_eq!(a.reason, WAKE_COST_NOTE);
        assert!(a.reason.contains("bills from boot"), "{}", a.reason);
        // `nodes.sleep` exists now, so the note must NOT still say nothing can
        // stop the machine — it names who can, because this client cannot.
        assert!(
            !a.reason.contains("nothing here can stop it"),
            "{}",
            a.reason
        );
        assert!(a.reason.contains("put back"), "{}", a.reason);
        assert!(a.reason.contains("not this one"), "{}", a.reason);
        // A wake already in flight closes the affordance the same way `waking`
        // does — three keystrokes must not become three cloud API calls.
        assert!(!wake_affordance(&node(NodeState::Stopped), true, true).enabled);
    }

    /// A permission check is not a feature check: the ONLY honest signal that a
    /// hub has no registry is the router's own "no provider" sentence.
    #[test]
    fn only_no_provider_reads_as_feature_absent() {
        assert!(is_registry_absent("no provider for nodes.list"));
        assert!(is_registry_absent("no provider for nodes.wake"));
        for real in [
            "bus disconnected",
            "bus write failed",
            "requires host authority",
            "no provider for agents.spawn",
            "",
        ] {
            assert!(!is_registry_absent(real), "{real} is a real failure");
        }
    }

    #[test]
    fn wake_errors_are_rendered_for_a_person() {
        assert_eq!(
            describe_wake_error("nodes.wake requires host authority"),
            "starting a machine needs an operator token."
        );
        assert_eq!(
            describe_wake_error("unknown node \"den\""),
            "this machine is no longer in the registry."
        );
        assert_eq!(
            describe_wake_error("den has no cloud coordinates or credential on this hub"),
            "this hub holds no cloud credentials for this machine."
        );
        assert_eq!(describe_wake_error("   "), "couldn't start the machine.");
        // The hub renders cloud failures BY CATEGORY, never quoting the API —
        // so its own sentence passes through untouched.
        assert_eq!(
            describe_wake_error("the cloud API is rate-limiting this machine"),
            "the cloud API is rate-limiting this machine"
        );
    }

    #[test]
    fn the_registry_keeps_the_hubs_order_and_patches_by_id() {
        let mut reg = NodeRegistry::new(nodes_from_rows(&json!([
            { "id": "a", "state": "available", "wakeable": true },
            { "id": "b", "state": "stopped", "wakeable": true },
        ])));
        assert_eq!(reg.nodes().len(), 2);
        reg.apply_change(
            node_from_row(&json!({ "id": "b", "state": "waking", "wakeable": true })).unwrap(),
        );
        assert_eq!(reg.get("b").unwrap().state, NodeState::Waking);
        // Order is the hub's, and patching must not reshuffle it.
        assert_eq!(
            reg.nodes()
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        // A node the registry grew under us appends rather than being dropped.
        reg.apply_change(
            node_from_row(&json!({ "id": "c", "state": "stopped", "wakeable": true })).unwrap(),
        );
        assert_eq!(reg.nodes().len(), 3);
        assert_eq!(reg.nodes()[2].id, "c");
    }

    #[test]
    fn only_nodes_worth_interrupting_someone_for_need_attention() {
        let reg = NodeRegistry::new(nodes_from_rows(&json!([
            { "id": "fine", "state": "available", "wakeable": true },
            { "id": "asleep", "state": "stopped", "wakeable": true },
            { "id": "draining", "state": "stopping", "wakeable": true },
            { "id": "crashed", "state": "available", "wakeable": true,
              "lastExit": { "reason": "brain-died", "exitCode": 2 } },
        ])));
        let ids: Vec<&str> = reg
            .needing_attention()
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        // A healthy machine says nothing; a machine back from a crash carries
        // the only notice of that crash, so it counts even though it is up.
        assert_eq!(ids, vec!["asleep", "draining", "crashed"]);
        assert_eq!(reg.summary(), "1 shutting down · 1 asleep · 2 connected");
        assert!(NodeRegistry::default().summary().is_empty());
    }
}

// ── the cross-language contract ─────────────────────────────────────────────
//
// A SEPARATE module, appended at the end of the file on purpose: this client's
// state vocabulary is under active edit, and a self-contained block is the
// cheapest thing to merge.
//
// `contracts/node-view-cases.json` is the corpus. The other loaders are
// `services/hub/internal/nodes/view_test.go` (the side that WRITES the payload)
// and `apps/desktop/src/renderer/tests/remoteNodes.test.ts`.
//
// THE POINT OF THE `null` COLUMN. This client is BEHIND the contract: the sleep
// path added `stopping` to the hub, the desktop and /m and not here, so
// `from_wire` still coerces it to `Unreachable` — the warning marker, the
// "can't reach" chip, and a `w  wake` offer for a machine in the middle of
// stopping. The fixture records that as a null tui* triple, and the tests below
// hold this client to the DOCUMENTED FALLBACK for exactly those states rather
// than to a presentation it does not have. So teaching this client a state is a
// failing test until the fixture is filled in, which is the whole mechanism.
#[cfg(test)]
mod node_view_contract {
    use super::*;

    const FIXTURE: &str = include_str!("../../../contracts/node-view-cases.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("contracts/node-view-cases.json does not parse")
    }

    fn rows<'a>(v: &'a Value, block: &[&str]) -> &'a Vec<Value> {
        let mut cur = v;
        for k in block {
            cur = cur.get(k).unwrap_or_else(|| {
                panic!("contracts/node-view-cases.json has no {k:?} — a block was renamed and this test is asserting nothing")
            });
        }
        cur.as_array().expect("block is not an array")
    }

    fn str_of<'a>(row: &'a Value, key: &str) -> &'a str {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("case is missing the string field {key:?}"))
    }

    /// Has this client been taught the state? The fixture's tui* column is the
    /// answer, so nothing here re-declares the vocabulary a third time.
    fn taught(row: &Value) -> bool {
        !row.get("tuiLabel").map(Value::is_null).unwrap_or(true)
    }

    fn node(state: &str) -> NodeView {
        NodeView {
            id: "ord".into(),
            label: "ord".into(),
            state: NodeState::from_wire(state),
            detail: None,
            wakeable: true,
            wake_failures: 0,
            last_exit: None,
            slept_by_hub: false,
            may_be_running: false,
        }
    }

    #[test]
    fn node_view_contract_states() {
        let fx = fixture();
        let taught_states: std::collections::HashSet<String> =
            rows(&fx, &["presentation", "cases"])
                .iter()
                .filter(|r| taught(r))
                .map(|r| str_of(r, "state").to_string())
                .collect();

        // A state string this client does not recognise becomes `unreachable`
        // and is never rendered raw: a state it cannot presume to understand is
        // one it does not know how to get a working node out of.
        for s in fx["unknownStates"].as_array().expect("unknownStates") {
            let s = s.as_str().expect("unknownStates entry is not a string");
            assert_eq!(
                NodeState::from_wire(s),
                NodeState::Unreachable,
                "the contract says {s:?} must coerce to `unreachable`"
            );
        }

        let mut checked = 0;
        for row in rows(&fx, &["states"]) {
            let state = str_of(row, "state");
            if !taught_states.contains(state) {
                continue; // the declared gap; node_view_contract_presentation owns it
            }
            checked += 1;
            let offered = row["wakeOffered"].as_bool().expect("wakeOffered");
            // THE MONEY COLUMN. `w  wake` starts a real machine, and the states
            // that must NOT offer it are the ones where a wake would fight
            // something already in flight.
            assert_eq!(
                wake_affordance(&node(state), true, false).enabled,
                offered,
                "state {state:?}: the contract says wakeOffered={offered}"
            );
            let transitional = row["transitional"].as_bool().expect("transitional");
            // The working markers, shared with a thinking agent: the two
            // halves of one filling circle, `waking` filling one way and
            // `stopping` the other. They are what keep a machine in motion
            // from painting like a failure, and the assertion bites hardest on
            // the other three states, which must NOT borrow a progress mark.
            let progress = ["◐", "◑"];
            assert_eq!(
                progress.contains(&NodeState::from_wire(state).marker()),
                transitional,
                "state {state:?}: the contract says transitional={transitional}"
            );
        }
        assert!(
            checked >= 4,
            "only {checked} contract states were checked — the tui* columns went null and this test stopped asserting anything"
        );
    }

    #[test]
    fn node_view_contract_last_exit() {
        let fx = fixture();
        assert_eq!(
            fx["lastExit"]["cleanPrefix"].as_str(),
            Some("signal-"),
            "the prefix this client tests for is hard-coded in crash_notice"
        );
        let (mut cleans, mut crashes) = (0, 0);
        for row in rows(&fx, &["lastExit", "cases"]) {
            let name = str_of(row, "name");
            let reason = str_of(row, "reason");
            let absent = row
                .get("recordAbsent")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut n = node("stopped");
            if !absent {
                n.last_exit = Some(NodeLastExit {
                    reason: (!reason.is_empty()).then(|| reason.to_string()),
                    exit_code: row.get("exitCode").and_then(Value::as_i64),
                    at: row.get("at").and_then(Value::as_str).map(String::from),
                });
            }
            let want_notice = row["notice"].as_bool().expect("notice");
            let got = crash_notice(&n);
            assert_eq!(
                got.is_some(),
                want_notice,
                "{name}: reason {reason:?} — the contract says notice={want_notice}, this client said {got:?}"
            );
            if let Some(line) = &got {
                // A notice that does not name the ending tells a person to look
                // without saying at what.
                assert!(line.contains(reason), "{name}: the notice omits the reason");
            }
            if row["clean"].as_bool().expect("clean") {
                cleans += 1;
                assert!(
                    got.is_none(),
                    "{name}: a deliberate stop got a crash notice"
                );
            }
            if want_notice {
                crashes += 1;
            }
        }
        // A corpus that drifted to all-clean or all-crash would satisfy every
        // assertion above while pinning one arm of the rule.
        assert!(
            cleans > 0 && crashes > 0,
            "the corpus exercises {cleans} clean and {crashes} crash endings — one arm is unpinned"
        );
    }

    #[test]
    fn node_view_contract_presentation() {
        let fx = fixture();
        let mut rendered = 0;
        let mut gaps = Vec::new();
        for row in rows(&fx, &["presentation", "cases"]) {
            let state = str_of(row, "state");
            if !taught(row) {
                // The declared gap. This client coerces the state, and the
                // contract is what says so out loud — including that the
                // coercion target itself can never be a gap.
                assert_ne!(
                    state, "unreachable",
                    "`unreachable` is the coercion target; it cannot be a declared gap"
                );
                assert_eq!(
                    NodeState::from_wire(state),
                    NodeState::Unreachable,
                    "the contract records {state:?} as not yet taught to this client, and from_wire resolves it — fill in the tuiLabel/tuiMarker/tuiFallbackDetail columns in contracts/node-view-cases.json"
                );
                gaps.push(state.to_string());
                continue;
            }
            rendered += 1;
            let s = NodeState::from_wire(state);
            // The labels are unique per variant, so matching all three pins the
            // variant as well as the words.
            assert_eq!(
                s.label(),
                str_of(row, "tuiLabel"),
                "state {state:?}: chip label"
            );
            assert_eq!(
                s.marker(),
                str_of(row, "tuiMarker"),
                "state {state:?}: row marker"
            );
            assert_eq!(
                s.fallback_detail(),
                str_of(row, "tuiFallbackDetail"),
                "state {state:?}: the line under the chip"
            );
            assert_eq!(
                detail_line(&node(state)),
                str_of(row, "tuiFallbackDetail"),
                "state {state:?}: detail_line must fall back to it when the hub sent no detail"
            );
        }
        assert!(
            rendered >= 4,
            "only {rendered} states have a presentation here (gaps: {gaps:?}) — this client has fallen further behind the contract than the corpus records"
        );
    }
}
