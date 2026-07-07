//! Wire and domain types, mirroring claudemon's REST API (`GET /sessions`,
//! `/sessions/:id`, `/sessions/:id/transcript`). claudemon is the source of
//! truth for a standalone TUI — the hub-bus capabilities the `/remote` client
//! uses are registered by the Electron app and absent when it isn't running.

use serde::Deserialize;
use serde_json::Value;

/// Token/cost/context usage for a session, as returned by claudemon's
/// `GET /sessions` (and `GET /sessions/:id`) in the additive `usage` field.
/// Fields are optional so older daemon versions that omit the block still
/// deserialize cleanly.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub model: Option<String>,
    /// Input side of the latest turn — a point-in-time view of context fullness.
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub context_limit: u64,
    /// Cumulative cost (USD) for the session.
    #[serde(default)]
    pub cost_usd: f64,
}

/// Claude's authoritative statusLine telemetry, streamed from claudemon's
/// `/statusline/stream`. Every field is optional (Claude omits some, and
/// rate-limit data only exists for Pro/Max accounts). Preferred over the
/// transcript-derived [`Usage`] when present — see [`derive_stats`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatusLine {
    #[serde(default)]
    pub model_display: Option<String>,
    /// `context_window.used_percentage` (0–100).
    #[serde(default)]
    pub context_used_pct: Option<f64>,
    /// Claude's own authoritative session cost.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// 5h rate-limit window used %, 0–100 (Pro/Max only).
    #[serde(default)]
    pub five_hour_pct: Option<f64>,
    /// 7d rate-limit window used %, 0–100 (Pro/Max only).
    #[serde(default)]
    pub seven_day_pct: Option<f64>,
}

/// Model / context-% / cost for a session, resolving claudemon's authoritative
/// statusLine first and falling back to transcript-derived [`Usage`] — the
/// terminal analogue of the desktop `deriveSessionStats` precedence.
#[derive(Debug, Default, Clone)]
pub struct DerivedStats {
    pub model: Option<String>,
    pub context_pct: Option<f64>,
    pub cost: Option<f64>,
}

pub fn derive_stats(agent: &Agent, sl: Option<&StatusLine>) -> DerivedStats {
    let model = sl
        .and_then(|s| s.model_display.clone())
        .or_else(|| agent.usage.as_ref().and_then(|u| u.model.clone()));
    let context_pct = sl.and_then(|s| s.context_used_pct).or_else(|| {
        agent.usage.as_ref().and_then(|u| {
            (u.context_limit > 0 && u.context_tokens > 0)
                .then(|| u.context_tokens as f64 / u.context_limit as f64 * 100.0)
        })
    });
    let cost = sl
        .and_then(|s| s.cost_usd)
        .filter(|c| *c > 0.0)
        .or_else(|| {
            agent
                .usage
                .as_ref()
                .map(|u| u.cost_usd)
                .filter(|c| *c > 0.0)
        });
    DerivedStats {
        model,
        context_pct,
        cost,
    }
}

/// The mode a Claude session can be in, mirroring claudemon's `SessionMode`.
///
/// Uses `#[serde(rename_all = "snake_case")]` to match the wire format claudemon
/// emits (e.g. `"input"`, `"approval"`). The `Unknown` catch-all variant absorbs
/// any future values so deserialization never fails on unknown modes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// No hook has fired yet, or the mode field was absent.
    #[default]
    Unknown,
    /// Chat prompt is up — waiting for the user to send a message.
    Input,
    /// Claude is actively producing a turn (streaming, thinking, or tool use).
    Responding,
    /// Paused waiting for a tool-permission yes/no/always decision.
    Approval,
    /// Claude asked the user a structured question via `AskUserQuestion`.
    Question,
    /// Session has ended.
    Stopped,
    /// A mode this client doesn't recognise yet — forward-compat catch-all.
    #[serde(other)]
    Other,
}

/// Serde default for [`Agent::provider`] — claudemon's un-managed PTY path is
/// Claude, and older daemons omit the field entirely, so an absent value means
/// Claude. Mirrors claudemon's own `default_provider`.
fn default_provider() -> String {
    "claude".to_string()
}

