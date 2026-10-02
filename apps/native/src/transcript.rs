//! Shared transcript presentation data. Never interpret tool output as assistant UI.
use crate::model::{Item, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn tool_name(name: &str) -> &str {
    let name = name.rsplit("__").next().unwrap_or(name);
    name.rsplit('.').next().unwrap_or(name)
}

pub fn head(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    /// Bounded serialized input, independent of JSON allocation overhead.
    pub input: String,
    pub output: String,
    pub complete: bool,
    #[serde(default)]
    pub completed_at_ms: Option<i64>,
    pub is_error: bool,
    pub clipped: bool,
}
impl Tool {
    pub fn from_item(item: &Item) -> Self {
        let input = serde_json::to_string(&item.input).unwrap_or_default();
        Self {
            id: if item.id.len() <= 512 {
                item.id.clone()
            } else {
                String::new()
            },
            name: head(&item.name, 256),
            clipped: input.len() > 32768,
            input: head(&input, 32768),
            ..Default::default()
        }
    }
    pub fn bytes(&self) -> usize {
        self.id.len() + self.name.len() + self.input.len() + self.output.len()
    }
    pub fn value(&self) -> Value {
        serde_json::from_str(&self.input).unwrap_or(Value::Null)
    }
    pub fn category(&self) -> &'static str {
        let normalized = self.name.to_ascii_lowercase();
        let name = tool_name(&normalized);
        match name {
            "skill" => "Skill",
            "agent" | "task" | "spawn_agent" | "spawn-agent" => "Subagent",
            "workflow" | "run_workflow" | "workflow_run" => "Workflow",
            "edit" | "multiedit" | "str_replace_editor" | "write" | "write_file"
            | "create_file" | "notebookedit" | "apply_patch" | "patch" => "Edit",
            "read" | "read_file" => "Read",
            "bash" | "shell" | "exec_command" | "run_command" | "terminal" => "Command",
            "grep" | "glob" | "search" => "Search",
            _ => "Tool",
        }
    }
    pub fn target(&self) -> String {
        let v = self.value();
        let keys: &[&str] = match self.category() {
            "Skill" => &["skill", "name"],
            "Subagent" | "Workflow" => {
                &["description", "label", "name", "task", "prompt", "message"]
            }
            "Command" => &["command", "cmd"],
            _ => &[
                "file_path",
                "filePath",
                "path",
                "filename",
                "pattern",
                "query",
            ],
        };
        keys.iter()
            .find_map(|k| v[*k].as_str())
            .map(|s| head(s.lines().next().unwrap_or(s), 180))
            .unwrap_or_else(|| self.name.clone())
    }
    /// Workspacer receipts contain session IDs; provider-native Agent/Task IDs
    /// belong to a separate namespace and must never become fleet links.
    pub fn spawned_session_id(&self) -> Option<String> {
        if !self.complete
            || self.is_error
            || !matches!(
                tool_name(&self.name.to_ascii_lowercase()),
                "spawn_agent" | "spawn-agent"
            )
        {
            return None;
        }
        fn session_id(value: &Value) -> Option<String> {
            let id = value.get("sessionId").and_then(Value::as_str)?;
            (!id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
                .then(|| id.to_owned())
        }
        let value: Value = serde_json::from_str(&self.output).ok()?;
        if value["isError"].as_bool() == Some(true) {
            return None;
        }
        session_id(&value)
            .or_else(|| session_id(&value["structuredContent"]))
            .or_else(|| {
                value["content"].as_array()?.iter().find_map(|block| {
                    if block["type"].as_str() != Some("text") {
                        return None;
                    }
                    session_id(&serde_json::from_str::<Value>(block["text"].as_str()?).ok()?)
                })
            })
    }

    pub fn changes(&self) -> Vec<FileChange> {
        self.changes_with_diff(true)
    }
    fn changes_with_diff(&self, include_diff: bool) -> Vec<FileChange> {
        if self.category() != "Edit" || self.is_error {
            return vec![];
        }
        let v = self.value();
        let mut changes = Vec::new();
        if let Some(entries) = v["changes"].as_array() {
            for entry in entries.iter().take(100) {
                add_change(&mut changes, entry, &self.name, include_diff);
            }
        } else if v["edits"].is_array() {
            for edit in v["edits"].as_array().unwrap().iter().take(100) {
                let mut edit = edit.clone();
                edit["file_path"] = ["file_path", "filePath", "path", "filename"]
                    .iter()
                    .find_map(|key| v.get(*key))
                    .cloned()
                    .unwrap_or(Value::Null);
                add_change(&mut changes, &edit, &self.name, include_diff);
            }
        } else {
            add_change(&mut changes, &v, &self.name, include_diff);
        }
        changes
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub diff: String,
}
fn add_change(out: &mut Vec<FileChange>, v: &Value, name: &str, include_diff: bool) {
    let path = v["file_path"]
        .as_str()
        .or_else(|| v["path"].as_str())
        .or_else(|| v["filePath"].as_str())
        .or_else(|| v["filename"].as_str())
        .unwrap_or("");
    let patch = v["diff"]
        .as_str()
        .or_else(|| v["patch"].as_str())
        .or_else(|| {
            matches!(
                tool_name(&name.to_ascii_lowercase()),
                "apply_patch" | "patch"
            )
            .then(|| v.as_str().or_else(|| v["input"].as_str()))
            .flatten()
        })
        .unwrap_or("");
    if !patch.is_empty() {
        // Multi-file unified and apply_patch payloads can carry paths in headers.
        let mut current = FileChange {
            path: path.into(),
            ..Default::default()
        };
        let mut has_lines = false;
        let lines: Vec<_> = patch.lines().collect();
        for (index, line) in lines.iter().copied().enumerate() {
            if line.starts_with("diff --git ") {
                if has_lines && !current.path.is_empty() {
                    out.push(current);
                }
                current = FileChange::default();
                has_lines = false;
            }
            // Unified patches begin each file with an old/new header pair.
            // Start the boundary at --- so it belongs to the new file, and
            // retain the old path when +++ is /dev/null (a deleted file).
            let old_header = line.strip_prefix("--- ").filter(|_| {
                lines
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("+++ "))
            });
            if old_header.is_some() && has_lines && !current.path.is_empty() {
                out.push(current);
                current = FileChange::default();
                has_lines = false;
            }
            let paired_new_header =
                line.starts_with("+++ ") && index > 0 && lines[index - 1].starts_with("--- ");
            let header = line
                .strip_prefix("*** Update File: ")
                .or_else(|| line.strip_prefix("*** Add File: "))
                .or_else(|| line.strip_prefix("*** Delete File: "))
                .or_else(|| line.strip_prefix("+++ b/"))
                .or_else(|| {
                    old_header
                        .filter(|file| *file != "/dev/null")
                        .map(|file| file.strip_prefix("a/").unwrap_or(file))
                });
            if let Some(file) = header {
                if !current.path.is_empty()
                    && has_lines
                    && (current.path != file && !paired_new_header)
                {
                    out.push(current);
                    current = FileChange::default();
                }
                current.path = file.into();
            }
            if line.starts_with('+') && !line.starts_with("+++") {
                current.added += 1;
            }
            if line.starts_with('-') && !line.starts_with("---") {
                current.removed += 1;
            }
            if include_diff {
                current.diff.push_str(line);
                current.diff.push('\n');
            }
            has_lines = true;
        }
        if !current.path.is_empty() {
            out.push(current);
        }
    } else if !path.is_empty() {
        let old = v["old_string"]
            .as_str()
            .or_else(|| v["oldString"].as_str())
            .or_else(|| v["old_str"].as_str())
            .or_else(|| v["oldText"].as_str())
            .unwrap_or("");
        let new = v["new_string"]
            .as_str()
            .or_else(|| v["newString"].as_str())
            .or_else(|| v["new_str"].as_str())
            .or_else(|| v["newText"].as_str())
            .or_else(|| {
                if matches!(
                    tool_name(&name.to_ascii_lowercase()),
                    "write" | "write_file" | "create_file"
                ) {
                    v["content"]
                        .as_str()
                        .or_else(|| v["text"].as_str())
                        .or_else(|| v["file_text"].as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");
        let diff = if include_diff {
            old.lines()
                .map(|l| format!("-{l}\n"))
                .chain(new.lines().map(|l| format!("+{l}\n")))
                .collect()
        } else {
            String::new()
        };
        out.push(FileChange {
            path: path.into(),
            added: new.lines().count(),
            removed: old.lines().count(),
            diff,
        });
    }
}

/// Rows within one assistant turn, with a frozen tool-input estimate, never a
/// later repository status mislabeled as the state of an earlier turn.
pub fn turn_changes<'a>(rows: impl Iterator<Item = &'a Row>) -> Vec<FileChange> {
    let mut files = BTreeMap::<String, FileChange>::new();
    for row in rows {
        if let Some(tool) = &row.tool
            && tool.complete
        {
            // The footer needs counts only. Constructing every inline diff on
            // each render would allocate and immediately discard all its lines.
            for change in tool.changes_with_diff(false) {
                let file = files
                    .entry(change.path.clone())
                    .or_insert_with(|| FileChange {
                        path: change.path.clone(),
                        ..Default::default()
                    });
                file.added += change.added;
                file.removed += change.removed;
                // Summaries retain counts and paths; inline diffs live on the tool.
            }
        }
    }
    files.into_values().collect()
}

/// Preserve non-image markers and failed image paths. The UI strips only markers
/// whose preview actually decoded, so attachments can never silently disappear.
pub fn image_paths(text: &str, user: bool, cwd: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    if user {
        let mut rest = text;
        while let Some(start) = rest.find("[Image: ") {
            rest = &rest[start..];
            let Some(end) = rest.find(']') else {
                break;
            };
            let marker = &rest[..=end];
            let path = rest[8..end].trim();
            if raster(path) {
                out.push((marker.into(), resolve_path(cwd, path)));
            }
            rest = &rest[end + 1..];
            if out.len() == 8 {
                break;
            }
        }
    } else {
        for token in text.split([
            '\n', '\r', '\t', ' ', '`', '"', '\'', '(', ')', '[', ']', '<', '>',
        ]) {
            let path = token.trim_end_matches([',', ';', '.', ':']);
            if raster(path) && absolute(path) && !out.iter().any(|(_, p)| p == path) {
                out.push((String::new(), path.into()));
                if out.len() == 4 {
                    break;
                }
            }
        }
    }
    out
}
fn raster(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        )
    })
}
pub fn absolute(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (path.as_bytes().get(1) == Some(&b':')
            && path
                .as_bytes()
                .get(2)
                .is_some_and(|b| *b == b'/' || *b == b'\\'))
}
pub fn resolve_path(cwd: &str, path: &str) -> String {
    if absolute(path) {
        path.into()
    } else {
        format!(
            "{}{sep}{}",
            cwd.trim_end_matches(['/', '\\']),
            path,
            sep = if cwd.contains('\\') { "\\" } else { "/" }
        )
    }
}

/// Explicit Markdown file targets supplement the native Markdown renderer's web links.
pub fn file_paths(text: &str, cwd: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for tail in text.split("](").skip(1) {
        let Some((target, _)) = tail.split_once(')') else {
            continue;
        };
        let mut target = target.trim().trim_matches(['<', '>']);
        if target.contains("://") || target.starts_with('#') || target.is_empty() {
            continue;
        }
        for _ in 0..2 {
            if let Some((path, line)) = target.rsplit_once(':')
                && !line.is_empty()
                && line.bytes().all(|b| b.is_ascii_digit())
            {
                target = path;
            }
        }
        if target.contains('\0') || target.len() > 4096 {
            continue;
        }
        let path = resolve_path(cwd, target);
        if !paths.contains(&path) {
            paths.push(path);
        }
        if paths.len() == 16 {
            break;
        }
    }
    paths
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssistantBlock {
    Markdown(String),
    Card(Card),
    Result(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub fallback: String,
    pub body: String,
    pub actions: Vec<CardAction>,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardAction {
    OpenWorker {
        label: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    ViewDiff {
        label: String,
        path: String,
    },
    FillComposer {
        label: String,
        text: String,
    },
}
impl CardAction {
    pub fn label(&self) -> String {
        match self {
            Self::OpenWorker { label, .. } => format!("Open worker: {label}"),
            Self::ViewDiff { label, .. } => format!("View diff: {label}"),
            Self::FillComposer { label, .. } => format!("Prefill: {label}"),
        }
    }
    fn valid(&self) -> bool {
        let (label, value, limit) = match self {
            Self::OpenWorker { label, session_id } => (label, session_id, 200),
            Self::ViewDiff { label, path } => (label, path, 4096),
            Self::FillComposer { label, text } => (label, text, 4000),
        };
        !label.trim().is_empty()
            && label.chars().count() <= 48
            && !value.trim().is_empty()
            && value.chars().count() <= limit
    }
}
fn parse_card(raw: &str) -> Option<Card> {
    if raw.len() > 65536 {
        return None;
    }
    let v: Value = serde_json::from_str(raw).ok()?;
    let title = v["title"].as_str()?.trim();
    let fallback = v["fallback"].as_str()?.trim();
    if title.is_empty()
        || title.chars().count() > 120
        || fallback.is_empty()
        || fallback.chars().count() > 4000
    {
        return None;
    }
    // Unknown/malformed envelopes still render readable fallback, with no actions.
    let actions = if v["v"] == 1
        && v["bodyHtml"].as_str().is_some_and(|s| !s.trim().is_empty())
        && v.get("css")
            .is_none_or(|v| v.as_str().is_some_and(|s| s.len() <= 16384))
    {
        let parsed: Option<Vec<CardAction>> = v
            .get("actions")
            .map(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(Some(vec![]));
        parsed
            .filter(|a| a.len() <= 8 && a.iter().all(CardAction::valid))
            .unwrap_or_default()
    } else {
        vec![]
    };
    let body = if v["v"] == 1 {
        v["bodyHtml"]
            .as_str()
            .map(native_card_html)
            .unwrap_or_default()
    } else {
        String::new()
    };
    Some(Card {
        title: title.into(),
        fallback: fallback.into(),
        body,
        actions,
    })
}
/// GPUI Component 0.5.1 parses HTML twice: its minifier decodes text entities
/// and writes them without re-escaping before the final parser. Encode for both
/// passes or literal tags disappear (and escaped tags can become live nodes).
pub fn escape_native_html_text(text: &str) -> String {
    text.replace('&', "&amp;amp;")
        .replace('<', "&amp;lt;")
        .replace('>', "&amp;gt;")
}

/// Convert response HTML into inert native text/table layout. No URLs, style,
/// scripts, forms or event handlers cross into the GPUI text parser.
pub fn native_card_html(html: &str) -> String {
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::{Handle, NodeData, RcDom};
    fn visit(node: &Handle, out: &mut String, depth: usize) {
        if depth > 64 {
            return;
        }
        match &node.data {
            NodeData::Text { contents } => {
                out.push_str(&escape_native_html_text(&contents.borrow()))
            }
            NodeData::Element { name, .. } => {
                let tag = name.local.as_ref();
                if matches!(
                    tag,
                    "script"
                        | "style"
                        | "iframe"
                        | "object"
                        | "embed"
                        | "svg"
                        | "math"
                        | "img"
                        | "video"
                        | "audio"
                        | "input"
                        | "button"
                        | "select"
                        | "textarea"
                        | "link"
                        | "meta"
                ) {
                    return;
                }
                let allowed = matches!(
                    tag,
                    "p" | "div"
                        | "span"
                        | "strong"
                        | "b"
                        | "em"
                        | "i"
                        | "s"
                        | "del"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "ul"
                        | "ol"
                        | "li"
                        | "blockquote"
                        | "pre"
                        | "code"
                        | "br"
                        | "hr"
                        | "table"
                        | "thead"
                        | "tbody"
                        | "tr"
                        | "td"
                        | "th"
                );
                if allowed {
                    out.push('<');
                    out.push_str(tag);
                    out.push('>');
                }
                for child in node.children.borrow().iter() {
                    visit(child, out, depth + 1);
                }
                if allowed && !matches!(tag, "br" | "hr") {
                    out.push_str("</");
                    out.push_str(tag);
                    out.push('>');
                }
            }
            _ => {
                for child in node.children.borrow().iter() {
                    visit(child, out, depth + 1);
                }
            }
        }
    }
    if html.len() > 65536 {
        return String::new();
    }
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
    let mut out = String::new();
    visit(&dom.document, &mut out, 0);
    out
}

pub fn assistant_blocks(text: &str) -> Vec<AssistantBlock> {
    let mut blocks = vec![];
    let mut prose = String::new();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        let result_fence = line.trim_start().trim_end();
        let marker = if result_fence.starts_with('`') {
            '`'
        } else {
            '~'
        };
        let width = result_fence.chars().take_while(|c| *c == marker).count();
        if width >= 3
            && result_fence[width..].trim() == "wks-result"
            && line.len() - line.trim_start().len() <= 3
            && let Some(end) = (i + 1..lines.len()).find(|j| {
                let closing = lines[*j].trim();
                closing.len() >= width && closing.chars().all(|c| c == marker)
            })
        {
            if !prose.is_empty() {
                blocks.push(AssistantBlock::Markdown(std::mem::take(&mut prose)));
            }
            blocks.push(AssistantBlock::Result(lines[i + 1..end].concat()));
            i = end + 1;
            continue;
        }
        if line == "```wks-html-card"
            && let Some(end) = (i + 1..lines.len()).find(|j| lines[*j].trim_end() == "```")
            && let Some(card) = parse_card(&lines[i + 1..end].concat())
        {
            if !prose.is_empty() {
                blocks.push(AssistantBlock::Markdown(std::mem::take(&mut prose)));
            }
            blocks.push(AssistantBlock::Card(card));
            i = end + 1;
            continue;
        }
        // A fence containing an example card is ordinary code, not an action.
        if line.starts_with("```") || line.starts_with("~~~") {
            let marker = if line.starts_with('`') { '`' } else { '~' };
            let width = line.chars().take_while(|c| *c == marker).count();
            prose.push_str(lines[i]);
            i += 1;
            while i < lines.len() {
                prose.push_str(lines[i]);
                let closing = lines[i].trim();
                i += 1;
                if closing.chars().all(|c| c == marker) && closing.len() >= width {
                    break;
                }
            }
            continue;
        }
        prose.push_str(lines[i]);
        i += 1;
    }
    if !prose.is_empty() {
        blocks.push(AssistantBlock::Markdown(prose));
    }
    blocks
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fleet {
    pub title: String,
    pub entries: Vec<(String, String)>,
    pub summaries: Vec<String>,
    pub reports: Vec<(String, String)>,
    pub text: String,
}
pub fn fleet(text: &str) -> Option<Fleet> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let title = match header {
        "[fleet] Worker finished:" => "Worker finished",
        "[fleet] Worker FAILED — did not complete:" => "Worker failed",
        "[fleet] Worker escalated — blocked and did not complete:" => "Worker escalated",
        "[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:" => {
            "Worker catch-up"
        }
        "[supervisor] An agent is now blocked on a decision:" => "Decision needed",
        "[fleet] A threshold you asked to be told about has been crossed:" => "Threshold reached",
        "[fleet] Progress update from a worker — it is STILL RUNNING; this is NOT a completion:" => {
            "Worker progress · still running"
        }
        _ => return None,
    };
    let mut entries = vec![];
    let mut summaries = vec![];
    for line in lines.skip_while(|l| l.is_empty()) {
        let Some(bullet) = line.strip_prefix("- ") else {
            break;
        };
        let (label, rest) = bullet.split_once(" (session:")?;
        let (id, detail) = rest.split_once(", ")?;
        if !detail.contains(')')
            || id.is_empty()
            || id.len() > 200
            || !id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return None;
        }
        entries.push((label.into(), id.into()));
        summaries.push(bullet.to_owned());
    }
    if entries.is_empty() {
        return None;
    }
    // Keep every result/error/escalation/full-reply block, including malformed JSON.
    let mut reports = vec![];
    for block in text.split("\n\n").skip(1) {
        if block.starts_with("Full final message — ") {
            break;
        }
        if block.starts_with("Structured result") || block.starts_with("Worker escalation") {
            let (title, body) = block.split_once('\n').unwrap_or((block, ""));
            reports.push((title.into(), body.into()));
        }
    }
    Some(Fleet {
        title: title.into(),
        entries,
        summaries,
        reports,
        text: text.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn workspacer_spawn_receipts_link_only_successful_workspacer_sessions() {
        let mut tool = Tool {
            name: "mcp__workspacer__spawn_agent".into(),
            input: json!({"message":"Review parsing\nKeep scope bounded"}).to_string(),
            complete: true,
            ..Default::default()
        };
        assert_eq!(tool.target(), "Review parsing");
        for output in [
            json!({"sessionId":"child-1"}),
            json!({"structuredContent":{"sessionId":"child-1"}}),
            json!({"content":[{"type":"text","text":"{\"sessionId\":\"child-1\"}"}]}),
        ] {
            tool.output = output.to_string();
            assert_eq!(tool.spawned_session_id().as_deref(), Some("child-1"));
        }
        tool.output =
            json!({"isError":true, "structuredContent":{"sessionId":"child-1"}}).to_string();
        assert!(tool.spawned_session_id().is_none());
        tool.output = json!({"sessionId":"child-1"}).to_string();
        tool.is_error = true;
        assert!(tool.spawned_session_id().is_none());
        tool.is_error = false;
        tool.complete = false;
        assert!(tool.spawned_session_id().is_none());
        tool.complete = true;
        tool.name = "Agent".into();
        assert!(tool.spawned_session_id().is_none());
        tool.name = "spawn_agent".into();
        tool.output = json!({"agent_id":"provider-thread"}).to_string();
        assert!(tool.spawned_session_id().is_none());
    }

    #[test]
    fn raw_codex_patch_inputs_match_structured_patch_file_summaries() {
        let patch = "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n+extra\n*** Add File: src/helper.rs\n+helper\n*** End Patch";
        let expected = Tool::from_item(&Item {
            name: "apply_patch".into(),
            input: json!({"patch":patch}),
            ..Default::default()
        })
        .changes();
        assert_eq!(expected.len(), 2);
        assert_eq!((expected[0].added, expected[0].removed), (2, 1));
        assert_eq!((expected[1].added, expected[1].removed), (1, 0));
        for input in [json!(patch), json!({"input":patch})] {
            let tool = Tool::from_item(&Item {
                name: "functions.apply_patch".into(),
                input,
                ..Default::default()
            });
            assert_eq!(tool.changes(), expected);
            assert_eq!(
                tool.changes_with_diff(false),
                expected
                    .iter()
                    .map(|change| FileChange {
                        diff: String::new(),
                        ..change.clone()
                    })
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn unified_patch_boundaries_keep_deletions_and_headers_with_their_files() {
        let tool = Tool::from_item(&Item {
            name: "functions.apply_patch".into(),
            input: json!({"diff":"diff --git a/old.rs b/old.rs\n--- a/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\ndiff --git a/new.rs b/new.rs\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+added\ndiff --git a/before.rs b/after.rs\n--- a/before.rs\n+++ b/after.rs\n@@ -1 +1 @@\n-old\n+new"}),
            ..Default::default()
        });
        let changes = tool.changes();
        assert_eq!(changes.len(), 3);
        assert_eq!(
            (&*changes[0].path, changes[0].added, changes[0].removed),
            ("old.rs", 0, 1)
        );
        assert_eq!(
            (&*changes[1].path, changes[1].added, changes[1].removed),
            ("new.rs", 1, 0)
        );
        assert_eq!(
            (&*changes[2].path, changes[2].added, changes[2].removed),
            ("after.rs", 1, 1)
        );
        assert!(
            changes[0]
                .diff
                .starts_with("diff --git a/old.rs b/old.rs\n--- a/old.rs\n")
        );
        assert!(!changes[0].diff.contains("--- /dev/null"));
        assert!(
            changes[1]
                .diff
                .starts_with("diff --git a/new.rs b/new.rs\n--- /dev/null\n")
        );
        assert!(
            changes[2]
                .diff
                .starts_with("diff --git a/before.rs b/after.rs\n--- a/before.rs\n")
        );
    }

    #[test]
    fn namespaced_writes_and_multiedit_path_alias_keep_content() {
        let write = Tool::from_item(&Item {
            name: "mcp__files__Write".into(),
            input: json!({"path":"a.rs", "content":"one\ntwo"}),
            ..Default::default()
        });
        assert_eq!(write.changes()[0].added, 2);
        let edit = Tool::from_item(&Item {
            name: "MultiEdit".into(),
            input: json!({"path":"b.rs", "edits":[{"old_string":"old", "new_string":"new"}]}),
            ..Default::default()
        });
        assert_eq!(edit.changes()[0].path, "b.rs");
        assert_eq!(edit.changes()[0].removed, 1);
        let alias = Tool::from_item(&Item {
            name: "str_replace_editor".into(),
            input: json!({"filePath":"c.rs", "old_str":"old", "new_str":"one\ntwo"}),
            ..Default::default()
        });
        assert_eq!(alias.category(), "Edit");
        assert_eq!(alias.target(), "c.rs");
        assert_eq!(
            (alias.changes()[0].added, alias.changes()[0].removed),
            (2, 1)
        );
        let write_alias = Tool::from_item(&Item {
            name: "write_file".into(),
            input: json!({"filename":"d.rs", "file_text":"one\ntwo"}),
            ..Default::default()
        });
        assert_eq!(write_alias.changes()[0].added, 2);
    }

    #[test]
    fn consecutive_patch_headers_preserve_header_only_files_without_carrying_counts() {
        // A header is itself a retained diff line. The established full-diff
        // parser therefore keeps a header-only intermediate file, with zero
        // counts; summary mode must neither inherit counts nor lose that file.
        for prefix in ["*** Update File: ", "*** Add File: ", "+++ b/"] {
            let tool = Tool {
                name: "apply_patch".into(),
                input:
                    json!({"patch":format!("{prefix}a\n-old\n+new\n{prefix}b\n{prefix}c\n+added")})
                        .to_string(),
                complete: true,
                ..Default::default()
            };
            assert_eq!(
                tool.changes(),
                vec![
                    FileChange {
                        path: "a".into(),
                        added: 1,
                        removed: 1,
                        diff: format!("{prefix}a\n-old\n+new\n")
                    },
                    FileChange {
                        path: "b".into(),
                        added: 0,
                        removed: 0,
                        diff: format!("{prefix}b\n")
                    },
                    FileChange {
                        path: "c".into(),
                        added: 1,
                        removed: 0,
                        diff: format!("{prefix}c\n+added\n")
                    },
                ],
            );
            assert_eq!(
                turn_changes(
                    [&Row {
                        tool: Some(tool),
                        ..Default::default()
                    }]
                    .into_iter()
                ),
                vec![
                    FileChange {
                        path: "a".into(),
                        added: 1,
                        removed: 1,
                        ..Default::default()
                    },
                    FileChange {
                        path: "b".into(),
                        added: 0,
                        removed: 0,
                        ..Default::default()
                    },
                    FileChange {
                        path: "c".into(),
                        added: 1,
                        removed: 0,
                        ..Default::default()
                    },
                ],
            );
        }
    }
    #[test]
    fn turn_summaries_match_inline_changes_without_materializing_diffs() {
        let cases = [
            (
                "Edit",
                json!({"file_path":"a","old_string":"old\r\n二","new_string":"new\nthree\n四"}),
            ),
            (
                "MultiEdit",
                json!({"file_path":"a","edits":[{"old_string":"one","new_string":"two\nthree"},{"oldString":"four\nfive","newString":"six"}]}),
            ),
            ("Write", json!({"path":"b","content":"one\ntwo\n"})),
            (
                "apply_patch",
                json!({"patch":"*** Update File: c\n@@\n-old\n+new\n*** Add File: d\n+added\n*** Delete File: e\n-gone"}),
            ),
            (
                "patch",
                json!({"diff":"--- a/f\n+++ b/f\n context only\n--- a/g\n+++ b/g\n-old\n+new"}),
            ),
            (
                "Edit",
                json!({"changes":[{"path":"h","diff":"+added"},{"path":"i","oldString":"removed","newString":""}]}),
            ),
        ];
        let mut expected = BTreeMap::<String, FileChange>::new();
        let mut rows = Vec::new();
        for (name, input) in cases {
            let mut tool = Tool::from_item(&Item {
                name: name.into(),
                input,
                ..Default::default()
            });
            tool.complete = true;
            let full = tool.changes();
            let summaries = tool.changes_with_diff(false);
            assert_eq!(full.len(), summaries.len());
            for (change, summary) in full.iter().zip(&summaries) {
                assert!(!change.diff.is_empty());
                assert!(summary.diff.is_empty());
                assert_eq!(
                    (&change.path, change.added, change.removed),
                    (&summary.path, summary.added, summary.removed)
                );
                let total = expected
                    .entry(change.path.clone())
                    .or_insert_with(|| FileChange {
                        path: change.path.clone(),
                        ..Default::default()
                    });
                total.added += change.added;
                total.removed += change.removed;
            }
            rows.push(Row {
                tool: Some(tool.clone()),
                ..Default::default()
            });
            tool.is_error = true;
            rows.push(Row {
                tool: Some(tool.clone()),
                ..Default::default()
            });
            tool.is_error = false;
            tool.complete = false;
            rows.push(Row {
                tool: Some(tool),
                ..Default::default()
            });
        }
        assert_eq!(
            turn_changes(rows.iter()),
            expected.into_values().collect::<Vec<_>>()
        );
        assert_eq!(
            turn_changes(rows.iter())
                .iter()
                .find(|f| f.path == "a")
                .map(|f| (f.added, f.removed)),
            Some((6, 5))
        );
    }
    #[test]
    fn edits_include_multifile_patches_and_failures_are_not_changes() {
        let mut tool = Tool::from_item(&Item {
            name: "apply_patch".into(),
            input: json!({"changes":[{"path":"a","diff":"@@\n-old\n+new"},{"path":"b","diff":"+other"}]}),
            ..Default::default()
        });
        assert_eq!(tool.changes().len(), 2);
        assert_eq!((tool.changes()[0].added, tool.changes()[0].removed), (1, 1));
        tool.is_error = true;
        assert!(tool.changes().is_empty());
    }
    #[test]
    fn image_markers_preserve_other_attachments_and_resolve_remote_windows_paths() {
        let paths = image_paths(
            "[Image: screen.png] [PDF: x.pdf] [Image: x.svg]",
            true,
            "C:\\repo",
        );
        assert_eq!(
            paths,
            vec![("[Image: screen.png]".into(), "C:\\repo\\screen.png".into())]
        );
        assert_eq!(image_paths("See `/tmp/plot.png`.", false, "").len(), 1);
    }
    #[test]
    fn cards_only_activate_closed_top_level_valid_fences() {
        let raw = json!({"v":1,"title":"Choice","bodyHtml":"<script>bad()</script>","fallback":"Pick one","actions":[{"kind":"fill_composer","label":"Continue","text":"Go"}]}).to_string();
        let text = format!("Before\n```wks-html-card\n{raw}\n```\nAfter");
        let blocks = assistant_blocks(&text);
        assert!(matches!(&blocks[1], AssistantBlock::Card(c) if c.actions.len() == 1));
        assert!(
            assistant_blocks(&format!("````markdown\n{text}\n````"))
                .iter()
                .all(|b| matches!(b, AssistantBlock::Markdown(_)))
        );
        assert!(
            assistant_blocks(&format!("```wks-html-card\n{raw}"))
                .iter()
                .all(|b| matches!(b, AssistantBlock::Markdown(_)))
        );
        let invalid = raw.replace("fill_composer", "shell");
        assert!(
            matches!(&assistant_blocks(&format!("```wks-html-card\n{invalid}\n```"))[0], AssistantBlock::Card(c) if c.actions.is_empty())
        );
    }
    #[test]
    fn native_html_keeps_tables_but_has_no_network_or_script_attributes() {
        let html = native_card_html(
            "<div onclick='bad()'><script>secret</script><table><tr><td>A</td><td>B</td></tr></table><img src='https://example.com'><a href='javascript:bad()'>link</a></div>",
        );
        assert!(html.contains("<td>A</td>"));
        assert!(html.contains("link"));
        for forbidden in [
            "script",
            "secret",
            "onclick",
            "https:",
            "javascript:",
            "img",
            "href",
        ] {
            assert!(!html.contains(forbidden), "{html}");
        }
    }
    #[test]
    fn literal_entities_survive_the_native_html_double_parse_boundary() {
        use html5ever::tendril::TendrilSink;
        use markup5ever_rcdom::{Handle, NodeData, RcDom};
        // Reproduce the component's minifier contract: parsed text is written
        // verbatim, then the result is parsed again by its native HTML renderer.
        fn minified(node: &Handle) -> String {
            match &node.data {
                NodeData::Text { contents } => contents.borrow().to_string(),
                NodeData::Element { name, .. } => format!(
                    "<{}>{}</{}>",
                    name.local,
                    node.children
                        .borrow()
                        .iter()
                        .map(minified)
                        .collect::<String>(),
                    name.local
                ),
                _ => node.children.borrow().iter().map(minified).collect(),
            }
        }
        fn text_only(node: &Handle, text: &mut String) {
            match &node.data {
                NodeData::Text { contents } => text.push_str(&contents.borrow()),
                NodeData::Element { name, attrs, .. } => {
                    assert!(!matches!(name.local.as_ref(), "img" | "script" | "iframe"));
                    assert!(attrs.borrow().is_empty());
                }
                _ => {}
            }
            for child in node.children.borrow().iter() {
                text_only(child, text);
            }
        }
        let original = "Keep <tags>, &lt;entities&gt;, **stars**, <img src=https://example.com>, and <script>alert(1)</script>.";
        let html = format!("<pre>{}</pre>", escape_native_html_text(original));
        let first = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
        let second = html5ever::parse_document(RcDom::default(), Default::default())
            .one(minified(&first.document));
        let mut rendered = String::new();
        text_only(&second.document, &mut rendered);
        assert_eq!(rendered, original);
        let card = native_card_html("<p>&lt;img src=https://example.com&gt;</p>");
        let first = html5ever::parse_document(RcDom::default(), Default::default()).one(card);
        let second = html5ever::parse_document(RcDom::default(), Default::default())
            .one(minified(&first.document));
        let mut rendered = String::new();
        text_only(&second.document, &mut rendered);
        assert_eq!(rendered, "<img src=https://example.com>");
    }

    #[test]
    fn result_cards_preserve_prose_invalid_payloads_and_example_fences() {
        let blocks = assistant_blocks("Done\n```wks-result\n{\"ok\":true}\n```\nAfter");
        assert!(
            matches!(&blocks[..], [AssistantBlock::Markdown(_), AssistantBlock::Result(raw), AssistantBlock::Markdown(_)] if raw.contains("true"))
        );
        assert!(
            matches!(&assistant_blocks("~~~wks-result\ninvalid\n~~~")[0], AssistantBlock::Result(raw) if raw == "invalid\n")
        );
        assert!(
            assistant_blocks("````markdown\n```wks-result\n{}\n```\n````")
                .iter()
                .all(|b| matches!(b, AssistantBlock::Markdown(_)))
        );
        assert!(matches!(
            &assistant_blocks("```wks-result\n{")[0],
            AssistantBlock::Markdown(_)
        ));
    }

    #[test]
    fn fleet_wakes_preserve_results_and_never_misidentify_cwd_as_session_id() {
        let text = "[fleet] Worker finished:\n- Builder (session:child-1, cwd /repo) — last reply: Done\n\nStructured result — Builder (session:child-1):\n{\"ok\":true}\n\nFull final message — Builder (session:child-1):\nAll the details";
        let message = fleet(text).unwrap();
        assert_eq!(message.entries, vec![("Builder".into(), "child-1".into())]);
        assert_eq!(message.text, text);
        assert!(fleet("[fleet] Worker finished:\n- malformed").is_none());
    }
    #[test]
    fn markdown_file_links_resolve_against_the_remote_project() {
        assert_eq!(
            file_paths(
                "[code](src/lib.rs:12) [web](https://example.com) [win](<C:\\repo\\a.rs:7>)",
                "/remote"
            ),
            vec!["/remote/src/lib.rs", "C:\\repo\\a.rs"]
        );
    }
    #[test]
    fn skill_target_is_not_argument_order_dependent() {
        let tool = Tool::from_item(&Item {
            name: "Skill".into(),
            input: json!({"args":"secret task", "skill":"review"}),
            ..Default::default()
        });
        assert_eq!(tool.target(), "review");
    }
}
