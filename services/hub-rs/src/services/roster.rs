//! The session roster: which session rows a hub lists and publishes.
//!
//! This is the ONE membership rule for every client of this hub (native, web,
//! `/m-next`, TUI peers, manager MCP tools). Desktop main implements the same
//! function in `apps/desktop/src/main/services/sessionRoster.ts`; both load
//! `contracts/session-roster-cases.json`. See `docs/session-roster.md`.
//!
//! A row is listed when it is live, when it is a recognisable agent still
//! starting, or when it is stopped and either on the sidebar (the shared
//! [`super::session_open`] set) or curated by a desktop layout. There is no
//! time window: nothing leaves the sidebar on its own. A layout only ever ADDS
//! stopped rows, so one client's pane curation cannot remove sessions from
//! another.
//!
//! The user's Hide is the shared archive (`sessionArchive`), which removes the
//! id from the sidebar set; clients still filter by it while that propagates.
use super::snapshots::{parse_time, state_fields_valid};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};

/// The retired time window, kept only for the one-time sidebar seed: rows
/// stopped this recently were visible before the upgrade.
pub const RECENT_HOURS: i64 = 24;

/// The shared inputs a membership decision may consult.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// The hub's shared open set (`sessionOpen`).
    pub open: BTreeSet<String>,
    /// Session ids a desktop/web layout curates.
    pub layout: BTreeSet<String>,
    /// Terminal shells this hub launched; never agents, so never Starting.
    pub shells: BTreeSet<String>,
}

fn id(row: &Value) -> &str {
    row["session_id"]
        .as_str()
        .or_else(|| row["sessionId"].as_str())
        .unwrap_or("")
}

fn valid(row: &Value) -> bool {
    state_fields_valid(row, &["session_id", "mode", "status", "updated_at"])
}

/// Stopped (or a legacy row that only says `status: ended`).
pub fn ended(row: &Value) -> bool {
    let mode = row["mode"].as_str().unwrap_or("");
    mode == "stopped" || (mode.is_empty() && row["status"] == "ended")
}

/// A running session with a known state.
pub fn live(row: &Value) -> bool {
    valid(row) && row["mode"] != "unknown" && !ended(row)
}

/// An agent session that has not reported its first state yet (a spawn or a
/// resume). A Claude PTY row in `unknown` is NOT starting: terminal shells are
/// indistinguishable from it by row fields, so only a stream-transport or
/// managed-provider row qualifies.
pub fn starting(row: &Value, ctx: &Ctx) -> bool {
    if !valid(row) || row["mode"] != "unknown" || ctx.shells.contains(id(row)) {
        return false;
    }
    row["transport"] == "stream"
        || row["provider"]
            .as_str()
            .is_some_and(|provider| !provider.is_empty() && provider != "claude")
}

/// Whether the hub lists (and publishes) this row.
pub fn listed(row: &Value, ctx: &Ctx, now: time::OffsetDateTime) -> bool {
    if !valid(row) {
        return false;
    }
    if row["mode"] == "unknown" {
        return starting(row, ctx);
    }
    if !ended(row) {
        return true;
    }
    // No time window: a stopped session is listed while it is on the
    // sidebar (open) or curated, and never only because it ended recently.
    let _ = now;
    let id = id(row);
    ctx.open.contains(id) || ctx.layout.contains(id)
}

/// Stopped within [`RECENT_HOURS`] of `now`: what the old time window listed.
/// Used once, to seed the sidebar at upgrade so nothing visible disappears.
pub fn recently_ended(row: &Value, now: time::OffsetDateTime) -> bool {
    valid(row)
        && ended(row)
        && row["updated_at"]
            .as_str()
            .and_then(parse_time)
            .is_some_and(|updated| now - updated <= time::Duration::hours(RECENT_HOURS))
}

/// Mark a listed starting row so clients render "Starting" instead of
/// guessing from `unknown`; clear a stale mark otherwise.
pub fn mark(mut row: Value, ctx: &Ctx) -> Value {
    if !row.is_object() {
        return row;
    }
    if starting(&row, ctx) {
        row["starting"] = Value::Bool(true);
    } else if let Some(fields) = row.as_object_mut() {
        fields.remove("starting");
    }
    row
}

