use super::super::{files, fleet_messages::clip, paths, pricing::Pricing};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::SystemTime,
};
fn number(value: f64) -> Value {
    if value.is_finite()
        && value.fract() == 0.
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        json!(value as i64)
    } else {
        json!(value)
    }
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn copy(from: &Value, to: &mut Value, keys: &[&str]) {
    for key in keys {
        if let Some(v) = from.get(*key) {
            to[*key] = v.clone();
        }
    }
}
fn names(root: &Path, dir: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::canonicalize(dir) else {
        return vec![];
    };
    let Ok(root) = std::fs::canonicalize(root) else {
        return vec![];
    };
    if !paths::contained(&dir, &root) {
        return vec![];
    }
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}
fn bytes(root: &Path, path: &Path) -> Option<Vec<u8>> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    if !paths::contained(&canonical, &root) {
        return None;
    }
    files::bounded_bytes(&canonical, 64 * 1024 * 1024).ok()
}
fn object(root: &Path, path: &Path) -> Option<Value> {
    serde_json::from_slice(&bytes(root, path)?)
        .ok()
        .filter(|v: &Value| match v {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64() != Some(0.),
            Value::String(s) => !s.is_empty(),
            _ => true,
        })
}
const TAIL_CHUNK: usize = 16 * 1024 * 1024;
const TAIL_LINE: usize = 64 * 1024 * 1024;
fn appended(root: &Path, path: &Path, offset: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let canonical = std::fs::canonicalize(path).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    if !paths::contained(&canonical, &root) {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(&canonical).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return None;
    }
    let current = options.open(&canonical).ok()?;
    if files::file_identity(&file).ok()? != files::file_identity(&current).ok()?
        || std::fs::canonicalize(&canonical).ok()? != canonical
    {
        return None;
    }
    if meta.len() <= offset {
        return Some(vec![]);
    }
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take((meta.len() - offset).min(TAIL_CHUNK as u64))
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}
#[derive(Default)]
struct Tail {
    offset: u64,
    remainder: Vec<u8>,
    discarding: bool,
}
impl Tail {
    fn lines(&mut self, root: &Path, path: &Path) -> Vec<String> {
        let Some(mut raw) = appended(root, path, self.offset) else {
            return vec![];
        };
        if raw.is_empty() {
            return vec![];
        }
        self.offset += raw.len() as u64;
        if self.discarding {
            let Some(end) = raw.iter().position(|b| *b == b'\n') else {
                return vec![];
            };
            raw.drain(..=end);
            self.discarding = false;
        }
        self.remainder.extend(raw);
        let Some(end) = self.remainder.iter().rposition(|b| *b == b'\n') else {
            if self.remainder.len() > TAIL_LINE {
                self.remainder.clear();
                self.discarding = true;
            }
            return vec![];
        };
        let all = std::mem::take(&mut self.remainder);
        self.remainder = all[end + 1..].to_vec();
        all[..end]
            .split(|b| *b == b'\n')
            .filter(|line| line.len() <= TAIL_LINE)
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .filter(|line| !super::super::dispatch_templates::trim_js(line).is_empty())
            .collect()
    }
}
#[derive(Default)]
struct Agent {
    info: Value,
    tail: Tail,
    last_usage: Option<Value>,
    mtime: Option<SystemTime>,
}
fn summary(name: &str, input: &Value) -> String {
    if !input.is_object() {
        return String::new();
    }
    let value = match name {
        "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            return text(input, "file_path")
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .into();
        }
        "Bash" | "PowerShell" => input
            .get("description")
            .filter(|v| !v.is_null())
            .unwrap_or(&input["command"]),
        "Grep" | "Glob" => &input["pattern"],
        "Agent" => &input["description"],
        _ => input
            .as_object()
            .unwrap()
            .values()
            .find(|v| v.is_string())
            .unwrap_or(&Value::Null),
    };
    let string = value.as_str().map(str::to_owned).unwrap_or_else(|| {
        if value.is_null() {
            String::new()
        } else {
            value.to_string()
        }
    });
    clip(string.split('\n').next().unwrap_or(""), 60, "")
}
fn apply(entry: &Value, stats: &mut Value, key: &mut Option<Value>, pricing: &Pricing) {
    if stats["startedAt"].as_i64().unwrap_or(0) == 0 {
        if let Some(ts) = entry["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        {
            stats["startedAt"] = ts.timestamp_millis().into();
        }
    }
    let Some(msg) = entry.get("message").filter(|v| !v.is_null()) else {
        return;
    };
    if entry["type"] == "user" && text(stats, "promptPreview").is_empty() {
        let content = msg["content"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                msg["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        if !content.is_empty() {
            stats["promptPreview"] = clip(&content, 160, "").into();
        }
    } else if entry["type"] == "assistant" {
        if !text(msg, "model").is_empty() {
            stats["model"] = msg["model"].clone();
        }
        if msg["usage"].is_object() {
            let id = msg
                .get("id")
                .filter(|v| !v.is_null())
                .or_else(|| entry.get("uuid").filter(|v| !v.is_null()));
            if let Some(id) = id.filter(|id| key.as_ref() != Some(*id)) {
                *key = Some(id.clone());
                stats["tokens"] = number(
                    stats["tokens"].as_f64().unwrap_or(0.)
                        + msg["usage"]["output_tokens"].as_f64().unwrap_or(0.),
                );
                stats["costUSD"] = json!(
                    stats["costUSD"].as_f64().unwrap_or(0.)
                        + pricing.turn_cost(
                            msg["model"].as_str().or_else(|| stats["model"].as_str()),
                            &msg["usage"]
                        )
                );
            }
        }
        for block in msg["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_use")
        {
            stats["toolCalls"] = json!(stats["toolCalls"].as_u64().unwrap_or(0) + 1);
            stats["lastToolName"] = block
                .get("name")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!("tool"));
            stats["lastToolSummary"] = summary(text(block, "name"), &block["input"]).into();
        }
    }
}
// serde_json's default map sorts keys. Tool summaries follow JavaScript's
// Object.values ordering, so retain source entry order only for that fallback.
fn ordered_tool_summary(raw: &str, entry: &Value) -> Option<String> {
    if entry["type"] != "assistant" {
        return None;
    }
    let last = entry["message"]["content"]
        .as_array()?
        .iter()
        .filter(|b| b["type"] == "tool_use")
        .last()?;
    if matches!(
        text(last, "name"),
        "Read"
            | "Edit"
            | "MultiEdit"
            | "Write"
            | "NotebookEdit"
            | "Bash"
            | "PowerShell"
            | "Grep"
            | "Glob"
            | "Agent"
    ) {
        return None;
    }
    let ordered: serde_yaml::Value = serde_json::from_str(raw).ok()?;
    let last = ordered["message"]["content"]
        .as_sequence()?
        .iter()
        .filter(|b| b["type"].as_str() == Some("tool_use"))
        .last()?;
    let input = last["input"].as_mapping()?;
    let mut values: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(position, (key, value))| {
            let key = key.as_str().unwrap_or("");
            let index = key
                .parse::<u32>()
                .ok()
                .filter(|index| *index < u32::MAX && index.to_string() == key);
            (index, position, value)
        })
        .collect();
    values.sort_by_key(|(index, position, _)| match index {
        Some(index) => (0, *index as usize),
        None => (1, *position),
    });
    let first = values
        .into_iter()
        .find_map(|(_, _, value)| value.as_str())
        .unwrap_or("");
    Some(clip(first.split('\n').next().unwrap_or(""), 60, ""))
}
impl Agent {
    fn refresh(&mut self, root: &Path, path: &Path, pricing: &Pricing, plain: bool) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        if self.mtime == Some(modified) && metadata.len() <= self.tail.offset {
            return false;
        }
        self.mtime = Some(modified);
        let lines = self.tail.lines(root, path);
        if lines.is_empty() {
            return false;
        }
        if self.info["status"] == "queued" {
            self.info["status"] = "running".into();
        }
        for line in lines {
            if let Ok(value) = serde_json::from_str(&line) {
                apply(&value, &mut self.info, &mut self.last_usage, pricing);
                if let Some(summary) = ordered_tool_summary(&line, &value) {
                    self.info["lastToolSummary"] = summary.into();
                }
            }
        }
        if plain {
            self.info.as_object_mut().unwrap().remove("promptPreview");
            self.info.as_object_mut().unwrap().remove("startedAt");
        }
        true
    }
}
struct Run {
    info: Value,
    dir: PathBuf,
    agents: BTreeMap<String, Agent>,
    order: Vec<String>,
    journal: Tail,
    finalized: bool,
}
pub fn script_meta(source: &str) -> Option<Value> {
    let re = regex::Regex::new(r"export\s+const\s+meta\s*=\s*\{").unwrap();
    let found = re.find(source)?;
    let start = found.end() - 1;
    let mut depth = 0;
    let mut quote = None;
    let mut escape = false;
    let mut end = None;
    for (offset, c) in source[start..].char_indices() {
        if quote.is_some() {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if Some(c) == quote {
                quote = None;
            }
            continue;
        }
        if matches!(c, '\'' | '"' | '`') {
            quote = Some(c);
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = Some(start + offset + 1);
                break;
            }
        }
    }
    let literal: serde_yaml::Value = json5::from_str(&source[start..end?]).ok()?;
    let value = serde_json::to_value(literal).ok()?;
    if !value.is_object() {
        return None;
    }
    let mut output = json!({});
    for key in ["name", "description"] {
        if value[key].is_string() {
            output[key] = value[key].clone();
        }
    }
    if value["phases"].is_array() {
        output["phases"] = phases(&value["phases"]);
    }
    Some(output)
}
fn phases(value: &Value) -> Value {
    json!(
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["title"].is_string())
            .map(|v| {
                let mut p = json!({"title":v["title"]});
                if v["detail"].is_string() {
                    p["detail"] = v["detail"].clone();
                }
                p
            })
            .collect::<Vec<_>>()
    )
}
impl Run {
    fn new(root: &Path, dir: PathBuf, id: &str, now: i64) -> Self {
        let started = std::fs::metadata(&dir)
            .ok()
            .and_then(|m| m.created().or_else(|_| m.modified()).ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|t| t.as_millis() as i64)
            .unwrap_or(now);
        let mut info =
            json!({"runId":id,"status":"running","startedAt":started,"phases":[],"agents":[]});
        for name in names(root, &root.join("workflows/scripts")) {
            if let Some(base) = name.strip_suffix(&format!("-{id}.js")) {
                info["name"] = base.into();
                if let Some(raw) = bytes(root, &root.join("workflows/scripts").join(&name)) {
                    if let Some(meta) = script_meta(&String::from_utf8_lossy(&raw)) {
                        copy(&meta, &mut info, &["description", "phases"]);
                        if !text(&meta, "name").is_empty() {
                            info["name"] = meta["name"].clone();
                        }
                    }
                }
                break;
            }
        }
        Self {
            info,
            dir,
            agents: BTreeMap::new(),
            order: vec![],
            journal: Tail::default(),
            finalized: false,
        }
    }
    fn adopt(&mut self, root: &Path) -> bool {
        let Some(value) = object(
            root,
            &root
                .join("workflows")
                .join(format!("{}.json", text(&self.info, "runId"))),
        ) else {
            return false;
        };
        self.info["status"] = if value["status"] == "completed" {
            "completed"
        } else {
            "failed"
        }
        .into();
        if value["workflowName"].is_string() {
            self.info["name"] = value["workflowName"].clone();
        }
        if value["startTime"].is_number() {
            self.info["startedAt"] = value["startTime"].clone();
        }
        if value["durationMs"].is_number() {
            self.info["durationMs"] = value["durationMs"].clone();
            self.info["completedAt"] = number(
                self.info["startedAt"].as_f64().unwrap_or(0.)
                    + value["durationMs"].as_f64().unwrap(),
            );
        }
        if value["phases"].is_array() {
            self.info["phases"] = phases(&value["phases"]);
        }
        for key in ["totalTokens", "totalToolCalls"] {
            if value[key].is_number() {
                self.info[key] = value[key].clone();
            }
        }
        let mut adopted = Vec::new();
        for row in value["workflowProgress"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["type"] == "workflow_agent" && !text(r, "agentId").is_empty())
        {
            let id = text(row, "agentId")
                .strip_prefix("agent-")
                .unwrap_or(text(row, "agentId"));
            let mut info = json!({"id":id,"status":if matches!(text(row,"state"),"error"|"failed"){"failed"}else{"done"},"tokens":row.get("tokens").filter(|v|v.is_number()).cloned().unwrap_or(json!(0)),"toolCalls":row.get("toolCalls").filter(|v|v.is_number()).cloned().unwrap_or(json!(0))});
            if let Some(cost) = self.agents.get(id).and_then(|a| a.info.get("costUSD")) {
                info["costUSD"] = cost.clone();
            }
            for key in [
                "label",
                "phaseTitle",
                "model",
                "lastToolName",
                "lastToolSummary",
            ] {
                if row[key].is_string() {
                    info[key] = row[key].clone();
                }
            }
            for key in ["startedAt", "durationMs"] {
                if row[key].is_number() {
                    info[key] = row[key].clone();
                }
            }
            if row["startedAt"].is_number() && row["durationMs"].is_number() {
                info["completedAt"] = number(
                    row["startedAt"].as_f64().unwrap() + row["durationMs"].as_f64().unwrap(),
                );
            }
            for (key, max) in [("promptPreview", 160), ("resultPreview", 200)] {
                if let Some(s) = row[key].as_str() {
                    info[key] = clip(s, max, "").into();
                }
            }
            adopted.push((id.to_owned(), info));
        }
        if !adopted.is_empty() {
            self.agents.clear();
            self.order.clear();
            for (id, info) in adopted {
                self.order.push(id.clone());
                self.agents.insert(
                    id,
                    Agent {
                        info,
                        ..Default::default()
                    },
                );
            }
        }
        self.finalized = true;
        true
    }
    fn refresh(&mut self, root: &Path, pricing: &Pricing, now: i64) -> bool {
        if self.finalized {
            return false;
        }
        if self.adopt(root) {
            return true;
        }
        let mut dirty = false;
        for name in names(root, &self.dir) {
            if let Some(id) = name
                .strip_prefix("agent-")
                .and_then(|s| s.strip_suffix(".meta.json"))
            {
                if !self.agents.contains_key(id) {
                    self.order.push(id.into());
                    self.agents.insert(
                        id.into(),
                        Agent {
                            info: json!({"id":id,"status":"queued","tokens":0,"toolCalls":0}),
                            ..Default::default()
                        },
                    );
                    dirty = true;
                }
            }
        }
        for (id, agent) in &mut self.agents {
            dirty |= agent.refresh(
                root,
                &self.dir.join(format!("agent-{id}.jsonl")),
                pricing,
                false,
            );
        }
        for line in self.journal.lines(root, &self.dir.join("journal.jsonl")) {
            let Ok(row) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = text(&row, "agentId")
                .strip_prefix("agent-")
                .unwrap_or(text(&row, "agentId"));
            let Some(agent) = self.agents.get_mut(id) else {
                continue;
            };
            if row["type"] == "started" && agent.info["status"] == "queued" {
                agent.info["status"] = "running".into();
                dirty = true;
            } else if row["type"] == "result" {
                agent.info["status"] = "done".into();
                agent.info["completedAt"] = now.into();
                if let Some(start) = agent.info["startedAt"].as_i64().filter(|s| *s != 0) {
                    agent.info["durationMs"] = (now - start).into();
                }
                if let Some(result) = row
                    .get("result")
                    .filter(|_| agent.info.get("resultPreview").is_none())
                {
                    agent.info["resultPreview"] = clip(
                        &result
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| result.to_string()),
                        200,
                        "",
                    )
                    .into();
                }
                dirty = true;
            }
        }
        dirty
    }
    fn snapshot(&self) -> Value {
        let mut info = self.info.clone();
        let agents: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.agents.get(id))
            .map(|a| a.info.clone())
            .collect();
        let cost: f64 = agents.iter().filter_map(|a| a["costUSD"].as_f64()).sum();
        if cost > 0. {
            info["totalCostUSD"] = json!(cost);
        }
        info["agents"] = json!(agents);
        info
    }
}
pub struct Watch {
    pub root: PathBuf,
    runs: BTreeMap<String, Run>,
    order: Vec<String>,
    plain: BTreeMap<String, Agent>,
    pub last_poke: i64,
}
impl Watch {
    pub fn new(root: PathBuf, now: i64) -> Self {
        Self {
            root,
            runs: BTreeMap::new(),
            order: vec![],
            plain: BTreeMap::new(),
            last_poke: now,
        }
    }
    pub fn active(&self, now: i64) -> bool {
        now - self.last_poke < 60_000 || self.runs.values().any(|r| !r.finalized)
    }
    pub fn refresh(&mut self, pricing: &Pricing, now: i64) -> Option<Value> {
        let mut dirty = false;
        let runs = self.root.join("subagents/workflows");
        for name in names(&self.root, &runs) {
            let dir = runs.join(&name);
            if name.starts_with("wf_") && dir.is_dir() && !self.runs.contains_key(&name) {
                self.order.push(name.clone());
                self.runs
                    .insert(name.clone(), Run::new(&self.root, dir, &name, now));
                dirty = true;
            }
        }
        for run in self.runs.values_mut() {
            dirty |= run.refresh(&self.root, pricing, now);
        }
        let plain = self.root.join("subagents");
        for name in names(&self.root, &plain) {
            if let Some(id) = name
                .strip_prefix("agent-")
                .and_then(|s| s.strip_suffix(".meta.json"))
            {
                if !self.plain.contains_key(id) {
                    let meta = object(&self.root, &plain.join(&name)).unwrap_or(json!({}));
                    let mut info = json!({"tokens":0,"toolCalls":0});
                    for key in ["agentType", "description", "toolUseId"] {
                        if meta[key].is_string() {
                            info[key] = meta[key].clone();
                        }
                    }
                    self.plain.insert(
                        id.into(),
                        Agent {
                            info,
                            ..Default::default()
                        },
                    );
                    dirty = true;
                }
            }
        }
        for (id, agent) in &mut self.plain {
            dirty |= agent.refresh(
                &self.root,
                &plain.join(format!("agent-{id}.jsonl")),
                pricing,
                true,
            );
        }
        dirty.then(|| self.snapshot())
    }
    pub fn snapshot(&self) -> Value {
        let mut runs: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.runs.get(id))
            .map(Run::snapshot)
            .collect();
        runs.sort_by(|a, b| {
            a["startedAt"]
                .as_f64()
                .unwrap_or(0.)
                .total_cmp(&b["startedAt"].as_f64().unwrap_or(0.))
        });
        let runs = runs
            .into_iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let ids: Vec<_> = runs
            .iter()
            .flat_map(|r| r["agents"].as_array().into_iter().flatten())
            .map(|a| a["id"].clone())
            .collect();
        let activity: BTreeMap<_, _> = self
            .plain
            .iter()
            .map(|(id, a)| (id.clone(), a.info.clone()))
            .collect();
        json!({"runs":runs,"workflowAgentIds":ids,"subagentActivity":activity})
    }
}

