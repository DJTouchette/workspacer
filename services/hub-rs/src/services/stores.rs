use super::{config::atomic_bytes, layout::scrub_saved_document, paths};
use crate::Options;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
mod yaml_strings;

pub fn slug(input: &str, variant: &str) -> String {
    let mut output = String::new();
    let mut previous_bad = false;
    for c in input.chars() {
        let c = c.to_ascii_lowercase();
        let bad = !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if variant == "library" {
            if bad {
                if !previous_bad {
                    output.push('-');
                }
            } else {
                output.push(c);
            }
            previous_bad = bad;
        } else {
            let c = if bad { '-' } else { c };
            if c != '-' || !output.ends_with('-') {
                output.push(c);
            }
        }
    }
    if variant != "session" {
        output = output.trim_matches('-').into();
    }
    if variant != "library" {
        output.truncate(output.len().min(64));
    }
    if variant != "session" {
        output = output.trim_matches('-').into();
    }
    if output.is_empty() {
        match variant {
            "library" => "item".into(),
            "layout" => "layout".into(),
            _ => output,
        }
    } else {
        output
    }
}
fn now_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond()
    )
}
pub struct Stores {
    directory: PathBuf,
    lock: Mutex<()>,
}
impl Stores {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            lock: Mutex::new(()),
        }
    }
    fn folder(&self, kind: &str) -> PathBuf {
        self.directory.join(kind)
    }
    fn layout_path(&self, id: &str) -> Result<PathBuf> {
        if id.contains(['/', '\\']) || id.contains("..") {
            bail!("layout id must not contain a path separator");
        }
        paths::selected_path(
            &self.folder("layouts"),
            &format!("{}.yaml", slug(id, "layout")),
        )
    }
    fn read(path: &Path) -> Option<Value> {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_yaml::from_slice::<Value>(&bytes).ok())
            .filter(|v| v.is_object() || v.is_null())
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let _lock = self.lock.lock().unwrap();
        match method {
            "layouts.list" => Ok(Value::Array(self.list(true))),
            "sessions.list" => Ok(Value::Array(self.list(false))),
            "layouts.save" => {
                let input = &params;
                let name = input["name"].as_str().unwrap_or("").trim();
                let id = input["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| slug(name, "layout"));
                let path = self.layout_path(&id)?;
                let id = slug(&id, "layout");
                let mut layout = json!({"id":id,"name":if name.is_empty(){id.as_str()}else{name},"createdAt":now_iso(),"agents":input.get("agents").filter(|v|!v.is_null()).cloned().unwrap_or(json!([]))});
                scrub_saved_document(&mut layout);
                atomic_bytes(&path, &yaml_strings::encode(&layout, "createdAt")?)?;
                Ok(layout)
            }
            "layouts.delete" => {
                let id = params["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("layouts.delete requires {{ id }}"))?;
                if let Ok(path) = self.layout_path(id) {
                    let _ = std::fs::remove_file(path);
                }
                Ok(json!({"ok":true}))
            }
            "sessions.load" | "sessions.delete" => {
                let name = params["filename"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("{method} requires {{ filename }}"))?;
                let path = paths::selected_path(&self.folder("sessions"), name);
                if method == "sessions.load" {
                    return Ok(path
                        .ok()
                        .and_then(|p| Self::read(&p))
                        .unwrap_or(Value::Null));
                }
                if let Ok(path) = path {
                    let _ = std::fs::remove_file(path);
                }
                Ok(json!({"ok":true}))
            }
            "sessions.save" => {
                let name = params["name"].as_str().unwrap_or("");
                let base = slug(name, "session");
                let mut filename = format!("{base}.yaml");
                let mut index = 2;
                let path = loop {
                    let path = paths::selected_path(&self.folder("sessions"), &filename)?;
                    match std::fs::read(&path) {
                        Err(_) => break path,
                        Ok(bytes) => {
                            if serde_yaml::from_slice::<Value>(&bytes)
                                .is_ok_and(|v| v["name"] == name)
                            {
                                break path;
                            }
                        }
                    }
                    filename = format!("{base}-{index}.yaml");
                    index += 1;
                };
                let mut data = json!({"name":name,"timestamp":now_iso(),"schemaVersion":1});
                if params["agents"].is_array() {
                    data["agents"] = params["agents"].clone();
                    if !params["activeAgentId"].is_null() {
                        data["activeAgentId"] = params["activeAgentId"].clone();
                    }
                    scrub_saved_document(&mut data);
                } else {
                    data["tabs"] = params
                        .get("tabs")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .unwrap_or(json!([]));
                    if !params["activeTabId"].is_null() {
                        data["activeTabId"] = params["activeTabId"].clone();
                    }
                }
                atomic_bytes(&path, &yaml_strings::encode(&data, "timestamp")?)?;
                Ok(json!(filename))
            }
            _ => bail!("unknown saved-state method"),
        }
    }
    fn list(&self, layouts: bool) -> Vec<Value> {
        let folder = self.folder(if layouts { "layouts" } else { "sessions" });
        let _ = std::fs::create_dir_all(&folder);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return vec![];
        };
        let mut names: Vec<_> = entries
            .flatten()
            .filter(|e| !e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|s| s.ends_with(".yaml"))
            .collect();
        names.sort();
        let mut result = Vec::new();
        for name in names {
            let Ok(path) = paths::selected_path(&folder, &name) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let mut data = match serde_yaml::from_slice::<Value>(&bytes) {
                Ok(v) if v.is_object() || v.is_null() => v,
                _ => {
                    quarantine(&path, &bytes);
                    continue;
                }
            };
            let Some(timestamp_text) = yaml_strings::list_timestamp(
                &bytes,
                &data,
                if layouts { "createdAt" } else { "timestamp" },
            ) else {
                continue;
            };
            if layouts {
                if data["agents"].is_array() {
                    if data["createdAt"].is_string() {
                        data["createdAt"] = json!(timestamp_text);
                    }
                    result.push(data);
                }
            } else {
                let display = data["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(name.trim_end_matches(".yaml"));
                let agents = data["agents"].as_array();
                let mut pane_count = 0;
                let count_tabs = |v: &Value| {
                    v.as_array()
                        .map(|tabs| {
                            tabs.iter()
                                .map(|t| t["panes"].as_array().map(Vec::len).unwrap_or(0))
                                .sum::<usize>()
                        })
                        .unwrap_or(0)
                };
                if let Some(agents) = agents.filter(|a| !a.is_empty()) {
                    for agent in agents {
                        pane_count += count_tabs(&agent["tabs"]);
                    }
                } else if let Some(tabs) = data.get("tabs") {
                    pane_count = count_tabs(tabs);
                } else {
                    pane_count = data["panes"].as_array().map(Vec::len).unwrap_or(0);
                }
                result.push(json!({"name":display,"filename":name,"timestamp":timestamp_text,"paneCount":pane_count,"agentCount":agents.map(|a|a.iter().filter(|v|v["global"]!=true).count()).unwrap_or(0)}));
            }
        }
        let timestamp = if layouts { "createdAt" } else { "timestamp" };
        result.sort_by(|a, b| {
            b[timestamp]
                .as_str()
                .unwrap_or("")
                .cmp(a[timestamp].as_str().unwrap_or(""))
        });
        result
    }
}
fn quarantine(path: &Path, bytes: &[u8]) {
    let Some(parent) = path.parent() else {
        return;
    };
    let prefix = format!("{}.broken-", path.file_name().unwrap().to_string_lossy());
    if std::fs::read_dir(parent).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
    }) {
        return;
    }
    let name = parent.join(format!("{prefix}{}", now_iso().replace(':', "-")));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    if let Ok(mut file) = options.open(name) {
        let _ = file.write_all(bytes);
    }
}
pub(crate) fn install(mut options: Options, directory: PathBuf) -> Options {
    let service = Arc::new(Stores::new(directory));
    for method in [
        "layouts.list",
        "layouts.save",
        "layouts.delete",
        "sessions.list",
        "sessions.load",
        "sessions.save",
        "sessions.delete",
    ] {
        let service = service.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move { tokio::task::spawn_blocking(move || service.call(method, params)).await? }
        });
    }
    options
}
