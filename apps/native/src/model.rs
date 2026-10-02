use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_ROWS: usize = 2_000;
pub const MAX_TRANSCRIPT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ROW_BYTES: usize = 64 * 1024;

/// Deliberately project only sidebar/control fields: rich snapshots may carry
/// megabytes of conversation, which belongs to the selected transcript alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub id: String,
    pub label: String,
    pub cwd: String,
    pub state: String,
    pub transport: String,
    pub provider: String,
    pub parent_session_id: String,
    pub skills: Value,
    pub subagents: Value,
    pub workflows: Value,
    pub model: String,
    /// Resolved model the runtime reports (`claude-opus-5-5`); never cleared
    /// by snapshots that omit it, unlike the selection alias in `model`.
    pub runtime_model: String,
    pub context_window: Option<u64>,
    pub context: ContextUsage,
    pub telemetry: crate::child_agents::Telemetry,
    pub approval: Option<Value>,
    pub questions: Option<Value>,
}

/// Context-window occupancy as the runtime reports it: the status line's
/// percentage/window pair, transcript-derived tokens held, and the daemon's
/// resolved window. Each source is replaced whole when a snapshot carries it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextUsage {
    pub status_pct: Option<f64>,
    pub status_window: Option<u64>,
    /// The provider knows its window but not current-request usage.
    pub waiting: bool,
    pub held_tokens: u64,
    pub usage_limit: Option<u64>,
    pub resolved_window: Option<u64>,
}

/// What the context meter shows; `pct` is 0–100.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextReading {
    pub pct: f64,
    pub tokens: Option<u64>,
    pub window: Option<u64>,
}

impl ContextUsage {
    fn merge(&mut self, value: &Value) {
        let field = |object: &Value, names: &[&str]| {
            names
                .iter()
                .find_map(|name| object.get(*name))
                .filter(|v| !v.is_null())
                .cloned()
        };
        if let Some(status) = field(value, &["statusLine", "status_line"]) {
            self.status_pct = field(&status, &["contextUsedPct", "context_used_pct"])
                .and_then(|v| v.as_f64())
                .filter(|pct| pct.is_finite());
            self.status_window = field(&status, &["contextWindowSize", "context_window_size"])
                .and_then(|v| v.as_u64())
                .filter(|w| *w > 0);
            self.waiting = matches!(
                field(&status, &["contextUsageState", "context_usage_state"])
                    .as_ref()
                    .and_then(Value::as_str),
                Some("waitingForRuntimeUsage" | "waiting_for_runtime_usage")
            );
        }
        if let Some(usage) = field(value, &["usage"]) {
            self.held_tokens = field(&usage, &["contextTokens", "context_tokens"])
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.usage_limit = field(&usage, &["contextLimit", "context_limit"])
                .and_then(|v| v.as_u64())
                .filter(|w| *w > 0);
        }
        if let Some(window) = field(value, &["resolvedContextWindow", "resolved_context_window"])
            .and_then(|v| v.as_u64())
            .filter(|w| *w > 0)
        {
            self.resolved_window = Some(window);
        }
    }

    /// TWIN: `derive_stats` in apps/tui/src/types.rs and desktop
    /// `deriveSessionStats`. A status percentage and its window are one claim;
    /// when the session demonstrably holds more than that window (past the
    /// shared 2% tolerance) both are rejected for the resolved window.
    pub fn reading(&self) -> Option<ContextReading> {
        if self.waiting {
            return None;
        }
        let owner = self.resolved_window.or(self.usage_limit);
        let from_usage = || {
            let window = owner?;
            (self.held_tokens > 0).then(|| ContextReading {
                pct: self.held_tokens as f64 / window as f64 * 100.,
                tokens: Some(self.held_tokens),
                window: Some(window),
            })
        };
        let disproved = self
            .status_window
            .is_some_and(|window| self.held_tokens > window.saturating_mul(102) / 100);
        if disproved {
            return from_usage();
        }
        match self.status_pct {
            Some(pct) => {
                let window = self.status_window.or(owner);
                Some(ContextReading {
                    pct,
                    tokens: match (self.status_window, self.held_tokens) {
                        (Some(window), _) => Some((pct / 100. * window as f64).round() as u64),
                        (None, held) if held > 0 => Some(held),
                        _ => None,
                    },
                    window,
                })
            }
            None => from_usage(),
        }
    }
}

