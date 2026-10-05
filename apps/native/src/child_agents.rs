//! Bounded child-agent projection. Provider agent IDs and session IDs are distinct.
use crate::model::{Row, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildKind {
    Workspacer,
    Provider,
}

/// A generic pending spawn has no provenance yet. Do not guess from its name
/// when only the completed receipt distinguishes provider IDs from sessions.
pub fn tool_kind(tool: &crate::transcript::Tool) -> Option<ChildKind> {
    let name = tool.name.to_ascii_lowercase();
    if tool.spawned_session_id().is_some()
        || name.starts_with("mcp__workspacer__") && tool.category() == "Subagent"
    {
        Some(ChildKind::Workspacer)
    } else if matches!(
        name.as_str(),
        "agent" | "task" | "codex_agent" | "functions.spawn_agent"
    ) {
        Some(ChildKind::Provider)
    } else {
        None
    }
}
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Telemetry {
    pub tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub observed_model: String,
    pub cost_usd: Option<f64>,
    pub tool_calls: Option<u64>,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub last_activity_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    pub last_tool_name: String,
    pub last_tool_summary: String,
}
fn text(v: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|k| v[*k].as_str())
        .map(|s| crate::transcript::head(s, 512))
        .unwrap_or_default()
}
pub(crate) fn timestamp(v: &Value, names: &[&str]) -> Option<i64> {
    let value = names.iter().find_map(|k| v.get(*k))?;
    value
        .as_i64()
        .or_else(|| {
            value
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.timestamp_millis())
        })
        .filter(|timestamp| *timestamp != 0)
}
impl Telemetry {
    pub fn merge(&mut self, v: &Value) {
        fn first<'a>(v: &'a Value, names: &[&str]) -> Option<&'a Value> {
            names.iter().find_map(|k| v.get(*k))
        }
        macro_rules! numeric {
            ($field:ident, $names:expr) => {
                if let Some(value) = first(v, $names) {
                    self.$field = value.as_u64();
                }
            };
        }
        macro_rules! at {
            ($field:ident, $names:expr) => {
                if first(v, $names).is_some() {
                    self.$field = timestamp(v, $names);
                }
            };
        }
        numeric!(tokens, &["tokens", "tokenCount", "totalTokens"]);
        numeric!(tool_calls, &["toolCalls", "totalToolCalls", "tool_calls"]);
        numeric!(duration_ms, &["durationMs"]);
        at!(started_at_ms, &["startedAt", "started_at", "createdAt"]);
        at!(completed_at_ms, &["completedAt", "completed_at", "endedAt"]);
        at!(
            last_activity_ms,
            &["lastActivity", "updatedAt", "updated_at"]
        );
        let line = v.get("statusLine").or_else(|| v.get("status_line"));
        let usage = v.get("usage");
        let input = line
            .and_then(|line| first(line, &["totalInputTokens", "total_input_tokens"]))
            .or_else(|| usage.and_then(|usage| first(usage, &["inputTokens", "input_tokens"])));
        let output = line
            .and_then(|line| first(line, &["totalOutputTokens", "total_output_tokens"]))
            .or_else(|| usage.and_then(|usage| first(usage, &["outputTokens", "output_tokens"])));
        if let Some(input) = input {
            self.input_tokens = input.as_u64();
        }
        if let Some(output) = output {
            self.output_tokens = output.as_u64();
        }
        if input.is_some() || output.is_some() {
            self.tokens = self
                .input_tokens
                .zip(self.output_tokens)
                .map(|(i, o)| i.saturating_add(o));
        } else if let Some(value) = usage.and_then(|usage| first(usage, &["totalTokens", "tokens"]))
        {
            self.tokens = value.as_u64();
        }
        // Context tokens are current context, never cumulative consumption.
        if let Some(cost) = first(v, &["costUSD", "totalCostUsd"])
            .or_else(|| usage.and_then(|usage| first(usage, &["costUSD", "cost_usd"])))
            .or_else(|| line.and_then(|line| first(line, &["costUSD", "cost_usd"])))
        {
            self.cost_usd = cost.as_f64().filter(|n| n.is_finite() && *n >= 0.);
        }
        if let Some(model) = usage
            .and_then(|usage| usage.get("model"))
            .or_else(|| v.get("model"))
        {
            self.observed_model = model
                .as_str()
                .map(|s| crate::transcript::head(s, 512))
                .unwrap_or_default();
        }
        for (target, names) in [
            (
                &mut self.last_tool_name,
                &["lastToolName", "last_tool_name"][..],
            ),
            (
                &mut self.last_tool_summary,
                &["lastToolSummary", "last_tool_summary"][..],
            ),
        ] {
            if first(v, names).is_some() {
                *target = text(v, names);
            }
        }
        // A resumed worker must not retain a duration or completion from its
        // previous run even when the sparse update omits those fields.
        if provider_active(text(v, &["status", "mode", "ambientState"]).as_str()) {
            self.completed_at_ms = None;
            self.duration_ms = None;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChildAgent {
    pub id: String,
    pub session_id: Option<String>,
    pub label: String,
    pub description: String,
    pub agent_type: String,
    pub status: String,
    pub model: String,
    pub telemetry: Telemetry,
}
impl ChildAgent {
    pub fn running(&self) -> bool {
        matches!(
            self.status.as_str(),
            "running" | "responding" | "streaming" | "working" | "thinking" | "background"
        )
    }
    pub fn failed(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "error" | "lost")
    }
    pub fn complete(&self) -> bool {
        matches!(
            self.status.as_str(),
            "complete" | "completed" | "done" | "stopped" | "ended"
        )
    }
    /// Finished for good: done, failed, ended, or a Workspacer child session
    /// back at its prompt.
    pub fn settled(&self) -> bool {
        self.complete()
            || self.failed()
            || (self.session_id.is_some() && matches!(self.status.as_str(), "input" | "idle"))
    }
    pub fn duration_ms(&self, now_ms: i64) -> Option<u64> {
        self.telemetry.duration_ms.or_else(|| {
            let start = self.telemetry.started_at_ms?;
            let end = if self.running() {
                Some(now_ms)
            } else {
                self.telemetry.completed_at_ms.or_else(|| {
                    (self.session_id.is_some()
                        && matches!(self.status.as_str(), "stopped" | "ended"))
                    .then_some(self.telemetry.last_activity_ms)
                    .flatten()
                })
            }?;
            u64::try_from(end.checked_sub(start)?).ok()
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChildAgents {
    pub by_tool: BTreeMap<String, Vec<ChildAgent>>,
    pub unanchored: Vec<ChildAgent>,
}
fn from_session(s: &Session) -> ChildAgent {
    ChildAgent {
        id: format!("session:{}", s.id),
        session_id: Some(s.id.clone()),
        label: crate::transcript::head(s.title(), 512),
        status: if s.approval.is_some() {
            "waiting_approval".into()
        } else if s.questions.is_some() {
            "waiting_input".into()
        } else {
            s.state.clone()
        },
        model: s.telemetry.observed_model.clone(),
        telemetry: s.telemetry.clone(),
        ..Default::default()
    }
}
/// Exact anchors first. A provider row carrying an unavailable explicit anchor
/// stays unanchored; it must never be silently attached to another spawn.
pub fn project<'a>(
    parent: &Session,
    fleet: &[Session],
    rows: impl IntoIterator<Item = &'a Row>,
) -> ChildAgents {
    let rows: Vec<&Row> = rows.into_iter().collect();
    let mut out = ChildAgents::default();
    let mut sessions = BTreeSet::new();
    let mut native = Vec::new();
    let mut native_ids = BTreeSet::new();
    for sub in parent.subagents.as_array().into_iter().flatten().take(32) {
        let id = text(sub, &["id"]);
        if id.is_empty() || !native_ids.insert(id.clone()) {
            continue;
        }
        let mut telemetry = Telemetry::default();
        telemetry.merge(sub);
        let child = ChildAgent {
            id,
            label: text(sub, &["type", "description"]),
            description: text(sub, &["description"]),
            agent_type: text(sub, &["type"]),
            status: text(sub, &["status"]),
            model: text(sub, &["model"]),
            telemetry,
            ..Default::default()
        };
        native.push((text(sub, &["toolUseId"]), child));
    }
    let mut claimed_native = BTreeSet::new();
    // Successful Workspacer receipts are authoritative session-ID joins.
    for row in &rows {
        let Some(tool) = &row.tool else {
            continue;
        };
        if tool.id.is_empty() {
            continue;
        }
        if let Some(id) = tool.spawned_session_id() {
            if !sessions.insert(id.clone()) {
                continue;
            }
            let child = fleet
                .iter()
                .find(|s| {
                    s.id == id
                        && (s.parent_session_id.is_empty() || s.parent_session_id == parent.id)
                })
                .map(from_session)
                .unwrap_or_else(|| ChildAgent {
                    id: format!("session:{id}"),
                    session_id: Some(id.clone()),
                    label: id,
                    status: "unknown".into(),
                    ..Default::default()
                });
            out.by_tool.entry(tool.id.clone()).or_default().push(child);
        }
        if tool.category() != "Subagent" {
            continue;
        }
        for (index, (anchor, child)) in native.iter().enumerate() {
            if !anchor.is_empty() && anchor == &tool.id && claimed_native.insert(index) {
                out.by_tool
                    .entry(tool.id.clone())
                    .or_default()
                    .push(child.clone());
            }
        }
    }
    // Legacy Claude hook rows have no toolUseId. Only a complete one-to-one
    // set with timestamps proving order can use the historical order fallback.
    let free: Vec<_> = rows
        .iter()
        .filter(|r| {
            r.tool.as_ref().is_some_and(|t| {
                t.name == "Agent" && !t.id.is_empty() && !out.by_tool.contains_key(&t.id)
            })
        })
        .collect();
    let candidates: Vec<_> = native
        .iter()
        .enumerate()
        .filter(|(i, (a, _))| a.is_empty() && !claimed_native.contains(i))
        .collect();
    if parent.provider == "claude"
        && !free.is_empty()
        && free.len() == candidates.len()
        && free.iter().zip(&candidates).all(|(row, (_, (_, child)))| {
            row.timestamp_ms
                .zip(child.telemetry.started_at_ms)
                .is_some_and(|(call, start)| call <= start)
        })
        && free
            .windows(2)
            .all(|pair| pair[0].timestamp_ms <= pair[1].timestamp_ms)
        && candidates
            .iter()
            .zip(free.iter().skip(1))
            .all(|((_, (_, child)), next)| {
                child
                    .telemetry
                    .started_at_ms
                    .zip(next.timestamp_ms)
                    .is_some_and(|(start, call)| start < call)
            })
        && candidates
            .windows(2)
            .all(|pair| pair[0].1.1.telemetry.started_at_ms <= pair[1].1.1.telemetry.started_at_ms)
    {
        for (row, (index, (_, child))) in free.iter().zip(&candidates) {
            claimed_native.insert(*index);
            out.by_tool
                .entry(row.tool.as_ref().unwrap().id.clone())
                .or_default()
                .push(child.clone());
        }
    }
    for (index, (_, child)) in native.into_iter().enumerate() {
        if !claimed_native.contains(&index) {
            out.unanchored.push(child);
        }
    }
    for child in fleet
        .iter()
        .filter(|s| s.parent_session_id == parent.id && s.id != parent.id)
        .take(32)
    {
        if sessions.insert(child.id.clone()) {
            out.unanchored.push(from_session(child));
        }
    }
    out
}

/// Where an unanchored child belongs in the timeline: after the last row that
/// precedes its start, so later messages flow below it. `None` when it has no
/// start time or no row carries a timestamp; the caller then pins it to the
/// row that was newest when the child first appeared.
pub fn overview_anchor<'a>(
    child: &ChildAgent,
    rows: impl IntoIterator<Item = &'a Row>,
) -> Option<usize> {
    let start = child.telemetry.started_at_ms?;
    let mut anchor = None;
    let mut timed = false;
    for (ix, row) in rows.into_iter().enumerate() {
        if let Some(at) = row.timestamp_ms {
            timed = true;
            if at <= start {
                anchor = Some(ix);
            } else {
                break;
            }
        }
    }
    // Started before every retained row: it leads the window.
    timed.then(|| anchor.unwrap_or(0))
}

/// Most cleared children remembered per hub; the oldest marks go first.
pub const MAX_CLEARED: usize = 512;
pub const MAX_PROVIDER_CHILDREN: usize = 32;

/// Only explicit terminal statuses are safe to clear or deprioritize.
pub fn provider_terminal(status: &str) -> bool {
    matches!(
        status,
        "complete" | "completed" | "done" | "stopped" | "ended" | "failed" | "error" | "lost"
    )
}

/// Positive evidence of live work or a pending decision, not an unknown replay.
pub fn provider_active(status: &str) -> bool {
    matches!(
        status,
        "running"
            | "responding"
            | "streaming"
            | "working"
            | "thinking"
            | "background"
            | "approval"
            | "question"
            | "waiting_approval"
            | "waiting_input"
    )
}

/// A finished child the user cleared from the sidebar. Only a device-local
/// visibility mark: nothing is stopped, closed, archived or forgotten, and the
/// session, transcript and history stay where they were. The work evidence
/// seen when clearing separates a replay of that same finish (stays cleared)
/// from the child working again (shown again).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClearMark {
    pub cleared_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub tool_calls: Option<u64>,
}
impl ClearMark {
    /// A provider-native run is its start and finish. Tool counts are left
    /// out: artifact scans backfill them after the finish.
    pub fn provider(child: &ChildAgent, now_ms: i64) -> Self {
        Self {
            cleared_at_ms: now_ms,
            started_at_ms: child.telemetry.started_at_ms,
            completed_at_ms: child.telemetry.completed_at_ms,
            tool_calls: None,
        }
    }
    /// A session's timestamps move with non-work updates; its tool count only
    /// moves when it works.
    pub fn session(session: &Session, now_ms: i64) -> Self {
        Self {
            cleared_at_ms: now_ms,
            tool_calls: session.telemetry.tool_calls,
            ..Default::default()
        }
    }
    /// `current` still describes the work this mark was taken on. A value
    /// unknown on either side is not evidence of new work, so late telemetry
    /// for the same finish never brings a cleared child back.
    pub fn covers(&self, current: &Self) -> bool {
        fn newer<T: PartialOrd>(marked: Option<T>, now: Option<T>) -> bool {
            matches!((marked, now), (Some(marked), Some(now)) if now > marked)
        }
        !(newer(self.started_at_ms, current.started_at_ms)
            || newer(self.completed_at_ms, current.completed_at_ms)
            || newer(self.tool_calls, current.tool_calls))
    }
}
pub fn clear_key_provider(parent: &str, agent: &str) -> String {
    format!("agent:{}", serde_json::json!([parent, agent]))
}
pub fn clear_key_session(id: &str) -> String {
    format!("session:{id}")
}
/// A Workspacer child session that has finished its work: back at its prompt
/// or ended, nothing pending, and none of its own provider-native children
/// still running.
pub fn session_finished(session: &Session) -> bool {
    matches!(
        session.state.as_str(),
        "input" | "idle" | "stopped" | "ended"
    ) && session.approval.is_none()
        && session.questions.is_none()
        && !session
            .subagents
            .as_array()
            .into_iter()
            .flatten()
            .any(|child| !provider_terminal(child["status"].as_str().unwrap_or("")))
}
/// Remember a clear, keeping the newest `MAX_CLEARED`.
pub fn remember_clear(marks: &mut BTreeMap<String, ClearMark>, key: String, mark: ClearMark) {
    marks.insert(key, mark);
    while marks.len() > MAX_CLEARED {
        let Some(oldest) = marks
            .iter()
            .min_by_key(|(_, mark)| mark.cleared_at_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        marks.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Tool;
    #[test]
    fn overview_pins_children_where_they_started_and_reports_settling() {
        let row = |at: Option<i64>| Row {
            timestamp_ms: at,
            ..Default::default()
        };
        let rows = [row(Some(10)), row(None), row(Some(20)), row(Some(30))];
        let child = |start: Option<i64>| {
            let mut c = ChildAgent::default();
            c.telemetry.started_at_ms = start;
            c
        };
        assert_eq!(overview_anchor(&child(Some(25)), &rows), Some(2));
        assert_eq!(overview_anchor(&child(Some(20)), &rows), Some(2));
        assert_eq!(overview_anchor(&child(Some(99)), &rows), Some(3));
        assert_eq!(overview_anchor(&child(Some(5)), &rows), Some(0));
        assert_eq!(overview_anchor(&child(None), &rows), None);
        assert_eq!(overview_anchor(&child(Some(5)), &[row(None)]), None);
        let mut c = ChildAgent {
            status: "running".into(),
            ..Default::default()
        };
        assert!(!c.settled());
        c.status = "completed".into();
        assert!(c.settled());
        c.status = "input".into();
        assert!(
            !c.settled(),
            "a native child at input is not known to be done"
        );
        c.session_id = Some("s".into());
        assert!(
            c.settled(),
            "a Workspacer child back at its prompt finished its turn"
        );
    }
    use serde_json::json;
    #[test]
    fn card_provenance_distinguishes_provider_dispatch_and_workspacer_receipts() {
        for name in ["Agent", "Task", "functions.spawn_agent", "codex_agent"] {
            let tool = Tool {
                name: name.into(),
                ..Default::default()
            };
            assert_eq!(tool_kind(&tool), Some(ChildKind::Provider));
        }
        let mut tool = Tool {
            name: "spawn_agent".into(),
            ..Default::default()
        };
        assert_eq!(tool_kind(&tool), None, "pending bare names are ambiguous");
        tool.name = "mcp__workspacer__spawn_agent".into();
        assert_eq!(tool_kind(&tool), Some(ChildKind::Workspacer));
        tool.name = "spawn_agent".into();
        tool.complete = true;
        tool.output = json!({"sessionId":"child"}).to_string();
        assert_eq!(tool_kind(&tool), Some(ChildKind::Workspacer));
    }

    #[test]
    fn artifact_unknown_start_does_not_become_epoch_duration() {
        let mut telemetry = Telemetry::default();
        telemetry.merge(&json!({"startedAt":0,"completedAt":4000}));
        let child = ChildAgent {
            status: "complete".into(),
            telemetry,
            ..Default::default()
        };
        assert_eq!(child.duration_ms(9000), None);
    }
    fn call(id: &str, name: &str, output: &str) -> Row {
        Row {
            timestamp_ms: Some(1000),
            tool: Some(Tool {
                id: id.into(),
                name: name.into(),
                output: output.into(),
                complete: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    #[test]
    fn exact_keys_namespace_and_multiple_agents_share_one_tool() {
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"parent","provider":"codex","subagents":[
            {"id":"native-id","toolUseId":"collab","status":"running","tokens":0,"startedAt":1000},
            {"id":"second","toolUseId":"collab","status":"complete","startedAt":1000,"completedAt":3000},
            {"id":"missing","toolUseId":"absent","status":"running"}
        ]}));
        let rows = [
            call("collab", "spawn_agent", "{\"agent_id\":\"native-id\"}"),
            call(
                "managed",
                "mcp__workspacer__spawn_agent",
                "{\"sessionId\":\"child\"}",
            ),
        ];
        let mut child = Session::default();
        child.merge(&json!({"sessionId":"child","parentSessionId":"parent","mode":"responding","usage":{"inputTokens":10,"outputTokens":20,"costUSD":0.01}}));
        let result = project(&parent, &[child], rows.iter());
        assert_eq!(result.by_tool["collab"].len(), 2);
        assert_eq!(result.by_tool["collab"][0].id, "native-id");
        assert_eq!(result.by_tool["collab"][0].session_id, None);
        assert_eq!(result.by_tool["collab"][0].telemetry.tokens, Some(0));
        assert_eq!(result.by_tool["collab"][0].duration_ms(4000), Some(3000));
        assert_eq!(result.by_tool["collab"][1].duration_ms(9000), Some(2000));
        assert_eq!(result.by_tool["managed"][0].telemetry.tokens, Some(30));
        assert_eq!(
            result.by_tool["managed"][0].session_id.as_deref(),
            Some("child")
        );
        assert_eq!(result.unanchored[0].id, "missing");
    }
    #[test]
    fn sparse_projection_preserves_numbers_and_does_not_copy_nested_payloads() {
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"parent","started_at":"2026-10-01T00:00:00Z","usage":{"contextTokens":42,"costUSD":0},"subagents":[{"id":"a","tokens":23,"costUSD":0,"startedAt":1000,"lastToolSummary":"reading","conversation":{"huge":"data"}}]}));
        assert_eq!(parent.telemetry.tokens, None);
        assert_eq!(parent.telemetry.cost_usd, Some(0.));
        assert!(parent.telemetry.started_at_ms.is_some());
        parent.merge(&json!({"subagents":[{"id":"a","status":"complete","completedAt":3000}]}));
        let result = project(&parent, &[], std::iter::empty());
        assert_eq!(result.unanchored[0].telemetry.tokens, Some(23));
        assert_eq!(result.unanchored[0].duration_ms(4000), Some(2000));
        assert!(parent.subagents[0].get("conversation").is_none());
        parent.merge(&json!({"subagents":[]}));
        assert!(
            project(&parent, &[], std::iter::empty())
                .unanchored
                .is_empty()
        );
    }
    #[test]
    fn explicit_missing_anchor_and_ambiguous_order_never_move_to_another_call() {
        let mut parent = Session::default();
        parent.merge(&json!({"provider":"claude","subagents":[{"id":"a","toolUseId":"not-loaded","startedAt":2000},{"id":"b","startedAt":2000}]}));
        let rows = [call("one", "Agent", ""), call("two", "Agent", "")];
        let projected = project(&parent, &[], rows.iter());
        assert!(projected.by_tool.is_empty());
        assert_eq!(projected.unanchored.len(), 2);
    }
    #[test]
    fn status_line_totals_actual_model_and_nulls_follow_wire() {
        let mut child = Session::default();
        child.merge(&json!({"sessionId":"child", "model":"actual-top", "requestedSelection":{"model":"requested"}, "usage":{"model":"actual-runtime", "contextTokens":999}, "statusLine":{"totalInputTokens":20,"totalOutputTokens":30,"costUSD":0}}));
        assert_eq!(child.model, "requested");
        assert_eq!(from_session(&child).model, "actual-runtime");
        assert_eq!(child.telemetry.tokens, Some(50));
        assert_eq!(child.telemetry.cost_usd, Some(0.));
        child.merge(&json!({"status_line":{"total_input_tokens":25}}));
        assert_eq!(child.telemetry.tokens, Some(55));
        child.merge(&json!({"statusLine":{"totalOutputTokens":null,"costUSD":null}}));
        assert_eq!(child.telemetry.tokens, None);
        assert_eq!(child.telemetry.cost_usd, None);
        child.merge(&json!({"status_line":{"total_input_tokens":0,"total_output_tokens":0,"cost_usd":0},"completedAt":9000,"durationMs":8000}));
        assert_eq!(child.telemetry.tokens, Some(0));
        child.merge(&json!({"mode":"responding","startedAt":10000}));
        assert_eq!(child.telemetry.completed_at_ms, None);
        assert_eq!(child.telemetry.duration_ms, None);
        child.merge(&json!({"startedAt":null,"usage":{"model":null}}));
        assert_eq!(child.telemetry.started_at_ms, None);
        assert_eq!(from_session(&child).model, "");
    }
    #[test]
    fn duplicates_nonspawn_and_empty_tool_ids_do_not_anchor() {
        let mut parent = Session::default();
        parent.merge(&json!({"subagents":[{"id":"same","toolUseId":"read"},{"id":"same","toolUseId":"spawn"}]}));
        let rows = [call("read", "Read", ""), call("", "Agent", "")];
        let projected = project(&parent, &[], rows.iter());
        assert!(projected.by_tool.is_empty());
        assert_eq!(projected.unanchored.len(), 1);
    }
    #[test]
    fn durations_reject_reversed_and_extreme_timestamps_without_overflow() {
        let mut child = ChildAgent {
            status: "running".into(),
            telemetry: Telemetry {
                started_at_ms: Some(i64::MIN),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(child.duration_ms(i64::MAX), None);
        child.telemetry.started_at_ms = Some(2000);
        assert_eq!(child.duration_ms(1000), None);
        assert_eq!(child.duration_ms(2000), Some(0));
        child.status = "complete".into();
        child.telemetry.completed_at_ms = Some(3000);
        assert_eq!(child.duration_ms(i64::MAX), Some(1000));
    }

    #[test]
    fn terminal_managed_duration_uses_reported_activity_but_idle_does_not() {
        let mut session = Session::default();
        session.merge(&json!({"session_id":"child","mode":"stopped","status":"ended", "started_at":"2026-10-01T00:00:00Z", "updated_at":"2026-10-01T00:00:03Z"}));
        assert_eq!(from_session(&session).duration_ms(i64::MAX), Some(3000));
        session.merge(&json!({"mode":"input","status":"active"}));
        assert_eq!(from_session(&session).duration_ms(i64::MAX), None);
        let provider = ChildAgent {
            status: "complete".into(),
            telemetry: session.telemetry.clone(),
            ..Default::default()
        };
        assert_eq!(provider.duration_ms(i64::MAX), None);
        session.merge(&json!({"mode":"stopped","started_at":"2026-10-01T00:00:05Z"}));
        assert_eq!(from_session(&session).duration_ms(i64::MAX), None);
    }

    #[test]
    fn clear_marks_survive_replays_and_late_telemetry_but_not_new_work() {
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"p","subagents":[{"id":"a","status":"complete","startedAt":1000,"completedAt":2000}]}));
        let child = |parent: &Session| {
            project(parent, &[], std::iter::empty())
                .unanchored
                .remove(0)
        };
        let mark = ClearMark::provider(&child(&parent), 5000);
        assert_eq!(
            (mark.started_at_ms, mark.completed_at_ms),
            (Some(1000), Some(2000))
        );
        // The same finish replayed, or backfilled with tool counts and usage.
        parent.merge(&json!({"subagents":[{"id":"a","status":"complete","startedAt":1000,"completedAt":2000,"toolCalls":7,"tokens":9}]}));
        assert!(mark.covers(&ClearMark::provider(&child(&parent), 6000)));
        // Reused for a second run: a later finish is new work.
        parent.merge(&json!({"subagents":[{"id":"a","status":"complete","startedAt":1000,"completedAt":9000}]}));
        assert!(!mark.covers(&ClearMark::provider(&child(&parent), 9500)));
        // A finish time that was unknown when cleared stays the same finish.
        let unknown = ClearMark {
            completed_at_ms: None,
            ..mark.clone()
        };
        assert!(unknown.covers(&ClearMark::provider(&child(&parent), 9500)));

        let mut session = Session::default();
        session.merge(&json!({"sessionId":"c","mode":"input","totalToolCalls":3}));
        let mark = ClearMark::session(&session, 1);
        session.merge(&json!({"lastActivity":99,"updated_at":"2026-10-05T00:00:00Z"}));
        assert!(
            mark.covers(&ClearMark::session(&session, 2)),
            "activity alone is not work"
        );
        session.merge(&json!({"totalToolCalls":4}));
        assert!(!mark.covers(&ClearMark::session(&session, 2)));
    }

    #[test]
    fn finished_sessions_exclude_pending_and_running_children() {
        let mut session = Session::default();
        session.merge(&json!({"sessionId":"c","mode":"input"}));
        assert!(session_finished(&session));
        session.merge(&json!({"subagents":[{"id":"a","status":"running"}]}));
        assert!(!session_finished(&session), "its own child still works");
        session.merge(
            &json!({"subagents":[{"id":"a","status":"complete"}],"pendingApproval":{"id":"x"}}),
        );
        assert!(!session_finished(&session));
        session.merge(&json!({"pendingApproval":null,"mode":"responding"}));
        assert!(!session_finished(&session));
        session.merge(&json!({"mode":"stopped"}));
        assert!(session_finished(&session));
    }

    #[test]
    fn remembered_clears_are_bounded_oldest_first() {
        let mut marks = BTreeMap::new();
        for at in 0..(MAX_CLEARED as i64 + 3) {
            remember_clear(
                &mut marks,
                clear_key_session(&format!("s{at}")),
                ClearMark {
                    cleared_at_ms: at,
                    ..Default::default()
                },
            );
        }
        assert_eq!(marks.len(), MAX_CLEARED);
        assert!(!marks.contains_key("session:s0") && !marks.contains_key("session:s2"));
        assert!(marks.contains_key("session:s3"));
        assert_ne!(clear_key_provider("p", "a"), clear_key_session("p/a"));
    }

    #[test]
    fn past_the_bound_the_newest_children_are_kept() {
        let mut children: Vec<_> = (0..40)
            .map(|ix| json!({"id":format!("c{ix}"),"status":"complete"}))
            .collect();
        children[39]["status"] = json!("running");
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"p","subagents":children}));
        let ids: Vec<_> = project(&parent, &[], std::iter::empty())
            .unanchored
            .into_iter()
            .map(|child| child.id)
            .collect();
        assert_eq!(ids.len(), 32);
        assert_eq!(ids.first().map(String::as_str), Some("c8"));
        assert_eq!(
            ids.last().map(String::as_str),
            Some("c39"),
            "the running child stays"
        );
    }
    #[test]
    fn inventory_keeps_old_running_and_approval_children_ahead_of_new_finishes() {
        let mut children: Vec<_> = (0..40)
            .map(|ix| json!({"id":format!("c{ix}"),"status":"complete"}))
            .collect();
        children[0]["status"] = json!("running");
        children[1]["status"] = json!("waiting_approval");
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"p","mode":"input","subagents":children}));
        let projected = project(&parent, &[], std::iter::empty());
        let ids: Vec<_> = projected.unanchored.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), MAX_PROVIDER_CHILDREN);
        assert_eq!(&ids[..2], &["c0", "c1"]);
        assert_eq!(ids.last(), Some(&"c39"));
        assert!(!session_finished(&parent));
        parent.merge(&json!({"subagents":[{"id":"approval","status":"waiting_approval"}]}));
        assert!(
            !session_finished(&parent),
            "pending native approval is not finished work"
        );
    }

    #[test]
    fn explicit_older_run_and_finish_replays_do_not_regress_a_provider_child() {
        let mut parent = Session::default();
        parent.merge(&json!({"sessionId":"p","subagents":[{"id":"a","status":"complete","startedAt":3000,"completedAt":6000}]}));
        parent.merge(&json!({"subagents":[{"id":"a","status":"running","startedAt":1000}]}));
        assert_eq!(parent.subagents[0]["status"], "complete");
        parent.merge(&json!({"subagents":[{"id":"a","status":"complete","startedAt":3000,"completedAt":4000}]}));
        assert_eq!(parent.subagents[0]["completedAt"], 6000);
        // A fresh explicit run is not held terminal by the previous one.
        parent.merge(&json!({"subagents":[{"id":"a","status":"running","startedAt":7000}]}));
        assert_eq!(parent.subagents[0]["status"], "running");
        assert!(parent.subagents[0]["completedAt"].is_null());
        assert_ne!(
            clear_key_provider("a/b", "c"),
            clear_key_provider("a", "b/c")
        );
    }
}