/// Serde default for [`Agent::transport`] — sessions are PTY-backed unless the
/// daemon says otherwise (it serializes `"stream"` for headless stream-json
/// sessions; older daemons omit the field entirely).
fn default_transport() -> String {
    "pty".to_string()
}

/// One live session, as returned by claudemon's `GET /sessions`.
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Which agent backend owns this session, straight from claudemon's session
    /// list (`"claude"` | `"codex"` | `"opencode"` | `"pi"`). Authoritative over
    /// the TUI's local managed-spawn map — see [`App::provider_for`]. Serde
    /// defaults to `"claude"` so older daemons deserialize unchanged.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// How the daemon talks to the session: `"pty"` (a real terminal we can
    /// stream and type into) or `"stream"` (a headless stream-json adapter —
    /// no PTY exists, so the TUI renders the transcript only). Serde defaults
    /// to `"pty"` so older daemons deserialize unchanged.
    #[serde(default = "default_transport")]
    pub transport: String,
    /// The current session mode. Defaults to `AgentMode::Unknown` when the
    /// field is absent, and falls back to `AgentMode::Other` for unrecognised
    /// values so deserialization never panics on future daemon versions.
    #[serde(default)]
    pub mode: AgentMode,
    /// What Claude is blocked on, if anything. `skip_deserializing` on the
    /// daemon means it can be absent; we tolerate that.
    #[serde(default)]
    pub pending: Option<Pending>,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub last_event: Option<String>,
    /// Token/cost/context/model as returned by claudemon's `/sessions` response.
    /// Absent when the daemon hasn't computed it yet (no assistant turns).
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Whatever Claude is waiting on, tagged by `kind` (matches claudemon's enum).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    Approval {
        #[serde(default)]
        tool: Option<String>,
        /// The raw permission-request hook payload, shown so the user can see
        /// exactly what the tool would do before approving.
        #[serde(default)]
        raw: Value,
    },
    Question {
        #[serde(default)]
        questions: Vec<Question>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Question {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A content fingerprint of a pending question set, so stepper state
/// ([`crate::app::QuestionFlow`]) can detect when the set it tracks was
/// superseded by a *different* set of the same length — length alone would
/// silently carry old answers over to questions the user never saw.
pub fn question_fingerprint(qs: &[Question]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for q in qs {
        q.question.hash(&mut h);
        q.header.hash(&mut h);
        q.multi_select.hash(&mut h);
        for o in &q.options {
            o.label.hash(&mut h);
        }
        // Separator so option/question boundaries can't alias.
        u8::MAX.hash(&mut h);
    }
    h.finish()
}

impl Agent {
    pub fn state(&self) -> &str {
        match &self.mode {
            AgentMode::Unknown => "unknown",
            AgentMode::Input => "input",
            AgentMode::Responding => "responding",
            AgentMode::Approval => "approval",
            AgentMode::Question => "question",
            AgentMode::Stopped => "stopped",
            AgentMode::Other => "other",
        }
    }

    /// True when the agent needs the user: an approval, a question, or a chat
    /// prompt awaiting the next message (matches the `/remote` semantics).
    pub fn is_waiting(&self) -> bool {
        matches!(
            self.mode,
            AgentMode::Input | AgentMode::Approval | AgentMode::Question
        )
    }

    /// True when the agent is actively producing a turn.
    pub fn is_busy(&self) -> bool {
        self.mode == AgentMode::Responding
    }

    /// True when the session has ended.
    pub fn is_stopped(&self) -> bool {
        self.mode == AgentMode::Stopped
    }

    /// True for headless stream-transport sessions — no PTY exists to attach,
    /// so the chat view is transcript-only.
    pub fn is_stream(&self) -> bool {
        self.transport == "stream"
    }

    pub fn cwd_str(&self) -> &str {
        self.cwd.as_deref().unwrap_or("")
    }

    /// Last path segments of the cwd — what the sidebar shows as a name.
    pub fn short_cwd(&self) -> String {
        let cwd = self.cwd_str();
        if cwd.is_empty() {
            return "(session)".into();
        }
        let parts: Vec<&str> = cwd.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
        if parts.len() <= 2 {
            cwd.to_string()
        } else {
            format!("…/{}", parts[parts.len() - 2..].join("/"))
        }
    }

    /// The pending approval as `(tool name, raw hook input)`, if any.
    pub fn approval(&self) -> Option<(&str, &Value)> {
        match &self.pending {
            Some(Pending::Approval { tool, raw, .. }) => {
                Some((tool.as_deref().unwrap_or("tool"), raw))
            }
            _ => None,
        }
    }

    pub fn questions(&self) -> Option<&[Question]> {
        match &self.pending {
            Some(Pending::Question { questions, .. }) => Some(questions),
            _ => None,
        }
    }

    pub fn has_question(&self) -> bool {
        self.questions().is_some_and(|q| !q.is_empty())
    }
}

/// One changed file from claudemon's `GET /git/status`. `staged`/`unstaged` are
/// the porcelain XY status codes (e.g. "M", "A", "D", "?", " ").
#[derive(Debug, Clone, Deserialize)]
pub struct FileStatus {
    pub path: String,
    #[serde(default)]
    pub orig_path: Option<String>,
    #[serde(default)]
    pub staged: String,
    #[serde(default)]
    pub unstaged: String,
}

impl FileStatus {
    /// Untracked files have no index/HEAD baseline — they diff as all-added.
    pub fn is_untracked(&self) -> bool {
        self.staged == "?" || self.unstaged == "?"
    }

    /// Display name: `orig → path` for renames/copies, else just `path`.
    pub fn display_path(&self) -> String {
        match &self.orig_path {
            Some(orig) => format!("{orig} → {}", self.path),
            None => self.path.clone(),
        }
    }
}

/// A rendered transcript turn — a role plus its text/tool parts, after the
/// noise (tool results, thinking, system reminders) has been filtered out.
#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    /// A tool call. `result` is the (truncated) tool output once it lands,
    /// prefixed with `error: ` when the tool failed. `edits` carries the
    /// `(old_string, new_string)` pairs of an Edit/MultiEdit call so the
    /// renderer can show a compact colored diff under the row.
    Tool {
        name: String,
        summary: String,
        result: Option<String>,
        edits: Vec<(String, String)>,
    },
}

/// Flatten a conversation's turns into searchable lines for content search:
/// non-empty text lines plus a compact `name summary result` line per tool.
/// Each line is clipped and the total is capped so a huge transcript can't blow
/// up the index.
pub fn search_lines(turns: &[Turn]) -> Vec<String> {
    const MAX_LINES: usize = 3000;
    const MAX_LEN: usize = 200;
    let clip = |s: &str| -> String { s.chars().take(MAX_LEN).collect() };
    let mut out = Vec::new();
    for t in turns {
        for p in &t.parts {
            match p {
                Part::Text(s) => {
                    for line in s.lines() {
                        let l = line.trim();
                        if !l.is_empty() {
                            out.push(clip(l));
                        }
                    }
                }
                Part::Tool {
                    name,
                    summary,
                    result,
                    ..
                } => {
                    let mut s = format!("{name} {summary}");
                    if let Some(r) = result {
                        s.push(' ');
                        s.push_str(r);
                    }
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(clip(s));
                    }
                }
            }
            if out.len() >= MAX_LINES {
                return out;
            }
        }
    }
    out
}