impl Session {
    pub fn id_of(value: &Value) -> Option<&str> {
        value
            .get("sessionId")
            .or_else(|| value.get("session_id"))?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    pub fn merge(&mut self, value: &Value) {
        self.telemetry.merge(value);
        self.context.merge(value);
        for (target, names) in [
            (&mut self.id, &["sessionId", "session_id"][..]),
            (
                &mut self.label,
                &["label", "customName", "name", "title"][..],
            ),
            (&mut self.provider, &["provider"][..]),
            (
                &mut self.parent_session_id,
                &["parentSessionId", "parent_session_id"][..],
            ),
            (&mut self.model, &["model"][..]),
            (&mut self.cwd, &["cwd"][..]),
            (&mut self.state, &["mode", "ambientState"][..]),
            (&mut self.transport, &["transport"][..]),
        ] {
            if let Some(s) = names
                .iter()
                .find_map(|name| value.get(name).and_then(Value::as_str))
            {
                *target = s.to_owned();
            }
        }
        if let Some(model) = value
            .pointer("/requestedSelection/model")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/settings/model").and_then(Value::as_str))
        {
            self.model = model.to_owned();
            self.context_window = if value.get("requestedSelection").is_some() {
                value
                    .pointer("/requestedSelection/contextWindow")
                    .and_then(Value::as_u64)
            } else {
                value
                    .pointer("/settings/contextWindow")
                    .and_then(Value::as_u64)
            };
        } else if value.get("model").is_none()
            && let Some(model) = value.pointer("/usage/model").and_then(Value::as_str)
            && !model.is_empty()
        {
            self.model = model.to_owned();
        }
        if let Some(model) = [
            "/statusLine/modelDisplay",
            "/status_line/model_display",
            "/usage/model",
        ]
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .filter(|m| !m.is_empty())
        {
            self.runtime_model = crate::transcript::head(model, 128);
        }
        if let Some(skills) = value.pointer("/statusLine/capabilities/inventory/skills") {
            self.skills = bounded_inventory(skills, &["name", "description", "origin", "path"]);
        }
        if let Some(items) = value.get("subagents") {
            let mut projected = bounded_inventory(
                items,
                &[
                    "id",
                    "toolUseId",
                    "description",
                    "type",
                    "status",
                    "model",
                    "lastToolName",
                    "lastToolSummary",
                    "tokens",
                    "costUSD",
                    "toolCalls",
                    "startedAt",
                    "completedAt",
                    "sessionId",
                    "lastActivity",
                    "tokenCount",
                    "totalCostUsd",
                    "durationMs",
                ],
            );
            // A supplied array owns membership; sparse entries preserve their
            // previously reported measurements without keeping removed agents.
            if let Some(rows) = projected.as_array_mut() {
                for row in rows {
                    if let Some(previous) = self
                        .subagents
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|previous| previous["id"] == row["id"] && row["id"].is_string())
                        .and_then(Value::as_object)
                    {
                        for (key, value) in previous {
                            row.as_object_mut()
                                .unwrap()
                                .entry(key.clone())
                                .or_insert_with(|| value.clone());
                        }
                    }
                    if row["status"] == "running" {
                        row["completedAt"] = Value::Null;
                        row["durationMs"] = Value::Null;
                    }
                }
            }
            self.subagents = projected;
        }
        if let Some(items) = value.get("workflows") {
            self.workflows = bounded_inventory(
                items,
                &["runId", "toolUseId", "name", "description", "status"],
            );
        }
        if value.get("status").and_then(Value::as_str) == Some("ended") {
            self.state = "stopped".into();
        }
        for (target, name) in [
            (&mut self.approval, "pendingApproval"),
            (&mut self.questions, "pendingQuestions"),
        ] {
            if let Some(v) = value.get(name) {
                *target = (!v.is_null()).then(|| v.clone());
            }
        }
        if let Some(pending) = value.get("pending") {
            if value.get("pendingApproval").is_none() {
                self.approval = (pending["kind"] == "approval").then(|| pending.clone());
            }
            if value.get("pendingQuestions").is_none() {
                self.questions =
                    (pending["kind"] == "question").then(|| pending["questions"].clone());
            }
        }
    }

    /// Human model name for display: the running model when it matches the
    /// selection (so `opus` reads "Opus 5.5"), otherwise the fresher selection.
    pub fn display_model(&self) -> String {
        let selected = self.model.split('[').next().unwrap_or("").trim();
        let observed = if self.runtime_model.is_empty() {
            self.telemetry.observed_model.trim()
        } else {
            self.runtime_model.trim()
        };
        let id = if selected.chars().any(|c| c.is_ascii_digit()) {
            selected
        } else if !observed.is_empty()
            && (selected.is_empty()
                || observed
                    .to_ascii_lowercase()
                    .contains(&selected.to_ascii_lowercase()))
        {
            observed
        } else {
            selected
        };
        model_display_name(id)
    }

    pub fn title(&self) -> &str {
        if !self.label.is_empty() {
            &self.label
        } else if !self.cwd.is_empty() {
            self.cwd
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .unwrap_or(&self.cwd)
        } else {
            &self.id
        }
    }

    pub fn working(&self) -> bool {
        !self.stopped()
            && self.approval.is_none()
            && self.questions.is_none()
            && matches!(
                self.state.as_str(),
                "responding" | "working" | "thinking" | "streaming" | "running"
            )
    }

    pub fn stopped(&self) -> bool {
        self.state == "stopped" || self.state == "ended"
    }
}

