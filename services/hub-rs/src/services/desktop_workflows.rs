//! Owner-only adapters for persisted workflows, captured requests and Claude artifacts.
use super::{dispatch_templates::trim_js, files, paths, profiles::Profiles};
use crate::Options;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn identity(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn result_text(block: &Value) -> String {
    block["content"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            block["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b["text"].as_str())
                .collect()
        })
}
pub fn parse_artifact(raw: &str, rich: bool, now: i64) -> Value {
    let mut turns: Vec<Value> = Vec::new();
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    for line in raw.lines() {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let msg = row.get("message").filter(|v| !v.is_null()).unwrap_or(&row);
        let role = msg["role"]
            .as_str()
            .or_else(|| row["type"].as_str())
            .unwrap_or("");
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let content = &msg["content"];
        let ts = row["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis())
            .filter(|ts| *ts != 0)
            .unwrap_or(now);
        if let Some(content) = content.as_str() {
            if !trim_js(content).is_empty() {
                turns.push(if rich {
                    json!({"role":role,"content":content,"timestamp":ts})
                } else {
                    json!({"role":role,"text":trim_js(content)})
                });
            }
            continue;
        }
        let mut parts = Vec::new();
        for block in content.as_array().into_iter().flatten() {
            match text(block, "type") {
                "text" => {
                    if let Some(s) = block["text"].as_str() {
                        if rich {
                            if !trim_js(s).is_empty() {
                                turns.push(json!({"role":role,"content":s,"timestamp":ts}));
                            }
                        } else {
                            parts.push(s.to_owned());
                        }
                    }
                }
                "tool_use" => {
                    let name = block
                        .get("name")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .unwrap_or(json!("tool"));
                    if rich {
                        let id = block
                            .get("id")
                            .filter(|v| !v.is_null())
                            .cloned()
                            .unwrap_or(json!(format!("tc-{ts}-{}", turns.len())));
                        if let Some(id) = id.as_str().filter(|s| !s.is_empty()) {
                            calls.insert(id.into(), turns.len());
                        }
                        turns.push(json!({"role":"assistant","content":"","timestamp":ts,"toolCalls":[{"id":id,"name":name,"input":block.get("input").filter(|v|!v.is_null()).cloned().unwrap_or(json!({})),"status":"running","startedAt":ts}]}));
                    } else {
                        parts.push(format!("⚙ {}", name.as_str().unwrap_or("tool")));
                    }
                }
                "tool_result" => {
                    let s = result_text(block);
                    if rich {
                        if let Some(index) = calls.get(text(block, "tool_use_id")) {
                            let call = &mut turns[*index]["toolCalls"][0];
                            call["response"] = s.into();
                            call["status"] = if block["is_error"] == true {
                                "failed"
                            } else {
                                "complete"
                            }
                            .into();
                            call["completedAt"] = ts.into();
                        }
                    } else if !trim_js(&s).is_empty() {
                        parts.push(format!("↳ {}", super::fleet_messages::clip(&s, 400, "")));
                    }
                }
                _ => (),
            }
        }
        if !rich {
            let joined = parts.join("\n");
            let s = trim_js(&joined);
            if !s.is_empty() {
                turns.push(json!({"role":role,"text":s}));
            }
        }
    }
    json!(turns)
}
pub struct Artifacts {
    home: PathBuf,
    profiles: Profiles,
    owner: super::task_store::OwnerLookup,
}
impl Artifacts {
    pub fn new(home: PathBuf, config: PathBuf, owner: super::task_store::OwnerLookup) -> Self {
        Self {
            home,
            profiles: Profiles::new(config),
            owner,
        }
    }
    fn roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            std::env::var_os("CLAUDE_CONFIG_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.home.join(".claude")),
        ];
        roots.extend(
            self.profiles
                .list()
                .into_iter()
                .filter(|p| p.provider.is_empty() || p.provider == "claude")
                .filter(|p| !p.config_dir.is_empty())
                .map(|p| {
                    if let Some(s) = p.config_dir.strip_prefix('~') {
                        PathBuf::from(format!("{}{}", self.home.display(), s))
                    } else {
                        p.config_dir.into()
                    }
                }),
        );
        roots
            .into_iter()
            .filter_map(|root| std::fs::canonicalize(root.join("projects")).ok())
            .collect()
    }
    pub(crate) fn locate(&self, id: &str, row: &Value) -> Option<PathBuf> {
        if !identity(id) || !text(row, "hub").is_empty() {
            return None;
        }
        let roots = self.roots();
        let mut candidates: Vec<PathBuf> = ["transcriptPath", "transcript_path"]
            .into_iter()
            .filter_map(|key| row[key].as_str())
            .filter(|s| s.ends_with(".jsonl"))
            .map(PathBuf::from)
            .collect();
        for root in &roots {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        candidates.push(entry.path().join(format!("{id}.jsonl")));
                    }
                }
            }
        }
        for candidate in candidates {
            let Ok(candidate) = std::fs::canonicalize(candidate) else {
                continue;
            };
            if !roots.iter().any(|root| paths::contained(&candidate, root)) || !candidate.is_file()
            {
                continue;
            }
            return Some(candidate);
        }
        None
    }
    pub fn read(&self, params: &Value, rich: bool) -> Result<Value> {
        let id = text(params, "sessionId");
        let run = text(params, "runId");
        let agent = text(params, "agentId");
        if ![id, run, agent].into_iter().all(identity) {
            bail!("Invalid workflow identity");
        }
        let Some(row) = (self.owner)(id).filter(|r| text(r, "hub").is_empty()) else {
            return Ok(Value::Null);
        };
        if let Some(candidate) = self.locate(id, &row) {
            let Some(stem) = candidate.to_str().and_then(|s| s.strip_suffix(".jsonl")) else {
                return Ok(Value::Null);
            };
            // The watcher registers directory names verbatim (wf_<id>), including their prefix.
            if !run.starts_with("wf_") {
                return Ok(Value::Null);
            }
            let root = Path::new(stem);
            let target = root.join("subagents/workflows").join(run).join(format!(
                "agent-{}.jsonl",
                agent.strip_prefix("agent-").unwrap_or(agent)
            ));
            let Ok(target) = std::fs::canonicalize(target) else {
                return Ok(Value::Null);
            };
            let Ok(root) = std::fs::canonicalize(root) else {
                return Ok(Value::Null);
            };
            if !paths::contained(&target, &root) {
                return Ok(Value::Null);
            }
            let Ok(bytes) = files::bounded_bytes(&target, 64 * 1024 * 1024) else {
                return Ok(Value::Null);
            };
            return Ok(parse_artifact(
                &String::from_utf8_lossy(&bytes),
                rich,
                chrono::Utc::now().timestamp_millis(),
            ));
        }
        Ok(Value::Null)
    }
}
pub(crate) fn install(mut options: Options) -> Options {
    if let Some(runtime) = options.workflow_runtime.clone() {
        options = options.handler("desktop.fleetWorkflowRequest", move |caller, params| {
            let runtime = runtime.clone();
            async move {
                if !caller.authenticated_host {
                    bail!("desktop services require authenticated owner authority");
                }
                tokio::task::spawn_blocking(move || Ok(runtime.request(&params["request"], "")))
                    .await?
            }
        });
        if let (Some(engine), Some(tracker)) =
            (options.engine.clone(), options.message_tracker.clone())
        {
            let requests = options.workflow_runtime.as_ref().unwrap().requests.clone();
            options=options.handler("desktop.managerRequestSend",move|caller,params|{let engine=engine.clone();let tracker=tracker.clone();let requests=requests.clone();async move{
                if !caller.authenticated_host{bail!("desktop services require authenticated owner authority");}
                let session=text(&params,"sessionId").to_owned();let request=text(&params,"requestId").to_owned();if session.is_empty()||request.is_empty(){bail!("captured message requires sessionId and requestId");}
                let store=requests.clone();let sid=session.clone();let rid=request.clone();let delivery=tokio::task::spawn_blocking(move||store.begin_delivery(&sid,&rid)).await??;
                if let Some(delivery)=delivery {
                    let source=json!({"requestId":request,"deliveryId":delivery["deliveryId"]});
                    let sent=super::manager_replacements::messages::send_engine(&engine,&tracker,&session,text(&delivery,"text"),&[],Some(source),false,Some(&requests)).await;
                    let status=match &sent{Ok(sent) if sent.held=>"pending",Ok(sent)=>match sent.outcome{super::manager_replacements::SendOutcome::Accepted=>"accepted",super::manager_replacements::SendOutcome::Rejected{..}=>"rejected",_=>"unknown"},Err(_)=>"unknown"};
                    requests.finish_delivery(&request,text(&delivery,"deliveryId"),status)?;
                }
                let receipt=requests.host_request(&session,&request)?;Ok(json!({"ok":matches!(text(&receipt,"delivery"),"accepted"|"pending"),"requestId":receipt["requestId"],"delivery":receipt["delivery"],"mode":receipt["delivery"]}))
            }});
        }
    }
    if let (Some(home), Some(config)) = (options.home_dir.clone(), options.config_dir.clone()) {
        let service = Arc::new(Artifacts::new(home, config, super::local_lookup(&options)));
        for method in [
            "desktop.workflowAgentTranscript",
            "desktop.workflowAgentConversation",
        ] {
            let service = service.clone();
            options = options.handler(method, move |caller, params| {
                let service = service.clone();
                async move {
                    if !caller.authenticated_host {
                        bail!("desktop services require authenticated owner authority");
                    }
                    tokio::task::spawn_blocking(move || {
                        service.read(&params, method.ends_with("Conversation"))
                    })
                    .await?
                }
            });
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::super::{
        config::Config, task_store::TaskStore, workflow_runtime::WorkflowRuntime,
        workflows::WorkflowStore,
    };
    use super::*;
    use crate::{Hub, client::Client};
    #[test]
    fn artifact_parser_matches_shipping_typescript_reference() {
        let rows: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/workflow-artifacts.json"))
                .unwrap();
        for row in rows.as_array().unwrap() {
            let raw = row["raw"].as_str().unwrap();
            let now = row["now"].as_i64().unwrap();
            assert_eq!(parse_artifact(raw, false, now), row["transcript"]);
            assert_eq!(parse_artifact(raw, true, now), row["conversation"]);
        }
    }
    #[test]
    fn artifact_views_pair_tools_and_preserve_interrupted_calls() {
        let raw = concat!(
            "garbage\n",
            r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"text","text":"Look"},{"type":"tool_use","id":"x","name":"Read","input":{"file":"a"}},{"type":"tool_use","id":"unresolved","name":"Bash"}]}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_result","tool_use_id":"x","is_error":true,"content":[{"text":"first"},{"text":"second"}]},{"type":"text","text":"  next  "}]}}"#
        );
        assert_eq!(
            parse_artifact(raw, false, 7),
            json!([{"role":"assistant","text":"Look\n⚙ Read\n⚙ Bash"},{"role":"user","text":"↳ firstsecond\n  next"}])
        );
        let result = parse_artifact(raw, true, 7);
        assert_eq!(
            result[1]["toolCalls"][0],
            json!({"id":"x","name":"Read","input":{"file":"a"},"status":"failed","startedAt":1767225600000i64,"completedAt":1767225601000i64,"response":"firstsecond"})
        );
        assert_eq!(result[2]["toolCalls"][0]["status"], "running");
        assert_eq!(result[3]["content"], "  next  ");
        assert_eq!(
            parse_artifact("{\"role\":\"assistant\",\"content\":\" hi \"}", true, 99)[0]["timestamp"],
            99
        );
    }
    #[test]
    fn artifact_lookup_discovers_profile_transcripts_and_confines_paths() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("account");
        let project = profile.join("projects/project");
        std::fs::create_dir_all(project.join("session/subagents/workflows/wf_run")).unwrap();
        std::fs::write(project.join("session.jsonl"), "").unwrap();
        let target = project.join("session/subagents/workflows/wf_run/agent-child.jsonl");
        std::fs::write(
            &target,
            "{\"type\":\"assistant\",\"message\":{\"content\":\"read from profile\"}}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("claude-profiles.json"),
            serde_json::to_vec(&json!({"profiles":[{"id":"custom","configDir":profile}]})).unwrap(),
        )
        .unwrap();
        let owner: super::super::task_store::OwnerLookup =
            Arc::new(|id| (id == "session").then(|| json!({"sessionId":"session"})));
        let reader = Artifacts::new(dir.path().into(), dir.path().into(), owner);
        let params = json!({"sessionId":"session","runId":"wf_run","agentId":"agent-child"});
        assert_eq!(
            reader.read(&params, false).unwrap(),
            json!([{"role":"assistant","text":"read from profile"}])
        );
        assert!(
            reader
                .read(
                    &json!({"sessionId":"session","runId":"../wf_run","agentId":"child"}),
                    true
                )
                .is_err()
        );
        assert!(
            reader
                .read(
                    &json!({"sessionId":"missing","runId":"wf_run","agentId":"child"}),
                    true
                )
                .unwrap()
                .is_null()
        );
        #[cfg(unix)]
        {
            std::fs::remove_file(&target).unwrap();
            let secret = dir.path().join("outside.jsonl");
            std::fs::write(
                &secret,
                "{\"type\":\"assistant\",\"message\":{\"content\":\"secret\"}}",
            )
            .unwrap();
            std::os::unix::fs::symlink(secret, &target).unwrap();
            assert!(reader.read(&params, true).unwrap().is_null());
        }
    }
    #[tokio::test]
    async fn owner_workflow_wrapper_preserves_no_implicit_manager_authority() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(Config::open(dir.path().join("config.yaml")));
        let definitions = Arc::new(WorkflowStore::new(dir.path().into(), config));
        let tasks = Arc::new(TaskStore::open(dir.path().join("tasks.json")).unwrap());
        let mut options = Options::default();
        options.workflow_runtime = Some(Arc::new(WorkflowRuntime::new(
            definitions,
            tasks,
            Arc::new(|id| {
                (id == "manager").then(||json!({"sessionId":"manager","isWakeTarget":true,"cwd":"/tmp","status":"active"}))
            }),
        )));
        let hub = Hub::start(install(options)).unwrap();
        hub.ready().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        let listed = client
            .call(
                "desktop.fleetWorkflowRequest",
                json!({"request":{"op":"list"}}),
            )
            .await
            .unwrap();
        assert_eq!(listed["ok"], true);
        let result=client.call("desktop.fleetWorkflowRequest",json!({"request":{"op":"start","title":"claimed","cwd":"/tmp","callerSessionId":"manager"}})).await.unwrap();
        assert_eq!(result["ok"], false);
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn captured_send_uses_engine_receipts_and_never_replays_unknown() {
        let _engine_guard = crate::backend::ENGINE_TEST_LOCK.lock().await;
        use super::super::manager_replacements::{MessageTracker, ReplacementState};
        use claudemon::daemon::{
            ServeConfig,
            embedded::{EmbeddedDaemon, Options as EngineOptions},
        };
        let dir = tempfile::tempdir().unwrap();
        let mut engine = EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: dir.path().join("daemon.db"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
        )
        .unwrap();
        engine.ready().await.unwrap();
        let config = Arc::new(Config::open(dir.path().join("config.yaml")));
        let definitions = Arc::new(WorkflowStore::new(dir.path().into(), config));
        let tasks = Arc::new(TaskStore::open(dir.path().join("tasks.json")).unwrap());
        let cwd = dir.path().to_owned();
        let runtime = Arc::new(WorkflowRuntime::new(
            definitions,
            tasks,
            Arc::new(move |id| {
                (id=="manager").then(||json!({"sessionId":"manager","isWakeTarget":true,"cwd":cwd,"status":"active"}))
            }),
        ));
        let requests = runtime.requests.clone();
        let prepared = requests.prepare("manager", "Inspect safely", true).unwrap();
        let id = prepared["requestId"].as_str().unwrap();
        let state = ReplacementState::open(dir.path().join("replacements.json")).unwrap();
        let mut options = Options::default();
        options.engine = Some(engine.client());
        options.workflow_runtime = Some(runtime);
        options.message_tracker = Some(MessageTracker::new(state));
        let token = crate::auth::mint(
            &dir.path().join("tokens.json"),
            crate::auth::Scope::Operator,
            "ordinary operator",
        )
        .unwrap();
        options.token = "owner".into();
        options.scoped_tokens = Some(dir.path().join("tokens.json"));
        let hub = Hub::start(install(options)).unwrap();
        hub.ready().await.unwrap();
        let host = Client::connect(&hub.handle()).await.unwrap();
        let scoped = Client::from_connection(
            hub.handle()
                .connect_authenticated(token.token, false)
                .await
                .unwrap(),
        );
        let params = json!({"sessionId":"manager","requestId":id});
        assert!(
            scoped
                .call("desktop.managerRequestSend", params.clone())
                .await
                .is_err()
        );
        assert_eq!(
            requests.host_request("manager", id).unwrap()["attempts"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let response = host
            .call("desktop.managerRequestSend", params.clone())
            .await
            .unwrap();
        assert_eq!(response["delivery"], "rejected");
        assert_eq!(response["ok"], false);
        let retry = requests.begin_delivery("manager", id).unwrap().unwrap();
        requests
            .finish_delivery(id, text(&retry, "deliveryId"), "unknown")
            .unwrap();
        let response = host
            .call("desktop.managerRequestSend", params)
            .await
            .unwrap();
        assert_eq!(response["delivery"], "unknown");
        assert_eq!(
            requests.host_request("manager", id).unwrap()["attempts"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        hub.shutdown().unwrap();
        engine.shutdown().await.unwrap();
    }
}