/// Parse claudemon's `/conversation` payload (`{ items: [...] }`) into renderable
/// turns. Items are a flat, `kind`-tagged stream (user_message / assistant_text
/// / tool_use / tool_result / usage); consecutive same-role items coalesce into
/// one turn, and a `tool_result` attaches back to its `tool_use` by id — so the
/// parsed view shows tool *output*, not just the call (richer than the old
/// transcript path).
///
/// `transport` is the session's wire transport (see [`Agent::transport`]). It
/// decides how consecutive `assistant_text` items coalesce into one text part:
/// stream sessions store per-token fragments, so they concatenate verbatim
/// (never trimming a fragment — the joined text is trimmed once at the end);
/// PTY sessions store whole blocks, joined with a blank line.
///
/// A re-delivered `tool_use` id (transcript compaction repeats a call, resume
/// replays history) folds into nothing — ids are globally unique, so a second
/// occurrence is always a duplicate (mirror of the desktop conversationApplier
/// dedup).
pub fn turns_from_conversation(v: &Value, transport: &str) -> Vec<Turn> {
    use std::collections::{HashMap, HashSet};
    let stream = transport == "stream";
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    let mut turns: Vec<Turn> = Vec::new();
    // tool_use id → (turn index, part index), so a later result can attach.
    let mut tool_loc: HashMap<String, (usize, usize)> = HashMap::new();
    // Every tool_use id seen, for dropping re-delivered duplicates.
    let mut seen_tool_ids: HashSet<String> = HashSet::new();
    // Consecutive assistant_text fragments, coalesced into one part at flush.
    let mut text_buf: Vec<String> = Vec::new();

    // Append a part to the open turn, starting a new turn on a role change.
    fn push(turns: &mut Vec<Turn>, role: Role, part: Part) -> (usize, usize) {
        if turns.last().map(|t| t.role) != Some(role) {
            turns.push(Turn {
                role,
                parts: Vec::new(),
            });
        }
        let ti = turns.len() - 1;
        turns[ti].parts.push(part);
        (ti, turns[ti].parts.len() - 1)
    }

    // Join the buffered fragments into ONE text part. Stream fragments concat
    // verbatim (they're per-token slices of one message); PTY blocks join with
    // a blank line. The combined text is trimmed exactly once, at the end.
    fn flush_text(turns: &mut Vec<Turn>, text_buf: &mut Vec<String>, stream: bool) {
        if text_buf.is_empty() {
            return;
        }
        let joined = if stream {
            text_buf.concat()
        } else {
            text_buf.join("\n\n")
        };
        text_buf.clear();
        let text = joined.trim();
        if !text.is_empty() {
            push(turns, Role::Assistant, Part::Text(text.to_string()));
        }
    }

    for item in &items {
        match item.get("kind").and_then(|k| k.as_str()).unwrap_or("") {
            "user_message" => {
                flush_text(&mut turns, &mut text_buf, stream);
                let text = item
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .trim();
                if text.is_empty() || is_meta_noise(text) {
                    continue;
                }
                push(&mut turns, Role::User, Part::Text(text.to_string()));
            }
            "assistant_text" => {
                let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                if stream {
                    // Never trim or drop a fragment — whitespace-only tokens
                    // are the joining glue between words.
                    if !text.is_empty() {
                        text_buf.push(text.to_string());
                    }
                } else {
                    let block = text.trim();
                    if !block.is_empty() {
                        text_buf.push(block.to_string());
                    }
                }
            }
            "tool_use" => {
                // Dedup by id first, so a replayed call vanishes without even
                // breaking the coalescing of the text around it.
                if let Some(id) = item
                    .get("id")
                    .and_then(|i| i.as_str())
                    .filter(|s| !s.is_empty())
                {
                    if !seen_tool_ids.insert(id.to_string()) {
                        continue;
                    }
                }
                flush_text(&mut turns, &mut text_buf, stream);
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let summary = tool_summary(item.get("input"));
                let edits = edit_pairs(item.get("input"));
                let loc = push(
                    &mut turns,
                    Role::Assistant,
                    Part::Tool {
                        name,
                        summary,
                        result: None,
                        edits,
                    },
                );
                if let Some(id) = item
                    .get("id")
                    .and_then(|i| i.as_str())
                    .filter(|s| !s.is_empty())
                {
                    tool_loc.insert(id.to_string(), loc);
                }
            }
            "tool_result" => {
                flush_text(&mut turns, &mut text_buf, stream);
                let tid = item
                    .get("tool_use_id")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let content = item
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .trim();
                let is_error = item
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                if content.is_empty() {
                    continue;
                }
                if let Some(&(ti, pi)) = tool_loc.get(tid) {
                    if let Some(Part::Tool { result, .. }) =
                        turns.get_mut(ti).and_then(|t| t.parts.get_mut(pi))
                    {
                        let snippet = truncate(content, 200);
                        *result = Some(if is_error {
                            format!("error: {snippet}")
                        } else {
                            snippet
                        });
                    }
                }
            }
            "usage" => {
                // Ignored entirely — claudemon interleaves a usage item before
                // every PTY assistant row's text, so treating it as a boundary
                // would defeat the blank-line block coalescing above.
            }
            _ => {
                // Any future kinds — still a boundary between assistant
                // messages, so close any open text run.
                flush_text(&mut turns, &mut text_buf, stream);
            }
        }
    }
    flush_text(&mut turns, &mut text_buf, stream);
    turns
}

