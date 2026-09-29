use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Telemetry {
    runs: BTreeMap<(String, String), String>,
    agents: BTreeMap<(String, String, String), String>,
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
impl Telemetry {
    pub fn forget(&mut self, session: &str) {
        self.runs.retain(|(s, _), _| s != session);
        self.agents.retain(|(s, _, _), _| s != session);
    }
    pub fn update(&mut self, session: &str, cwd: &str, runs: &Value) -> Vec<Value> {
        let mut events = Vec::new();
        for run in runs.as_array().into_iter().flatten() {
            let id = text(run, "runId");
            let status = text(run, "status");
            let key = (session.into(), id.into());
            let previous = self.runs.insert(key, status.into());
            if previous.as_deref() != Some(status) {
                let topic = if status == "running" && previous.is_none() {
                    Some("workflow.started")
                } else if status == "completed" {
                    Some("workflow.completed")
                } else if status == "failed" {
                    Some("workflow.failed")
                } else {
                    None
                };
                if let Some(topic) = topic {
                    let mut data = json!({"sessionId":session,"cwd":cwd,"runId":id,"agents":run["agents"].as_array().map(Vec::len).unwrap_or(0)});
                    copy(run, &mut data, &["name"]);
                    if topic == "workflow.started" {
                        copy(run, &mut data, &["description", "startedAt"]);
                        data["phases"] = json!(run["phases"].as_array().map(Vec::len).unwrap_or(0));
                    } else {
                        copy(
                            run,
                            &mut data,
                            &["status", "durationMs", "totalTokens", "totalToolCalls"],
                        );
                    }
                    events.push(json!({"type":topic,"data":data}));
                }
            }
            for agent in run["agents"].as_array().into_iter().flatten() {
                let status = text(agent, "status");
                if !matches!(status, "done" | "failed") {
                    continue;
                }
                let key = (session.into(), id.into(), text(agent, "id").into());
                if self.agents.insert(key, status.into()).as_deref() == Some(status) {
                    continue;
                }
                let mut data =
                    json!({"sessionId":session,"cwd":cwd,"runId":id,"agentId":agent["id"]});
                copy(
                    agent,
                    &mut data,
                    &[
                        "label",
                        "model",
                        "status",
                        "durationMs",
                        "tokens",
                        "toolCalls",
                        "phaseTitle",
                    ],
                );
                events.push(json!({"type":"workflow.agent.finished","data":data}));
            }
        }
        events
    }
}