/// Where a hub reads the roster's shared inputs from.
#[derive(Clone, Default)]
pub struct Sources {
    pub layout: Option<Arc<super::layout::Layout>>,
    /// A full-scope provider relay curates with the central hub's layout.
    pub upstream_layout: Option<Arc<RwLock<Value>>>,
    pub open: Option<Arc<super::session_open::SessionOpen>>,
    pub terminals: Option<Arc<super::terminals::Terminals>>,
}

impl Sources {
    pub(crate) fn from_options(options: &crate::Options) -> Self {
        Self {
            layout: options.layout.clone(),
            upstream_layout: options.upstream_layout.clone(),
            open: options.session_open.clone(),
            terminals: options.terminals.clone(),
        }
    }
    fn layout_document(&self) -> Value {
        if let Some(upstream) = &self.upstream_layout {
            return upstream.read().unwrap().clone();
        }
        self.layout.as_ref().map(|l| l.get()).unwrap_or(Value::Null)
    }
    pub fn shells(&self) -> BTreeSet<String> {
        self.terminals
            .as_ref()
            .map(|t| {
                let mut ids = t.owned_shell_ids();
                ids.extend(t.uncertain_shell_ids());
                ids
            })
            .unwrap_or_default()
    }
    /// Every input, for judging many rows at once.
    pub fn ctx(&self) -> Ctx {
        Ctx {
            open: self.open.as_ref().map(|o| o.ids()).unwrap_or_default(),
            layout: super::snapshots::layout_ids(&self.layout_document()).0,
            shells: self.shells(),
        }
    }
    /// Only the inputs `row` can depend on: a live row needs none, an
    /// `unknown` row only the shells, a stopped row the open set and layout.
    pub fn ctx_for(&self, row: &Value) -> Ctx {
        if row["mode"] == "unknown" {
            Ctx {
                shells: self.shells(),
                ..Default::default()
            }
        } else if ended(row) {
            Ctx {
                open: self.open.as_ref().map(|o| o.ids()).unwrap_or_default(),
                layout: super::snapshots::layout_ids(&self.layout_document()).0,
                shells: BTreeSet::new(),
            }
        } else {
            Ctx::default()
        }
    }
    pub fn listed(&self, row: &Value, now: time::OffsetDateTime) -> bool {
        listed(row, &self.ctx_for(row), now)
    }
    pub fn mark(&self, row: Value) -> Value {
        if row["mode"] == "unknown" || row.get("starting").is_some() {
            let ctx = self.ctx_for(&row);
            mark(row, &ctx)
        } else {
            row
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(value: &Value) -> BTreeSet<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn shared_roster_cases() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../contracts/session-roster-cases.json"
        ))
        .unwrap();
        assert!(
            fixture["vocabulary"]["blocks"]["cases"]["loaders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "services/hub-rs/src/services/roster.rs::shared_roster_cases")
        );
        let now = parse_time(fixture["now"].as_str().unwrap()).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        for case in cases {
            let ctx = Ctx {
                open: set(&case["ctx"]["open"]),
                layout: set(&case["ctx"]["layout"]),
                shells: set(&case["ctx"]["shells"]),
            };
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                listed(&case["row"], &ctx, now),
                case["listed"].as_bool().unwrap(),
                "listed: {name}"
            );
            assert_eq!(
                starting(&case["row"], &ctx),
                case["starting"].as_bool().unwrap(),
                "starting: {name}"
            );
            // Starting rows are always listed.
            if starting(&case["row"], &ctx) {
                assert!(listed(&case["row"], &ctx, now), "{name}");
            }
        }
    }

    #[test]
    fn mark_adds_and_clears_the_starting_flag() {
        let ctx = Ctx::default();
        let row = mark(
            json!({"session_id":"a","mode":"unknown","transport":"stream"}),
            &ctx,
        );
        assert_eq!(row["starting"], true);
        let mut later = row.clone();
        later["mode"] = json!("input");
        assert!(mark(later, &ctx).get("starting").is_none());
        let shells = Ctx {
            shells: ["a".into()].into(),
            ..Default::default()
        };
        assert!(mark(row, &shells).get("starting").is_none());
    }
}