/// The `(old_string, new_string)` pairs of an Edit/MultiEdit input, if any:
/// a top-level `old_string`/`new_string` pair (Edit), plus every entry of an
/// `edits` array (MultiEdit). Empty for every other tool shape.
fn edit_pairs(input: Option<&Value>) -> Vec<(String, String)> {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let pair_of = |o: &serde_json::Map<String, Value>| -> Option<(String, String)> {
        let old = o.get("old_string").and_then(|v| v.as_str())?;
        let new = o.get("new_string").and_then(|v| v.as_str())?;
        (!old.is_empty() || !new.is_empty()).then(|| (old.to_string(), new.to_string()))
    };
    let mut out = Vec::new();
    if let Some(p) = pair_of(obj) {
        out.push(p);
    }
    if let Some(edits) = obj.get("edits").and_then(|e| e.as_array()) {
        out.extend(
            edits
                .iter()
                .filter_map(|e| e.as_object())
                .filter_map(pair_of),
        );
    }
    out
}

/// Slash-command echoes, injected reminders, and background-task notifications
/// (emitted by workflows) aren't real conversation.
fn is_meta_noise(text: &str) -> bool {
    const TAGS: [&str; 5] = [
        "<local-command",
        "<command-name",
        "<command-message",
        "<system-reminder",
        "<task-notification",
    ];
    TAGS.iter().any(|t| text.starts_with(t))
}

