//! Live switches and handoff/history adapters over the owned daemon.
use super::agent_lifecycle::Lifecycle;
use crate::{Handle, Options, protocol::Event};
use anyhow::{Result, bail};
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn segment(id: &str) -> Result<()> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "invalid sessionId"
    );
    Ok(())
}
fn failed(error: impl std::fmt::Display) -> Value {
    json!({"ok":false,"error":error.to_string()})
}
fn acknowledged(mut receipt: Value, bookkeeping: Result<()>) -> Value {
    if let Err(error) = bookkeeping {
        eprintln!("live control was accepted but local bookkeeping failed: {error}");
        receipt["warning"]="The setting change was accepted, but its local metadata or notification update was not confirmed. Do not repeat the change solely for this warning.".into();
    }
    receipt
}
struct Controls {
    engine: EmbeddedClient,
    lifecycle: Option<Arc<Lifecycle>>,
    rows: Arc<RwLock<BTreeMap<String, Value>>>,
    hub: Handle,
    home: PathBuf,
    config: PathBuf,
}
impl Controls {
    async fn request(&self, method: &str, path: String, payload: Option<Value>) -> Result<Value> {
        self.engine
            .request(Command::Request {
                method: method.into(),
                path,
                payload,
            })
            .await
    }
    fn generation(&self, id: &str) -> String {
        self.lifecycle
            .as_ref()
            .and_then(|l| l.records().get(id).map(|r| r.generation.clone()))
            .unwrap_or_default()
    }
    async fn note(&self, id: &str, generation: &str, patch: Value) -> Result<()> {
        if let Some(lifecycle) = &self.lifecycle {
            if !generation.is_empty() {
                if !lifecycle.note_live_control(id, generation, &patch).await? {
                    return Ok(());
                }
            } else if lifecycle.records().contains_key(id) {
                return Ok(());
            }
        }
        let snapshot = {
            let mut rows = self.rows.write().unwrap();
            let Some(row) = rows.get_mut(id) else {
                return Ok(());
            };
            if row["status"] == "ended" || row["hub"].as_str().is_some_and(|s| !s.is_empty()) {
                return Ok(());
            }
            if let Some(settings) = patch["settings"].as_object() {
                if !row["settings"].is_object() {
                    row["settings"] = json!({});
                }
                for (key, value) in settings {
                    row["settings"][key] = value.clone();
                }
            }
            for key in ["livePermissionMode", "requestedSelection"] {
                if let Some(value) = patch.get(key) {
                    row[key] = value.clone();
                }
            }
            row.clone()
        };
        self.hub
            .publish(Event::new("agent.snapshot", "brain", snapshot))?;
        Ok(())
    }
    async fn provider(&self, id: &str) -> String {
        if let Some(provider) = self
            .rows
            .read()
            .unwrap()
            .get(id)
            .and_then(|r| r["provider"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        {
            return provider;
        }
        self.request("GET", format!("/sessions/{id}"), None)
            .await
            .ok()
            .and_then(|r| {
                r["provider"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "claude".into())
    }
    async fn call(&self, method: &str, p: Value) -> Result<Value> {
        if method == "sessions.recent" {
            let raw = self
                .request("GET", "/sessions?include_archived=true".into(), None)
                .await
                .unwrap_or(json!([]));
            let names = if self.config.is_absolute() {
                super::profile_accounts::read(&self.config.join("tui-names.json"))
            } else {
                Value::Null
            };
            let records = self
                .lifecycle
                .as_ref()
                .map(|l| l.records())
                .unwrap_or_default();
            return Ok(recent(&raw, |id, cwd| {
                records
                    .get(id)
                    .and_then(|r| r.metadata["label"].as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| names[cwd].as_str().unwrap_or(""))
                    .to_owned()
            }));
        }
        let id = text(&p, "sessionId");
        segment(id)?;
        let generation = self.generation(id);
        match method {
            "claude.setPermissionMode" => {
                let mode = text(&p, "mode");
                anyhow::ensure!(
                    !mode.is_empty(),
                    "claude.setPermissionMode requires {{ sessionId, mode }}"
                );
                match self
                    .request(
                        "POST",
                        format!("/sessions/{id}/permission-mode"),
                        Some(json!({"mode":mode})),
                    )
                    .await
                {
                    Err(error) => Ok(failed(error)),
                    Ok(reply) => {
                        let mode = reply["mode"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(mode);
                        let bookkeeping=self.note(id,&generation,json!({"settings":{"permissionMode":mode},"livePermissionMode":mode})).await;
                        Ok(acknowledged(json!({"ok":true,"mode":mode}), bookkeeping))
                    }
                }
            }
            "claude.setEffort" => {
                let effort = text(&p, "effort").trim();
                if effort.is_empty() {
                    return Ok(failed("requires a session and an effort level"));
                }
                let response = if self.provider(id).await == "claude" {
                    self.engine
                        .request(Command::Message {
                            id: id.into(),
                            text: format!("/effort {effort}"),
                        })
                        .await
                } else {
                    self.request(
                        "POST",
                        format!("/sessions/{id}/model"),
                        Some(json!({"effort":effort})),
                    )
                    .await
                };
                match response {
                    Err(error) => Ok(failed(error)),
                    Ok(value) if value["ok"] == false => {
                        Ok(failed("this session can't take input right now (ended)"))
                    }
                    Ok(_) => {
                        let bookkeeping = self
                            .note(id, &generation, json!({"settings":{"effort":effort}}))
                            .await;
                        Ok(acknowledged(
                            json!({"ok":true,"effort":effort}),
                            bookkeeping,
                        ))
                    }
                }
            }
            "claude.setModel" => {
                let provider = self.provider(id).await;
                let payload = match model_payload(&provider, &p) {
                    Ok(payload) => payload,
                    Err(error) => return Ok(failed(error)),
                };
                match self
                    .request(
                        "POST",
                        format!("/sessions/{id}/model"),
                        Some(payload.clone()),
                    )
                    .await
                {
                    Err(error) => Ok(failed(error)),
                    Ok(reply) => {
                        let model = reply["model"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| text(&payload, "model"));
                        let selection=reply.get("requested_selection").filter(|v|v.is_object()).map(|v|json!({"model":v["model"],"contextWindow":v["context_window"]})).or_else(||payload["model_identity"].as_str().map(|model|json!({"model":model,"contextWindow":payload["context_window"]})));
                        let mut result = json!({"ok":true});
                        if !model.is_empty() {
                            result["model"] = model.into();
                        }
                        if let Some(selection) = selection.clone() {
                            result["requestedSelection"] = selection;
                        }
                        if reply["queued"] == true {
                            result["queued"] = true.into();
                        } else {
                            let mut patch = json!({"settings":{}});
                            if !model.is_empty() {
                                patch["settings"]["model"] = model.into();
                            }
                            if !text(&p, "effort").trim().is_empty() {
                                patch["settings"]["effort"] = text(&p, "effort").trim().into();
                            }
                            if let Some(selection) = selection {
                                patch["requestedSelection"] = selection;
                            }
                            result = acknowledged(result, self.note(id, &generation, patch).await);
                        }
                        if !text(&reply, "disposition").is_empty() {
                            result["disposition"] = reply["disposition"].clone();
                        }
                        Ok(result)
                    }
                }
            }
            "claude.handoffBrief" => Ok(
                match self
                    .request("POST", format!("/sessions/{id}/handoff"), Some(json!({})))
                    .await
                {
                    Ok(reply) => {
                        json!({"ok":true,"markdown":reply["markdown"],"path":reply["path"]})
                    }
                    Err(error) => failed(error),
                },
            ),
            "claude.handoffAgentBrief" => self.agent_brief(id).await,
            _ => bail!("unknown live control"),
        }
    }
    async fn agent_brief(&self, id: &str) -> Result<Value> {
        anyhow::ensure!(self.home.is_absolute(), "home directory unavailable");
        let directory = self.home.join(".workspacer/handoffs");
        tokio::fs::create_dir_all(&directory).await?;
        let target = directory.join(format!(
            "{}-{}-agent.md",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            &uuid::Uuid::new_v4().to_string()[..8]
        ));
        let instruction = format!(
            "Stop what you're doing and write a handoff brief to {} — another AI coding agent is about to take over this session and will read that file first. Create the file (markdown) with:\n1. The goal of this session, in one paragraph.\n2. State of the work: what's done and verified, what's in progress, what hasn't been started.\n3. Key files touched and why.\n4. Decisions and constraints your successor must respect (including approaches tried and rejected, and why).\n5. Gotchas or surprises you hit.\n6. The exact next step you would take.\nWrite only that file, then reply \"Handoff brief written.\" — do not continue any other work.",
            target.display()
        );
        let accepted = self
            .engine
            .request(Command::Message {
                id: id.into(),
                text: instruction,
            })
            .await
            .is_ok_and(|v| v["ok"] != false);
        let reason = if accepted {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if tokio::fs::symlink_metadata(&target)
                    .await
                    .is_ok_and(|m| m.is_file() && m.len() > 0)
                {
                    return Ok(json!({"ok":true,"path":target}));
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            "Source agent did not write the brief before the deadline"
        } else {
            "Source agent could not accept the brief request"
        };
        Ok(
            match self
                .request("POST", format!("/sessions/{id}/handoff"), Some(json!({})))
                .await
            {
                Ok(reply) => {
                    json!({"ok":!text(&reply,"path").is_empty(),"path":reply["path"],"fallback":true,"error":reason})
                }
                Err(error) => failed(format!(
                    "{reason}; mechanical fallback also failed: {error}"
                )),
            },
        )
    }
}
fn model_payload(provider: &str, p: &Value) -> Result<Value> {
    let model = text(p, "model");
    let identity = text(p, "modelIdentity");
    let effort = text(p, "effort").trim();
    if model.trim().is_empty()
        && identity.trim().is_empty()
        && p["contextWindow"].is_null()
        && effort.is_empty()
    {
        bail!("empty-model")
    }
    let window = if p["contextWindow"].is_null() {
        None
    } else {
        Some(
            p["contextWindow"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or_else(|| anyhow::anyhow!("invalid-context-window"))?,
        )
    };
    let selection = crate::model_selection::normalize_model_input(
        provider,
        (!model.is_empty()).then_some(model),
        (!identity.is_empty()).then_some(identity),
        window,
    )
    .map_err(|e| anyhow::anyhow!(e.code()))?;
    let mut result = json!({});
    if !effort.is_empty() {
        result["effort"] = effort.into();
    }
    if let Some(selection) = selection {
        result["model"] = selection.legacy_model.into();
        result["model_identity"] = selection.selection.model.into();
        if let Some(window) = selection.selection.context_window {
            result["context_window"] = window.into();
        }
    } else if let Some(window) = window {
        result["context_window"] = window.into();
    }
    Ok(result)
}
fn recent(raw: &Value, name: impl Fn(&str, &str) -> String) -> Value {
    let millis = |value: &Value| {
        value
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis())
            .unwrap_or(0)
    };
    let mut rows=raw.as_array().into_iter().flatten().filter(|r|!text(r,"session_id").is_empty()&&!text(r,"session_id").starts_with("agent-")).map(|r|{
        let default=|key:&str,fallback:&str|r[key].as_str().filter(|s|!s.is_empty()).unwrap_or(fallback).to_owned();
        json!({"sessionId":r["session_id"],"provider":default("provider","claude"),"cwd":text(r,"cwd"),"mode":default("mode","unknown"),"transport":default("transport","pty"),"archived":r["archived"]==true,"updatedAt":millis(&r["updated_at"]),"startedAt":millis(&r["started_at"]),"name":name(text(r,"session_id"),text(r,"cwd")),"title":"","model":""})
    }).collect::<Vec<_>>();
    rows.sort_by_key(|r| std::cmp::Reverse(r["updatedAt"].as_i64().unwrap_or(0)));
    json!(rows)
}
pub(crate) fn install(mut options: Options, hub: Handle) -> Options {
    let Some(engine) = options.engine.clone() else {
        return options;
    };
    let service = Arc::new(Controls {
        engine,
        lifecycle: options.launch_lifecycle.clone(),
        rows: options.session_snapshots.clone(),
        hub,
        home: options.home_dir.clone().unwrap_or_default(),
        config: options.config_dir.clone().unwrap_or_default(),
    });
    for method in [
        "claude.setModel",
        "claude.setEffort",
        "claude.setPermissionMode",
        "claude.handoffBrief",
        "claude.handoffAgentBrief",
        "sessions.recent",
    ] {
        let service = service.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move { service.call(method, params).await }
        });
    }
    options
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_live_selection_does_not_default_codex_window_or_send_empty_model() {
        assert_eq!(
            model_payload("codex", &json!({"effort":" high "})).unwrap(),
            json!({"effort":"high"})
        );
        assert_eq!(
            model_payload("claude", &json!({"model":"opus[1m]"})).unwrap(),
            json!({"model":"opus[1m]","model_identity":"opus","context_window":1_000_000})
        );
        assert!(model_payload("claude", &json!({"model":" "})).is_err());
        assert!(
            model_payload("claude", &json!({"model":"opus","modelIdentity":"sonnet"})).is_err()
        );
    }
    #[test]
    fn recents_preserve_archived_history_and_unknown_cost() {
        let rows = recent(
            &json!([{"session_id":"old","archived":true},{"session_id":"new","updated_at":"2026-09-28T12:00:00Z","cwd":"/repo"},{"session_id":"agent-pending"}]),
            |id, _| id.into(),
        );
        assert_eq!(rows.as_array().unwrap().len(), 2);
        assert_eq!(rows[0]["sessionId"], "new");
        assert_eq!(rows[0]["provider"], "claude");
        assert_eq!(rows[1]["archived"], true);
        assert!(rows[0].get("costUSD").is_none());
        assert_eq!(rows[0]["title"], "");
    }
}