#[cfg(test)]
mod tests {
    use super::super::telemetry::Telemetry;
    use super::*;
    #[test]
    fn exact_typescript_tailing_final_adoption_last_three_and_telemetry_corpus() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("session");
        std::fs::create_dir(&root).unwrap();
        let pricing = Pricing::new(dir.path().into());
        let mut watch = Watch::new(root.clone(), 1767225600000);
        let mut telemetry = Telemetry::default();
        let steps: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/workflow-watcher.json"
        ))
        .unwrap();
        for (index, step) in steps.as_array().unwrap().iter().enumerate() {
            let now = step["now"].as_i64().unwrap();
            for file in step["files"].as_array().unwrap() {
                let path = root.join(file["path"].as_str().unwrap());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let mut output = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(file["append"] == true)
                    .truncate(file["append"] != true)
                    .open(path)
                    .unwrap();
                output
                    .write_all(file["content"].as_str().unwrap().as_bytes())
                    .unwrap();
                output
                    .set_modified(
                        SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(now as u64),
                    )
                    .unwrap();
            }
            let changed = watch.refresh(&pricing, now).is_some();
            for run in watch.runs.values_mut() {
                run.info["startedAt"] = json!(1767225600000i64);
            }
            let update = watch.snapshot();
            assert_eq!(changed, step["changed"].as_bool().unwrap(), "step{index}");
            assert_eq!(update, step["update"], "step{index}");
            let events = if changed {
                telemetry.update("session", "/project", &update["runs"])
            } else {
                vec![]
            };
            assert_eq!(json!(events), step["events"], "events step{index}");
        }
        assert_eq!(watch.runs.len(), 4);
        assert!(watch.runs["wf_a"].finalized);
        assert_eq!(watch.runs["wf_a"].info["name"], "Final name");
        assert!(
            telemetry
                .update("session", "/project", &watch.snapshot()["runs"])
                .is_empty()
        );
        telemetry.forget("session");
        assert!(
            !telemetry
                .update("session", "/project", &watch.snapshot()["runs"])
                .is_empty()
        );
    }
    #[test]
    fn metadata_literal_contract_never_evaluates_computed_code() {
        assert_eq!(script_meta("export const meta = {name:'Name', phases:[{title:'One', detail:'Details'}, {other:1}],}").unwrap(),json!({"name":"Name","phases":[{"title":"One","detail":"Details"}]}));
        assert_eq!(
            script_meta("export const meta = {name:'Literal', unused:Infinity}").unwrap(),
            json!({"name":"Literal"})
        );
        assert!(script_meta("export const meta = {name: (() => 'computed')()}").is_none());
        assert!(script_meta("export const meta = {name: process.env.HUB_TOKEN}").is_none());
        assert!(script_meta("const other = {}").is_none());
    }
}