fn bounded_inventory(value: &Value, fields: &[&str]) -> Value {
    Value::Array(
        value
            .as_array()
            .into_iter()
            .flatten()
            .take(32)
            .map(|item| {
                let mut projected = serde_json::Map::new();
                for field in fields {
                    if let Some(text) = item[*field].as_str() {
                        projected.insert(
                            (*field).into(),
                            Value::String(crate::transcript::head(text, 512)),
                        );
                    }
                    if item[*field].is_number() {
                        projected.insert((*field).into(), item[*field].clone());
                    }
                    if item.get(*field).is_some_and(Value::is_null) {
                        projected.insert((*field).into(), Value::Null);
                    }
                }
                Value::Object(projected)
            })
            .collect(),
    )
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Item {
    #[serde(default, alias = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub steps: Value,
    #[serde(default, alias = "updatedAt")]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub args: Option<String>,
}

impl Item {
    pub fn display(self) -> Option<(&'static str, String)> {
        Some(match self.kind.as_str() {
            "user_message" => ("You", self.text),
            "assistant_text" => ("Assistant", self.text),
            "tool_use" => ("Tool", format!("{}\n{}", self.name, self.input)),
            "tool_result" => (
                if self.is_error {
                    "Tool error"
                } else {
                    "Tool result"
                },
                self.content,
            ),
            "slash_command" => (
                "Command",
                format!("/{} {}", self.name, self.args.unwrap_or_default()),
            ),
            "command_output" => (
                if self.is_error {
                    "Command error"
                } else {
                    "Command output"
                },
                self.output,
            ),
            "plan" => (
                "Plan",
                serde_json::to_string_pretty(&self.steps).unwrap_or_default(),
            ),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConversationSnapshot {
    pub seq: u64,
    #[serde(default)]
    pub first_seq: u64,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Delta {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub reset: bool,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    pub key: u64,
    pub role: String,
    pub text: String,
    pub truncated: bool,
    pub timestamp: Option<String>,
    pub timestamp_ms: Option<i64>,
    pub tool: Option<crate::transcript::Tool>,
    pub result_id: String,
}

impl Row {
    pub fn same_content(&self, other: &Self) -> bool {
        self.role == other.role
            && self.text == other.text
            && self.truncated == other.truncated
            && self.timestamp == other.timestamp
            && self.timestamp_ms == other.timestamp_ms
            && self.tool == other.tool
            && self.result_id == other.result_id
    }

    pub fn copy_text(&self) -> String {
        self.tool
            .as_ref()
            .map(|t| format!("{}\n{}\n{}", t.name, t.input, t.output))
            .unwrap_or_else(|| self.text.clone())
    }

    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        let mut add = |text: &str| {
            for b in text.bytes() {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
            }
        };
        add(&self.text);
        if let Some(t) = &self.tool {
            add(&t.input);
            add(&t.output);
            add(if t.complete { "complete" } else { "pending" });
            add(if t.is_error { "error" } else { "ok" });
        }
        hash
    }
    fn identity(&self) -> String {
        let identity = self
            .tool
            .as_ref()
            .map(|t| t.id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| self.timestamp.clone())
            .unwrap_or_else(|| crate::transcript::head(&self.text, 64));
        format!("{}:{identity}", self.role)
    }
    pub fn bytes(&self) -> usize {
        self.text.len()
            + self.timestamp.as_ref().map_or(0, String::len)
            + self.result_id.len()
            + self.tool.as_ref().map_or(0, crate::transcript::Tool::bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Changed,
    Unchanged,
    Gap,
}

/// Sequence numbers count raw events, not coalesced rows. Only a full snapshot
/// or a contiguous delta may advance the cursor. This type has no UI/runtime.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    pub rows: VecDeque<Arc<Row>>,
    pub seq: Option<u64>,
    pub bytes: usize,
    pub omitted: bool,
    /// The daemon retains older items than this window; page back for them.
    pub has_older: bool,
    /// The client's row/byte budget dropped rows from the front.
    trimmed: bool,
    pub revision: u64,
    next_key: u64,
    history: bool,
}

impl Transcript {
    pub(crate) fn append_history(&mut self, item: Item) {
        self.history = true;
        self.push(item, false);
    }

    pub fn snapshot(&mut self, snapshot: ConversationSnapshot) {
        self.snapshot_for_transport(snapshot, false);
    }

    /// Apply the same fragment folding on initial reads, resyncs and live updates.
    pub fn snapshot_for_transport(&mut self, snapshot: ConversationSnapshot, streaming: bool) {
        let previous_order = self.rows.clone();
        let mut previous = std::collections::HashMap::<String, VecDeque<Arc<Row>>>::new();
        for row in self.rows.drain(..) {
            previous.entry(row.identity()).or_default().push_back(row);
        }
        self.bytes = 0;
        self.omitted = snapshot.first_seq > 1;
        self.trimmed = false;
        self.has_older = false;
        for item in snapshot.items {
            self.push(item, streaming);
        }
        for (ix, row) in self.rows.iter_mut().enumerate() {
            let old = previous
                .get_mut(&row.identity())
                .and_then(VecDeque::pop_front)
                .or_else(|| {
                    // A timestamp-free streamed reply may grow on reseed. Keep
                    // its key only when it remains at the same position and the
                    // old row has not already been reused by an exact match.
                    let old = previous_order.get(ix)?;
                    if row.role != old.role
                        || row.tool.is_some()
                        || old.tool.is_some()
                        || !row.text.starts_with(&old.text)
                    {
                        return None;
                    }
                    let candidates = previous.get_mut(&old.identity())?;
                    if candidates.front()?.key != old.key {
                        return None;
                    }
                    candidates.pop_front()
                });
            if let Some(old) = old {
                Arc::make_mut(row).key = old.key;
                if **row == *old {
                    *row = old;
                }
            }
        }
        self.seq = Some(snapshot.seq);
        self.revision += 1;
    }

    /// A limited read: `window_first_seq` is the daemon's sequence for the
    /// first returned item (absent from daemons without paging, whose reads
    /// are complete). Older retained items exist while it exceeds `first_seq`;
    /// paging stops at the client budget, past which History owns the rest.
    pub fn snapshot_page(
        &mut self,
        snapshot: ConversationSnapshot,
        window_first_seq: Option<u64>,
        streaming: bool,
    ) {
        let first_seq = snapshot.first_seq.max(1);
        self.snapshot_for_transport(snapshot, streaming);
        self.has_older = !self.trimmed && window_first_seq.is_some_and(|w| w > first_seq);
    }

    pub fn delta(&mut self, delta: Delta, streaming: bool) -> Fold {
        if delta.ready {
            return Fold::Unchanged;
        }
        if delta.reset {
            self.snapshot_for_transport(
                ConversationSnapshot {
                    seq: delta.seq,
                    first_seq: 0,
                    items: delta.items,
                },
                streaming,
            );
            return Fold::Changed;
        }
        let Some(seq) = self.seq else {
            return Fold::Gap;
        };
        if delta.seq <= seq {
            return Fold::Unchanged;
        }
        if delta.seq.checked_sub(delta.items.len() as u64) != Some(seq) {
            return Fold::Gap;
        }
        for item in delta.items {
            self.push(item, streaming);
        }
        self.seq = Some(delta.seq);
        self.revision += 1;
        Fold::Changed
    }

    fn push(&mut self, item: Item, streaming: bool) {
        let timestamp_ms = item
            .timestamp
            .as_deref()
            .and_then(crate::timing::parse_timestamp);
        let timestamp = item
            .timestamp
            .clone()
            .map(|s| crate::transcript::head(&s, 128));
        let result_id = if item.tool_use_id.len() <= 512 {
            item.tool_use_id.clone()
        } else {
            String::new()
        };
        if item.kind == "tool_result"
            && !result_id.is_empty()
            && let Some(row) = self
                .rows
                .iter_mut()
                .rev()
                .find(|r| r.tool.as_ref().is_some_and(|t| t.id == result_id))
        {
            self.bytes -= row.bytes();
            let row = Arc::make_mut(row);
            let tool = row.tool.as_mut().unwrap();
            tool.output = item.content;
            if !self.history {
                row.truncated |= truncate(&mut tool.output, MAX_ROW_BYTES / 2);
            }
            tool.is_error = item.is_error;
            tool.complete = true;
            tool.completed_at_ms = timestamp_ms.or(tool.completed_at_ms);
            self.bytes += row.bytes();
            self.enforce_bounds();
            return;
        }
        let tool = (item.kind == "tool_use").then(|| {
            let mut tool = crate::transcript::Tool::from_item(&item);
            if self.history {
                tool.input = serde_json::to_string(&item.input).unwrap_or_default();
                tool.clipped = false;
            }
            tool
        });
        // An empty failed result still communicates a failure, including when
        // the matching call was outside the retained conversation window.
        let empty_error = item.is_error
            && ((item.kind == "tool_result" && item.content.is_empty())
                || (item.kind == "command_output" && item.output.is_empty()));
        let Some((role, text)) = item.display() else {
            return;
        };
        // Stream sends are echoed without a timestamp, then the provider's
        // transcript tailer delivers the same turn with its real timestamp.
        // Merge that acknowledgement in place, including full snapshots.
        // Fresh timestamp-less sends and distinct timestamps remain separate.
        if matches!(role, "You" | "Assistant")
            && timestamp.is_some()
            && let Some(row) = self
                .rows
                .iter_mut()
                .rev()
                .take(5)
                .take_while(|r| role != "Assistant" || r.role != "You")
                .find(|r| {
                    r.role == role
                        && r.text == text
                        && (r.timestamp.is_none() || r.timestamp == timestamp)
                })
        {
            self.bytes -= row.bytes();
            let row = Arc::make_mut(row);
            row.timestamp = timestamp;
            row.timestamp_ms = timestamp_ms;
            self.bytes += row.bytes();
            self.enforce_bounds();
            return;
        }
        let text = if empty_error {
            "Failed without output".into()
        } else {
            text
        };
        if text.is_empty() && tool.is_none() {
            return;
        }
        // Only stream transports coalesce fragments; PTY transcript blocks are
        // already complete. Accumulated provider chunks replace their prefix.
        if streaming
            && role == "Assistant"
            && let Some(last) = self.rows.back_mut().filter(|r| r.role == role)
        {
            self.bytes -= last.bytes();
            let last = Arc::make_mut(last);
            last.timestamp_ms = last.timestamp_ms.or(timestamp_ms);
            last.timestamp = last.timestamp.take().or(timestamp.clone());
            if text.starts_with(&last.text) && !last.truncated {
                last.text = text;
            } else {
                last.text.push_str(&text);
            }
            last.truncated |= truncate(&mut last.text, MAX_ROW_BYTES);
            self.bytes += last.bytes();
        } else {
            let mut text = text;
            let truncated = !self.history && truncate(&mut text, MAX_ROW_BYTES);
            let row = Row {
                key: self.next_key,
                role: role.into(),
                text: if let Some(t) = &tool {
                    t.name.clone()
                } else {
                    text
                },
                truncated: truncated || tool.as_ref().is_some_and(|t| t.clipped),
                timestamp,
                timestamp_ms,
                tool,
                result_id,
            };
            self.bytes += row.bytes();
            self.rows.push_back(Arc::new(row));
            self.next_key += 1;
        }
        self.enforce_bounds();
    }

    fn enforce_bounds(&mut self) {
        if self.history {
            return;
        }
        while self.rows.len() > MAX_ROWS || self.bytes > MAX_TRANSCRIPT_BYTES {
            self.bytes -= self
                .rows
                .pop_front()
                .expect("over budget implies a row")
                .bytes();
            self.omitted = true;
            self.trimmed = true;
        }
    }
}

/// Preserve a UTF-8 boundary and the newest content. The UI labels clipping.
fn truncate(text: &mut String, limit: usize) -> bool {
    if text.len() <= limit {
        return false;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    // drain() would leave the original (possibly multi-megabyte) capacity
    // allocated behind a short string, defeating the retained-text budget.
    *text = text[start..].to_owned();
    true
}

/// Friendly model name: `claude-opus-5-5` → "Opus 5.5", `opus[1m]` → "Opus",
/// `claude-3-5-sonnet-20241022` → "Sonnet 3.5", `gpt-5.6-sol` → "GPT-5.6 Sol".
/// Unrecognized ids are returned unchanged.
/// The provider whose brand a model id belongs to, when the id says so.
pub fn model_provider(id: &str) -> Option<&'static str> {
    let lower = id.trim().to_ascii_lowercase();
    let lower = lower.rsplit('/').next().unwrap_or(&lower);
    if lower.starts_with("claude")
        || ["opus", "sonnet", "haiku", "fable"]
            .iter()
            .any(|f| lower.split('-').any(|w| w == *f))
    {
        Some("claude")
    } else if lower.starts_with("gpt") || lower.contains("codex") {
        Some("codex")
    } else {
        None
    }
}

pub fn model_display_name(id: &str) -> String {
    let id = id.trim();
    let id = id.rsplit('/').next().unwrap_or(id);
    let id = id.split('[').next().unwrap_or(id);
    let lower = id.to_ascii_lowercase();
    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars
            .next()
            .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default()
    };
    let parts: Vec<&str> = lower
        .strip_prefix("claude-")
        .unwrap_or(&lower)
        .split('-')
        .filter(|p| !(p.len() >= 6 && p.chars().all(|c| c.is_ascii_digit())))
        .collect();
    const FAMILIES: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];
    if let Some(family) = parts.iter().find(|p| FAMILIES.contains(p)) {
        let version = parts
            .iter()
            .filter(|p| p.chars().all(|c| c.is_ascii_digit()))
            .copied()
            .collect::<Vec<_>>()
            .join(".");
        return format!("{} {version}", capitalize(family))
            .trim()
            .to_owned();
    }
    if let Some(rest) = lower.strip_prefix("gpt-") {
        let mut words = rest.split('-');
        let version = words.next().unwrap_or_default();
        let mut name = format!("GPT-{version}");
        for word in words {
            name.push(' ');
            name.push_str(&capitalize(word));
        }
        return name;
    }
    id.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_reading_follows_the_runtime_and_rejects_disproved_windows() {
        let mut session = Session::default();
        assert_eq!(session.context.reading(), None);
        session.merge(&json!({"usage":{"contextTokens":50_000,"contextLimit":200_000}}));
        let reading = session.context.reading().unwrap();
        assert_eq!(reading.pct, 25.);
        assert_eq!(reading.tokens, Some(50_000));
        // The status line is authoritative and supplies its own window.
        session.merge(&json!({"statusLine":{"contextUsedPct":42.0,"contextWindowSize":200_000}}));
        let reading = session.context.reading().unwrap();
        assert_eq!(reading.pct, 42.);
        assert_eq!(reading.tokens, Some(84_000));
        // Snapshots without the sources keep the last reading.
        session.merge(&json!({"mode":"responding"}));
        assert_eq!(session.context.reading().unwrap().pct, 42.);
        // Holding more than the claimed window disproves the pair.
        session.merge(&json!({
            "usage":{"contextTokens":400_000,"contextLimit":1_000_000},
            "resolvedContextWindow":1_000_000
        }));
        let reading = session.context.reading().unwrap();
        assert_eq!(reading.pct, 40.);
        assert_eq!(reading.window, Some(1_000_000));
        // Waiting for current-request usage shows nothing rather than a guess.
        session.merge(&json!({"statusLine":{"contextWindowSize":200_000,"contextUsageState":"waitingForRuntimeUsage"}}));
        assert!(session.context.waiting);
        assert_eq!(session.context.reading(), None);
        // claudemon's snake_case spelling reads the same.
        let mut raw = Session::default();
        raw.merge(
            &json!({"status_line":{"context_used_pct":10.0,"context_window_size":1_000_000}}),
        );
        assert_eq!(raw.context.reading().unwrap().tokens, Some(100_000));
    }

    #[test]
    fn model_ids_name_their_provider() {
        assert_eq!(model_provider("claude-sonnet-4-6"), Some("claude"));
        assert_eq!(model_provider("opus"), Some("claude"));
        assert_eq!(model_provider("anthropic/claude-haiku-4-5"), Some("claude"));
        assert_eq!(model_provider("gpt-5.6-luna"), Some("codex"));
        assert_eq!(model_provider("gpt-5-codex"), Some("codex"));
        assert_eq!(model_provider("llama-3"), None);
        assert_eq!(model_provider(""), None);
    }

    #[test]
    fn timestamped_provider_echoes_converge_in_snapshots_and_live_deltas() {
        let user = Item {
            kind: "user_message".into(),
            text: "Try spawning an agent".into(),
            ..Default::default()
        };
        let assistant = assistant("Which task should it work on?");
        let stamp = |mut item: Item| {
            item.timestamp = Some("2026-09-30T22:03:27Z".into());
            item
        };
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 4,
            first_seq: 1,
            items: vec![
                user.clone(),
                stamp(user.clone()),
                assistant.clone(),
                stamp(assistant.clone()),
            ],
        });
        assert_eq!(transcript.rows.len(), 2);
        assert!(transcript.rows.iter().all(|r| r.timestamp_ms.is_some()));
        assert_eq!(
            transcript.bytes,
            transcript.rows.iter().map(|r| r.bytes()).sum::<usize>()
        );
        // Sequence cursors still count raw events, even when echoes share a row.
        assert_eq!(
            transcript.delta(
                Delta {
                    seq: 5,
                    items: vec![stamp(assistant)],
                    ..Default::default()
                },
                true
            ),
            Fold::Changed
        );
        assert_eq!(transcript.rows.len(), 2);
        assert_eq!(transcript.seq, Some(5));
        let mut repeated = stamp(user.clone());
        repeated.timestamp = Some("2026-09-30T22:04:27Z".into());
        transcript.delta(
            Delta {
                seq: 6,
                items: vec![repeated],
                ..Default::default()
            },
            true,
        );
        assert_eq!(
            transcript.rows.len(),
            3,
            "a genuine repeat with a new timestamp survives"
        );
        transcript.delta(
            Delta {
                seq: 8,
                items: vec![user.clone(), user],
                ..Default::default()
            },
            true,
        );
        assert_eq!(transcript.rows.len(), 5, "two fresh sends remain distinct");
    }

    fn assistant(text: &str) -> Item {
        Item {
            kind: "assistant_text".into(),
            text: text.into(),
            ..Default::default()
        }
    }
    fn delta(seq: u64, text: &str) -> Delta {
        Delta {
            seq,
            items: vec![assistant(text)],
            ..Default::default()
        }
    }

    #[test]
    fn stream_snapshot_and_reset_preserve_markdown_across_fragments() {
        let parts = vec![
            assistant("Both scouts are running:\n- **Workspacer"),
            assistant(" child:** WS feature scout\n- **Native subagent:** native_scout"),
        ];
        let mut t = Transcript::default();
        t.snapshot_for_transport(
            ConversationSnapshot {
                seq: 2,
                first_seq: 1,
                items: parts.clone(),
            },
            true,
        );
        assert_eq!(t.rows.len(), 1);
        assert_eq!(
            t.rows[0].text,
            "Both scouts are running:\n- **Workspacer child:** WS feature scout\n- **Native subagent:** native_scout"
        );
        let key = t.rows[0].key;
        t.delta(
            Delta {
                seq: 2,
                reset: true,
                items: parts.clone(),
                ..Default::default()
            },
            true,
        );
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].key, key);
        t.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: parts,
        });
        assert_eq!(t.rows.len(), 2, "PTY blocks remain separate");
    }

    #[test]
    fn coalesced_snapshot_sequence_is_not_row_count() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 100,
            first_seq: 1,
            items: vec![assistant("hello")],
        });
        assert_eq!(t.delta(delta(101, " world"), true), Fold::Changed);
        assert_eq!(t.rows[0].text, "hello world");
        assert_eq!(t.delta(delta(101, " world"), true), Fold::Unchanged);
        assert_eq!(t.delta(delta(103, "lost"), true), Fold::Gap);
        assert_eq!(t.seq, Some(101));
    }

    #[test]
    fn reset_can_move_sequence_backwards_and_accumulated_chunks_replace() {
        let mut t = Transcript::default();
        t.delta(
            Delta {
                seq: 40,
                reset: true,
                items: vec![assistant("old")],
                ..Default::default()
            },
            true,
        );
        t.delta(
            Delta {
                seq: 1,
                reset: true,
                items: vec![assistant("new")],
                ..Default::default()
            },
            true,
        );
        t.delta(delta(2, "new text"), true);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].text, "new text");
    }

    #[test]
    fn bounded_history_preserves_utf8_and_published_rows() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![assistant("hello")],
        });
        let before = t.clone();
        t.delta(delta(2, " world"), true);
        assert_eq!(before.rows[0].text, "hello");
        for _ in 0..3_000 {
            t.push(assistant(&"🦀".repeat(20_000)), false);
        }
        assert!(t.rows.len() <= MAX_ROWS);
        assert!(t.bytes <= MAX_TRANSCRIPT_BYTES);
        assert!(
            t.rows
                .iter()
                .all(|r| r.text.len() <= MAX_ROW_BYTES && r.truncated)
        );
        assert!(t.omitted);
        assert!(t.rows.iter().all(|r| r.text.capacity() <= MAX_ROW_BYTES));
    }

    #[test]
    fn tools_pair_by_id_preserve_empty_errors_and_survive_refresh() {
        let items = vec![
            Item {
                kind: "tool_use".into(),
                id: "a".into(),
                name: "Read".into(),
                input: json!({"file_path":"a.rs"}),
                ..Default::default()
            },
            Item {
                kind: "tool_use".into(),
                id: "b".into(),
                name: "Edit".into(),
                input: json!({"file_path":"b.rs","old_string":"a","new_string":"b"}),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "b".into(),
                content: "".into(),
                is_error: true,
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "a".into(),
                content: "1 code".into(),
                ..Default::default()
            },
        ];
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 4,
            first_seq: 1,
            items: items.clone(),
        });
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].tool.as_ref().unwrap().output, "1 code");
        assert!(t.rows[1].tool.as_ref().unwrap().is_error);
        assert!(t.rows[1].tool.as_ref().unwrap().complete);
        let before = t.rows[0].clone();
        t.snapshot(ConversationSnapshot {
            seq: 4,
            first_seq: 1,
            items,
        });
        assert!(Arc::ptr_eq(&before, &t.rows[0]));
        assert_eq!(t.bytes, t.rows.iter().map(|r| r.bytes()).sum::<usize>());
        assert_eq!(
            t.delta(
                Delta {
                    seq: 5,
                    items: vec![Item {
                        kind: "tool_result".into(),
                        tool_use_id: "evicted".into(),
                        content: "Still visible".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                true
            ),
            Fold::Changed
        );
        assert_eq!(t.rows.back().unwrap().text, "Still visible");
    }
    #[test]
    fn failed_results_outside_the_window_and_command_stderr_stay_visible() {
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 3,
            first_seq: 10,
            items: vec![
                Item {
                    kind: "tool_result".into(),
                    tool_use_id: "missing".into(),
                    is_error: true,
                    ..Default::default()
                },
                Item {
                    kind: "command_output".into(),
                    output: "permission denied".into(),
                    is_error: true,
                    ..Default::default()
                },
                Item {
                    kind: "command_output".into(),
                    is_error: true,
                    ..Default::default()
                },
            ],
        });
        assert_eq!(transcript.rows.len(), 3);
        assert_eq!(transcript.rows[0].role, "Tool error");
        assert_eq!(transcript.rows[0].text, "Failed without output");
        assert_eq!(transcript.rows[1].role, "Command error");
        assert_eq!(transcript.rows[1].text, "permission denied");
        assert_eq!(transcript.rows[2].text, "Failed without output");
    }

    #[test]
    fn structured_payloads_obey_the_transcript_budget() {
        let mut t = Transcript::default();
        for n in 0..100 {
            t.push(
                Item {
                    kind: "tool_use".into(),
                    id: n.to_string(),
                    name: "Write".into(),
                    input: json!({"content":"x".repeat(100000)}),
                    ..Default::default()
                },
                false,
            );
            t.push(
                Item {
                    kind: "tool_result".into(),
                    tool_use_id: n.to_string(),
                    content: "y".repeat(100000),
                    ..Default::default()
                },
                false,
            );
        }
        assert!(t.bytes <= MAX_TRANSCRIPT_BYTES);
        assert!(t.omitted);
        assert!(t.rows.iter().all(|r| r.truncated));
        assert_eq!(t.bytes, t.rows.iter().map(|r| r.bytes()).sum::<usize>());
    }

    #[test]
    fn sparse_aliases_do_not_clear_missing_fields_but_null_clears_approval() {
        let mut s = Session::default();
        s.merge(&json!({"session_id":"s", "sessionId":"s", "label":"Build", "mode":"approval", "pendingApproval":{"toolName":"Bash"}}));
        s.merge(&json!({"sessionId":"s", "cwd":"/tmp/repo"}));
        assert!(s.approval.is_some());
        assert_eq!(s.title(), "Build");
        s.merge(&json!({"sessionId":"s", "pendingApproval":null, "mode":"input"}));
        assert!(s.approval.is_none());
    }

    #[test]
    fn model_names_read_like_product_names() {
        for (id, name) in [
            ("claude-opus-5-5", "Opus 5.5"),
            ("claude-haiku-4-5-20251001", "Haiku 4.5"),
            ("claude-3-5-sonnet-20241022", "Sonnet 3.5"),
            ("anthropic/claude-sonnet-4-6", "Sonnet 4.6"),
            ("opus[1m]", "Opus"),
            ("fable", "Fable"),
            ("gpt-5.6-sol", "GPT-5.6 Sol"),
            ("gpt-5.4", "GPT-5.4"),
            ("o3", "o3"),
            ("", ""),
        ] {
            assert_eq!(model_display_name(id), name, "{id}");
        }
    }

    #[test]
    fn display_model_prefers_running_model_only_when_it_matches_the_selection() {
        let mut session = Session::default();
        session.merge(&json!({"requestedSelection":{"model":"opus"}, "statusLine":{"modelDisplay":"claude-opus-5-5"}}));
        assert_eq!(session.display_model(), "Opus 5.5");
        // Later snapshots with a bare alias or a null usage model keep it.
        session.merge(&json!({"model":"opus[1m]", "usage":{"model":null}}));
        assert_eq!(session.display_model(), "Opus 5.5");
        // A fresh switch shows the new selection until the runtime reports it.
        session.merge(&json!({"requestedSelection":{"model":"sonnet"}}));
        assert_eq!(session.display_model(), "Sonnet");
        session.merge(&json!({"requestedSelection":{"model":"claude-sonnet-4-6"}}));
        assert_eq!(session.display_model(), "Sonnet 4.6");
    }

    #[test]
    fn session_model_uses_reported_usage_when_selection_is_absent() {
        let mut session = Session::default();
        session.merge(&json!({"usage":{"model":"reported-model"}}));
        assert_eq!(session.model, "reported-model");
        session.merge(&json!({"label":"Updated label"}));
        assert_eq!(session.model, "reported-model");
        session.merge(&json!({"requestedSelection":{"model":"selected-model"}, "usage":{"model":"old-model"}}));
        assert_eq!(session.model, "selected-model");
    }
    #[test]
    fn message_times_keep_stream_start_and_join_tool_completion() {
        let start = "2026-09-27T12:00:00Z";
        let end = "2026-09-27T12:00:07Z";
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: "Hello".into(),
                timestamp: Some(start.into()),
                ..Default::default()
            }],
        });
        t.delta(
            Delta {
                seq: 2,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: " world".into(),
                    timestamp: Some(end.into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            true,
        );
        assert_eq!(
            t.rows[0].timestamp_ms,
            crate::timing::parse_timestamp(start)
        );
        t.push(
            Item {
                kind: "tool_use".into(),
                id: "t1".into(),
                name: "Read".into(),
                timestamp: Some(start.into()),
                ..Default::default()
            },
            false,
        );
        t.push(
            Item {
                kind: "tool_result".into(),
                tool_use_id: "t1".into(),
                content: "result".into(),
                timestamp: Some(end.into()),
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            t.rows[1].timestamp_ms,
            crate::timing::parse_timestamp(start)
        );
        assert_eq!(
            t.rows[1].tool.as_ref().unwrap().completed_at_ms,
            crate::timing::parse_timestamp(end)
        );
        let missing: Item =
            serde_json::from_value(json!({"kind":"user_message","text":"legacy"})).unwrap();
        assert!(missing.timestamp.is_none());
    }
    #[test]
    fn reseeding_history_preserves_unchanged_rows_and_streaming_identity() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: vec![assistant("First"), assistant("Second")],
        });
        let before = t.clone();
        t.snapshot(ConversationSnapshot {
            seq: 3,
            first_seq: 1,
            items: vec![assistant("First"), assistant("Second grows")],
        });
        assert!(Arc::ptr_eq(&t.rows[0], &before.rows[0]));
        assert!(!Arc::ptr_eq(&t.rows[1], &before.rows[1]));
        assert_eq!(t.rows[1].key, before.rows[1].key);
        assert_eq!(before.rows[1].text, "Second");
    }
}
