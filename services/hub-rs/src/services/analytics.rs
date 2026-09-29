//! Persistent headless history, separate from claudemon's owned state database.
//! Daemon reads use its embedded API; failed reads never become an empty fleet.
mod fold;
mod store;
use super::pricing::Pricing;
use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn n(values: &[&Value]) -> f64 {
    values
        .iter()
        .find_map(|v| v.as_f64().filter(|n| n.is_finite() && *n >= 0.))
        .unwrap_or(0.)
}
fn iso(value: &Value) -> String {
    let date = if let Some(s) = value.as_str() {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc))
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|d| d.and_utc())
            })
    } else {
        value
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
    };
    date.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
pub struct Analytics {
    db: Mutex<Connection>,
    pricing: Arc<Pricing>,
}
impl Analytics {
    pub fn open(path: PathBuf, pricing: Arc<Pricing>) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(include_str!("analytics/schema.sql"))?;
        if !store::query(&tx, "PRAGMA table_info(session_history)", vec![])?
            .iter()
            .any(|r| r["name"] == "provider")
        {
            tx.execute_batch("ALTER TABLE session_history ADD COLUMN provider TEXT DEFAULT '';")?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_session_history_provider ON session_history(provider);",
        )?;
        tx.commit()?;
        Ok(Self {
            db: Mutex::new(db),
            pricing,
        })
    }
    pub fn read(&self, method: &str, params: &Value) -> Result<Value> {
        store::read(
            &self.db.lock().unwrap(),
            method,
            params,
            chrono::Utc::now().timestamp_millis(),
        )
    }
    pub fn observe_and_read(
        &self,
        method: &str,
        params: &Value,
        snapshots: &[Value],
        roots: &[PathBuf],
        now_ms: i64,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let transcripts = index(roots);
        let rates = self.pricing.overrides();
        for row in snapshots {
            let id = row["sessionId"]
                .as_str()
                .or_else(|| row["session_id"].as_str())
                .unwrap_or("");
            if id.is_empty()
                || id.len() > 128
                || id.starts_with("agent-")
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                || !text(&row["hub"]).is_empty()
            {
                continue;
            }
            let prior = store::query(
                &db,
                "SELECT * FROM session_history WHERE session_id=?",
                vec![id.to_string().into()],
            )?
            .pop()
            .unwrap_or(Value::Null);
            let provider = row["provider"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("claude");
            let sl = row
                .get("statusLine")
                .or_else(|| row.get("status_line"))
                .unwrap_or(&Value::Null);
            let usage = &row["usage"];
            let model = [
                &usage["model"],
                &sl["modelDisplay"],
                &sl["model_display"],
                &sl["model"],
                &prior["model"],
            ]
            .into_iter()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
            .unwrap_or("");
            let stopped = row["status"] == "ended" || row["mode"] == "stopped";
            let started = iso(row
                .get("startedAt")
                .or_else(|| row.get("started_at"))
                .unwrap_or(&Value::Null));
            let start = if started.is_empty() {
                text(&prior["started_at"]).to_string()
            } else {
                started
            };
            let updated = iso(row
                .get("updated_at")
                .or_else(|| row.get("lastActivity"))
                .unwrap_or(&Value::Null));
            let end = if stopped {
                if updated.is_empty() {
                    text(&prior["ended_at"]).to_string()
                } else {
                    updated
                }
            } else {
                String::new()
            };
            let workflows = row["workflows"].as_array().cloned().unwrap_or_default();
            let subagents = row["subagents"].as_array().map_or(0, Vec::len)
                + workflows
                    .iter()
                    .map(|w| w["agents"].as_array().map_or(0, Vec::len))
                    .sum::<usize>();
            let cwd = text(&row["cwd"]);
            let label = row["label"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| Path::new(cwd).file_name().and_then(|s| s.to_str()))
                .unwrap_or(&id[..id.len().min(8)]);
            let duration = chrono::DateTime::parse_from_rfc3339(&start)
                .ok()
                .map(|s| {
                    let finish = if stopped {
                        chrono::DateTime::parse_from_rfc3339(&end)
                            .map(|d| d.timestamp_millis())
                            .unwrap_or(s.timestamp_millis())
                    } else {
                        now_ms
                    };
                    (finish - s.timestamp_millis()).max(0)
                })
                .unwrap_or(0);
            let mut record = json!({"sessionId":id,"cwd":cwd,"agentName":label,"provider":provider,"model":model,"gitBranch":sl.get("gitBranch").or_else(||sl.get("git_branch")).unwrap_or(&prior["git_branch"]),"startedAt":start,"endedAt":end,"durationMs":duration,
    "inputTokens":n(&[&usage["totalInputTokens"],&sl["totalInputTokens"],&sl["total_input_tokens"],&prior["input_tokens"]]),"outputTokens":n(&[&usage["totalOutputTokens"],&sl["totalOutputTokens"],&sl["total_output_tokens"],&prior["output_tokens"]]),
    "costUSD":if provider=="claude"{n(&[&prior["cost_usd"],&usage["costUSD"],&usage["cost_usd"],&sl["costUSD"],&sl["cost_usd"]])}else{n(&[&sl["costUSD"],&sl["cost_usd"],&prior["cost_usd"]])},
    "peakContext":n(&[&row["peakContext"],&usage["contextTokens"],&usage["context_tokens"]]).max(n(&[&prior["peak_context"]])),"toolCalls":n(&[&row["totalToolCalls"],&row["tool_calls"],&prior["tool_calls"]]),"messageCount":n(&[&row["messageCount"],&prior["message_count"]]),"subagentCount":subagents,"workflowRuns":if workflows.is_empty(){n(&[&prior["workflow_runs"]])}else{workflows.len() as f64},"workflowFailed":workflows.iter().filter(|w|w["status"]=="failed").count(),"status":if stopped{"ended"}else{"active"},"updatedAt":iso(&now_ms.into())});
            let mut models = None;
            let mut fingerprint = None;
            if provider == "claude" {
                let candidate = row["transcriptPath"]
                    .as_str()
                    .or_else(|| row["transcript_path"].as_str())
                    .map(PathBuf::from)
                    .or_else(|| transcripts.get(id).cloned());
                if let Some(candidate) = candidate {
                    let files = fold::files(&candidate, roots)?;
                    let folded = (|| -> Result<()> {
                        let stamp = fold::fingerprint(&files)?;
                        let previous = store::query(
                            &db,
                            "SELECT fingerprint FROM analytics_sources WHERE session_id=?",
                            vec![id.to_string().into()],
                        )?
                        .pop()
                        .unwrap_or(Value::Null);
                        if previous["fingerprint"] != stamp {
                            if let Some(fold) = fold::recompute(&files, &rates)? {
                                record["model"] = fold.model.into();
                                record["inputTokens"] = fold.input.into();
                                record["outputTokens"] = fold.output.into();
                                record["costUSD"] = fold.cost.into();
                                record["peakContext"] = fold.peak.into();
                                models = Some(fold.models);
                            }
                        } else if prior.is_object() {
                            for (to, from) in [
                                ("inputTokens", "input_tokens"),
                                ("outputTokens", "output_tokens"),
                                ("costUSD", "cost_usd"),
                                ("model", "model"),
                                ("peakContext", "peak_context"),
                            ] {
                                record[to] = prior[from].clone();
                            }
                        }
                        fingerprint = Some(stamp);
                        Ok(())
                    })();
                    if let Err(error) = folded {
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                        {
                            if prior.is_object() {
                                for (to, from) in [
                                    ("inputTokens", "input_tokens"),
                                    ("outputTokens", "output_tokens"),
                                    ("costUSD", "cost_usd"),
                                ] {
                                    record[to] = prior[from].clone();
                                }
                            }
                        } else {
                            return Err(error);
                        }
                    }
                }
            } else if !model.is_empty() {
                models = Some(serde_json::Map::from_iter([(
                    model.to_string(),
                    json!({"inputTokens":record["inputTokens"],"outputTokens":record["outputTokens"],"costUSD":record["costUSD"]}),
                )]));
            }
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            store::record(&tx, &record)?;
            if let Some(models) = models {
                tx.execute("DELETE FROM session_model_usage WHERE session_id=?", [id])?;
                for (model, slice) in models {
                    tx.execute("INSERT INTO session_model_usage(session_id,model,input_tokens,output_tokens,cost_usd,updated_at) VALUES(?,?,?,?,?,?)",rusqlite::params![id,model,n(&[&slice["inputTokens"]]),n(&[&slice["outputTokens"]]),n(&[&slice["costUSD"]]),text(&record["updatedAt"])])?;
                }
            }
            if let Some(stamp) = fingerprint {
                tx.execute(
                    "INSERT OR REPLACE INTO analytics_sources VALUES(?,?)",
                    rusqlite::params![id, stamp],
                )?;
            }
            tx.commit()?;
        }
        store::read(&db, method, params, now_ms)
    }
}
fn index(roots: &[PathBuf]) -> std::collections::BTreeMap<String, PathBuf> {
    let mut out = std::collections::BTreeMap::new();
    for root in roots {
        if let Ok(projects) = std::fs::read_dir(root) {
            for project in projects.flatten() {
                if !project.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(project.path()) {
                    for file in files.flatten() {
                        let p = file.path();
                        if p.extension().is_some_and(|e| e == "jsonl") {
                            if let Some(id) = p.file_stem().and_then(|s| s.to_str()) {
                                out.insert(id.into(), p);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

pub mod watcher;
pub use watcher::{Watcher, install};
