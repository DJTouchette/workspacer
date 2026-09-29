use super::paths;
use crate::Options;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, BufReader};

pub(crate) fn install(options: Options) -> Options {
    options.handler(
        "search.project",
        |_, params| async move { search(params).await },
    )
}
pub struct Collector {
    cwd: PathBuf,
    limit: usize,
    total: usize,
    truncated: bool,
    indices: HashMap<PathBuf, usize>,
    files: Vec<Value>,
}
impl Collector {
    pub fn new(cwd: PathBuf, requested: i64) -> Self {
        Self {
            cwd,
            limit: if requested <= 0 {
                500
            } else {
                requested as usize
            },
            total: 0,
            truncated: false,
            indices: HashMap::new(),
            files: Vec::new(),
        }
    }
    pub fn add(&mut self, line: &str) -> bool {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        if message["type"] != "match" {
            return false;
        }
        let data = &message["data"];
        let Some(path) = data["path"]["text"].as_str().filter(|s| !s.is_empty()) else {
            return false;
        };
        let path = self.cwd.join(path.strip_prefix("./").unwrap_or(path));
        if !data["lines"].is_object() && !data["lines"].is_null() {
            return false;
        }
        let text = match data["lines"].get("text") {
            None | Some(Value::Null) => "",
            Some(Value::String(text)) => text,
            _ => return false,
        }
        .trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
        let clipped: String = text.chars().take(300).collect();
        let Some(line_number) = integer(&data["line_number"]) else {
            return false;
        };
        let mut columns = Vec::new();
        if !data["submatches"].is_null() {
            let Some(matches) = data["submatches"].as_array() else {
                return false;
            };
            for item in matches {
                if !item.is_object() && !item.is_null() {
                    return false;
                }
                let Some(column) = integer(&item["start"]).and_then(|start| start.checked_add(1))
                else {
                    return false;
                };
                columns.push(column);
            }
        }
        if columns.is_empty() {
            columns.push(1);
        }
        for column in columns {
            if self.total >= self.limit {
                self.truncated = true;
                return true;
            }
            let index = *self.indices.entry(path.clone()).or_insert_with(|| {
                let index = self.files.len();
                self.files.push(json!({"file":path,"matches":[]}));
                index
            });
            self.files[index]["matches"]
                .as_array_mut()
                .unwrap()
                .push(json!({"line":line_number,"column":column,"text":clipped}));
            self.total += 1;
        }
        false
    }
    pub fn result(self) -> Value {
        json!({"results":self.files,"truncated":self.truncated})
    }
}
fn integer(value: &Value) -> Option<i64> {
    if value.is_null() {
        Some(0)
    } else {
        value.as_i64()
    }
}
pub async fn search(params: Value) -> Result<Value> {
    if !params.is_object() && !params.is_null() {
        bail!("search parameters must be an object");
    }
    for key in ["cwd", "query"] {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            bail!("{key} must be text");
        }
    }
    for key in ["caseSensitive", "wholeWord", "regex"] {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            bail!("{key} must be a boolean");
        }
    }
    if params
        .get("maxResults")
        .is_some_and(|value| !value.is_null() && value.as_i64().is_none())
    {
        bail!("maxResults must be an integer");
    }
    let query = params["query"].as_str().unwrap_or("");
    let cwd = paths::canonicalize(Path::new(params["cwd"].as_str().unwrap_or("")))?;
    if query.is_empty() {
        return Ok(json!({"results":[],"truncated":false}));
    }
    let mut command = tokio::process::Command::new("rg");
    command
        .current_dir(&cwd)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.args(["--json", "--line-number", "--column"]);
    command.arg(if params["caseSensitive"] == true {
        "-s"
    } else {
        "--smart-case"
    });
    if params["wholeWord"] == true {
        command.arg("-w");
    }
    if params["regex"] != true {
        command.arg("-F");
    }
    command.args(["--", query, "."]);
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("ripgrep not runnable (is `rg` on PATH?): {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let mut collector = Collector::new(cwd, params["maxResults"].as_i64().unwrap_or(0));
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let mut truncated = false;
        while let Some(line) = lines.next_line().await? {
            if collector.add(&line) {
                truncated = true;
                break;
            }
        }
        if truncated {
            child.kill().await?;
        }
        let status = child.wait().await?;
        if !truncated && !status.success() && status.code() != Some(1) {
            bail!("ripgrep failed (exit {})", status.code().unwrap_or(-1));
        }
        Ok::<_, anyhow::Error>(collector.result())
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("ripgrep timed out after 15s");
        }
    }
}