/// A one-line gist of a tool call, drawn from whichever well-known field is
/// present.
fn tool_summary(input: Option<&Value>) -> String {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return String::new();
    };
    const KEYS: [&str; 8] = [
        "file_path",
        "path",
        "command",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
    ];
    for k in KEYS {
        if let Some(s) = obj.get(k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return truncate(s, 64);
            }
        }
    }
    String::new()
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_sessions_list_shape() {
        // Exactly what claudemon's GET /sessions returns (with the new usage block).
        let json = serde_json::json!([{
            "session_id": "abc",
            "cwd": "/home/u/proj",
            "mode": "responding",
            "pending": null,
            "started_at": "2026-06-04T03:00:00Z",
            "updated_at": "2026-06-04T03:00:10Z",
            "tool_calls": 3,
            "last_event": "PreToolUse",
            "usage": {
                "model": "claude-sonnet-4-6",
                "context_tokens": 5200,
                "context_limit": 200000,
                "cost_usd": 0.042
            }
        }]);
        let agents: Vec<Agent> = serde_json::from_value(json).unwrap();
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.session_id, "abc");
        assert_eq!(a.state(), "responding");
        assert!(a.is_busy() && !a.is_waiting());
        assert_eq!(a.short_cwd(), "…/u/proj");
        assert!(a.approval().is_none() && a.questions().is_none());
        // usage comes straight from the API now — no transcript fetch needed.
        let u = a.usage.as_ref().expect("usage present");
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(u.context_tokens, 5200);
        assert_eq!(u.context_limit, 200_000);
        assert!((u.cost_usd - 0.042).abs() < 1e-9);
    }

    #[test]
    fn parses_sessions_list_without_usage() {
        // Daemons that don't yet emit the usage block must still deserialize.
        let json = serde_json::json!([{
            "session_id": "abc",
            "mode": "responding",
        }]);
        let agents: Vec<Agent> = serde_json::from_value(json).unwrap();
        assert!(agents[0].usage.is_none());
    }

    #[test]
    fn provider_wire_field_parses_and_defaults_to_claude() {
        // Explicit provider is carried through verbatim.
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "x", "mode": "input", "provider": "codex"
        }))
        .unwrap();
        assert_eq!(a.provider, "codex");

        // Absent provider (older daemon / un-managed PTY) defaults to "claude".
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "y", "mode": "input"
        }))
        .unwrap();
        assert_eq!(a.provider, "claude");
    }

    #[test]
    fn derive_stats_prefers_statusline_then_falls_back_to_usage() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "claude-sonnet-4-6", "context_tokens": 50_000,
                       "context_limit": 200_000, "cost_usd": 1.0 }
        }))
        .unwrap();

        // No statusLine → transcript usage fallback (25% ctx, $1.00).
        let d = derive_stats(&agent, None);
        assert_eq!(d.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(d.context_pct, Some(25.0));
        assert_eq!(d.cost, Some(1.0));

        // statusLine present → its authoritative values win.
        let sl = StatusLine {
            model_display: Some("Opus 4.8".into()),
            context_used_pct: Some(73.0),
            cost_usd: Some(12.5),
            ..Default::default()
        };
        let d = derive_stats(&agent, Some(&sl));
        assert_eq!(d.model.as_deref(), Some("Opus 4.8"));
        assert_eq!(d.context_pct, Some(73.0));
        assert_eq!(d.cost, Some(12.5));
    }

    #[test]
    fn conversation_groups_turns_and_attaches_tool_results() {
        let v = serde_json::json!({ "items": [
            { "kind": "user_message", "text": "do it" },
            { "kind": "assistant_text", "text": "on it" },
            { "kind": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_result", "tool_use_id": "t1", "content": "a\nb\nc", "is_error": false },
            { "kind": "usage", "usage": {} },
            { "kind": "tool_use", "id": "t2", "name": "Edit", "input": {"file_path": "/x.rs"} },
            { "kind": "tool_result", "tool_use_id": "t2", "content": "boom", "is_error": true },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(
            turns.len(),
            2,
            "one user turn, one coalesced assistant turn"
        );
        assert_eq!(turns[0].role, Role::User);
        assert_eq!(turns[1].role, Role::Assistant);
        // assistant turn: text + 2 tools (usage skipped, results attached, not parts)
        let parts = &turns[1].parts;
        assert_eq!(parts.len(), 3);
        match &parts[1] {
            Part::Tool { name, result, .. } => {
                assert_eq!(name, "Bash");
                assert_eq!(result.as_deref(), Some("a\nb\nc"));
            }
            other => panic!("expected Bash tool, got {other:?}"),
        }
        match &parts[2] {
            Part::Tool { name, result, .. } => {
                assert_eq!(name, "Edit");
                assert_eq!(result.as_deref(), Some("error: boom"), "errors prefixed");
            }
            other => panic!("expected Edit tool, got {other:?}"),
        }
    }

    #[test]
    fn conversation_filters_injected_meta_user_text() {
        let v = serde_json::json!({ "items": [
            { "kind": "user_message", "text": "<system-reminder>noise</system-reminder>" },
            { "kind": "user_message", "text": "real" },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns.len(), 1);
        assert!(matches!(&turns[0].parts[0], Part::Text(t) if t == "real"));
    }

    #[test]
    fn transport_wire_field_parses_and_defaults_to_pty() {
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "x", "mode": "input", "transport": "stream"
        }))
        .unwrap();
        assert_eq!(a.transport, "stream");
        assert!(a.is_stream());

        // Absent transport (older daemon / PTY session) defaults to "pty".
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "y", "mode": "input"
        }))
        .unwrap();
        assert_eq!(a.transport, "pty");
        assert!(!a.is_stream());
    }

    #[test]
    fn redelivered_tool_use_dedups_by_id() {
        // Transcript compaction / resume replays re-deliver the same call; ids
        // are globally unique, so the second occurrence folds into nothing.
        let v = serde_json::json!({ "items": [
            { "kind": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_result", "tool_use_id": "toolu_1", "content": "ok" },
            { "kind": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/f"} },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].parts.len(), 2, "the duplicate call is dropped");
        assert!(matches!(&turns[0].parts[0], Part::Tool { name, .. } if name == "Bash"));
        assert!(matches!(&turns[0].parts[1], Part::Tool { name, .. } if name == "Read"));
    }

    #[test]
    fn stream_transport_concats_fragments_verbatim() {
        // Stream sessions store per-token fragments; they must join into ONE
        // text part with no per-fragment trimming ("Hello" + " " + "world").
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "Hel" },
            { "kind": "assistant_text", "text": "lo" },
            { "kind": "assistant_text", "text": " " },
            { "kind": "assistant_text", "text": "world" },
            { "kind": "assistant_text", "text": "\n" },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].parts.len(), 1, "fragments coalesce into one part");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "Hello world"),
            "verbatim concat, trimmed once at the end: {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn stream_fragments_split_on_intervening_items() {
        // A tool call between text runs is a real boundary — two text parts.
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "first " },
            { "kind": "assistant_text", "text": "message" },
            { "kind": "tool_use", "id": "t1", "name": "Bash", "input": {} },
            { "kind": "assistant_text", "text": "second" },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        let parts = &turns[0].parts;
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], Part::Text(t) if t == "first message"));
        assert!(matches!(&parts[2], Part::Text(t) if t == "second"));
    }

    #[test]
    fn pty_transport_joins_whole_blocks_with_a_blank_line() {
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "block one\n" },
            { "kind": "assistant_text", "text": "  block two  " },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns[0].parts.len(), 1, "blocks coalesce into one part");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "block one\n\nblock two"),
            "got {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn usage_items_do_not_break_pty_block_coalescing() {
        // Real PTY conversations interleave a usage item before every
        // assistant row's text; it's ignored, not a text-run boundary.
        let v = serde_json::json!({ "items": [
            { "kind": "usage", "usage": {"input_tokens": 1} },
            { "kind": "assistant_text", "text": "para one" },
            { "kind": "usage", "usage": {"input_tokens": 2} },
            { "kind": "assistant_text", "text": "para two" },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns[0].parts.len(), 1, "blocks coalesce across usage");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "para one\n\npara two"),
            "got {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn edit_and_multiedit_inputs_carry_diff_pairs() {
        let v = serde_json::json!({ "items": [
            { "kind": "tool_use", "id": "e1", "name": "Edit",
              "input": {"file_path": "/a.rs", "old_string": "foo", "new_string": "bar"} },
            { "kind": "tool_use", "id": "e2", "name": "MultiEdit",
              "input": {"file_path": "/b.rs", "edits": [
                  {"old_string": "x", "new_string": "y"},
                  {"old_string": "p", "new_string": "q"}
              ]} },
            { "kind": "tool_use", "id": "e3", "name": "Bash", "input": {"command": "ls"} },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        let parts = &turns[0].parts;
        match &parts[0] {
            Part::Tool { edits, .. } => assert_eq!(edits, &[("foo".into(), "bar".into())]),
            other => panic!("expected Edit tool, got {other:?}"),
        }
        match &parts[1] {
            Part::Tool { edits, .. } => assert_eq!(
                edits,
                &[("x".to_string(), "y".to_string()), ("p".into(), "q".into())]
            ),
            other => panic!("expected MultiEdit tool, got {other:?}"),
        }
        match &parts[2] {
            Part::Tool { edits, .. } => assert!(edits.is_empty(), "Bash carries no diff"),
            other => panic!("expected Bash tool, got {other:?}"),
        }
    }

    #[test]
    fn derive_stats_empty_when_nothing_known() {
        let agent: Agent =
            serde_json::from_value(serde_json::json!({ "session_id": "s" })).unwrap();
        let d = derive_stats(&agent, None);
        assert!(d.model.is_none() && d.context_pct.is_none() && d.cost.is_none());
    }

    #[test]
    fn parses_pending_approval() {
        let json = serde_json::json!({
            "session_id": "x", "mode": "approval",
            "pending": {"kind": "approval", "tool": "Bash",
                        "raw": {"tool_input": {"command": "ls -la"}}}
        });
        let a: Agent = serde_json::from_value(json).unwrap();
        assert!(a.is_waiting());
        let (tool, raw) = a.approval().expect("approval present");
        assert_eq!(tool, "Bash");
        assert_eq!(raw["tool_input"]["command"], "ls -la");
    }

    #[test]
    fn parses_pending_question() {
        let json = serde_json::json!({
            "session_id": "x", "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Which?", "header": "Pick", "multi_select": false,
                 "options": [{"label": "A"}, {"label": "B", "description": "the b one"}]}
            ]}
        });
        let a: Agent = serde_json::from_value(json).unwrap();
        assert!(a.has_question());
        let qs = a.questions().unwrap();
        assert_eq!(qs[0].options.len(), 2);
        assert_eq!(qs[0].options[1].description.as_deref(), Some("the b one"));
    }

    // ── agent-mode contract characterization ────────────────────────────────
    // These tests pin the is_waiting / is_busy / state() behavior for every
    // mode that claudemon can emit, now that mode is a typed AgentMode enum.

    fn agent_with_mode(mode: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": "test",
            "mode": mode
        }))
        .unwrap()
    }

    /// "input" — user's turn to type the next message.
    #[test]
    fn mode_input_is_waiting_not_busy() {
        let a = agent_with_mode("input");
        assert!(a.is_waiting(), "input must be waiting");
        assert!(!a.is_busy(), "input must not be busy");
        assert_eq!(a.state(), "input");
    }

    /// "approval" — Claude wants to run a tool and needs a y/n.
    #[test]
    fn mode_approval_is_waiting_not_busy() {
        let a = agent_with_mode("approval");
        assert!(a.is_waiting(), "approval must be waiting");
        assert!(!a.is_busy(), "approval must not be busy");
        assert_eq!(a.state(), "approval");
    }

    /// "question" — Claude asked the user a structured question.
    #[test]
    fn mode_question_is_waiting_not_busy() {
        let a = agent_with_mode("question");
        assert!(a.is_waiting(), "question must be waiting");
        assert!(!a.is_busy(), "question must not be busy");
        assert_eq!(a.state(), "question");
    }

    /// "responding" — Claude is actively generating a turn.
    #[test]
    fn mode_responding_is_busy_not_waiting() {
        let a = agent_with_mode("responding");
        assert!(a.is_busy(), "responding must be busy");
        assert!(!a.is_waiting(), "responding must not be waiting");
        assert_eq!(a.state(), "responding");
    }

    /// "stopped" — session is finished / Claude process exited.
    #[test]
    fn mode_stopped_is_neither_waiting_nor_busy() {
        let a = agent_with_mode("stopped");
        assert!(!a.is_waiting(), "stopped must not be waiting");
        assert!(!a.is_busy(), "stopped must not be busy");
        assert_eq!(a.state(), "stopped");
    }

    /// Absent mode field — #[serde(default)] yields AgentMode::Unknown, so
    /// state() returns "unknown".
    #[test]
    fn mode_empty_state_is_unknown() {
        let a: Agent = serde_json::from_value(serde_json::json!({"session_id": "t"})).unwrap();
        assert_eq!(
            a.state(),
            "unknown",
            "absent mode yields 'unknown' from state()"
        );
        assert!(!a.is_waiting());
        assert!(!a.is_busy());
    }

    /// An explicit empty string or an unrecognised mode (e.g. a future value
    /// the daemon emits) maps to AgentMode::Other via #[serde(other)], which
    /// state() renders as "other". It is neither waiting nor busy.
    #[test]
    fn mode_unknown_string_maps_to_other() {
        // Explicit empty string falls to #[serde(other)] => AgentMode::Other.
        let a_empty = agent_with_mode("");
        assert_eq!(a_empty.state(), "other");
        assert!(!a_empty.is_waiting());
        assert!(!a_empty.is_busy());

        // An arbitrary future mode string also maps to AgentMode::Other.
        let a = agent_with_mode("future_mode");
        assert_eq!(a.state(), "other");
        assert!(!a.is_waiting());
        assert!(!a.is_busy());
    }

    /// Exhaustive table confirming the three-way classification for all known
    /// daemon-emitted modes.  Each tuple: (mode, is_waiting, is_busy).
    #[test]
    fn mode_classification_table() {
        let cases: &[(&str, bool, bool)] = &[
            ("input", true, false),
            ("approval", true, false),
            ("question", true, false),
            ("responding", false, true),
            ("stopped", false, false),
        ];
        for (mode, want_waiting, want_busy) in cases {
            let a = agent_with_mode(mode);
            assert_eq!(
                a.is_waiting(),
                *want_waiting,
                "is_waiting mismatch for mode={mode:?}"
            );
            assert_eq!(
                a.is_busy(),
                *want_busy,
                "is_busy mismatch for mode={mode:?}"
            );
        }
    }
}
