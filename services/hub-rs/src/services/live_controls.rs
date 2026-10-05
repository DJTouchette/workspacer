//! Live switches and handoff/history adapters over the owned daemon.
mod confirmation;
mod handoff;
use super::agent_lifecycle::Lifecycle;
use crate::{Handle, Options, protocol::Event};
use anyhow::{Result, bail};
use claudemon::daemon::embedded::{Command, EmbeddedClient};
pub(crate) use confirmation::ConfirmedControls;
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
    confirmed: Arc<ConfirmedControls>,
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
    async fn note(
        &self,
        id: &str,
        generation: &str,
        stamp: Option<u64>,
        patch: Value,
    ) -> Result<()> {
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
            if !self.confirmed.confirm(row, stamp, &patch) {
                return Ok(());
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
            // Same precedence as live snapshots: launch label, then the user's
            // cwd rename, then the hub's automatic title.
            return Ok(recent(&raw, |id, cwd| {
                let named = |value: &Value| {
                    value
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_owned)
                };
                let record = records.get(id);
                record
                    .and_then(|r| named(&r.metadata["label"]))
                    .or_else(|| named(&names[cwd]))
                    .or_else(|| record.and_then(|r| named(&r.metadata["autoTitle"]["title"])))
                    .unwrap_or_default()
            }));
        }
        let id = text(&p, "sessionId");
        if method == "claude.setEffort" && (id.is_empty() || text(&p, "effort").trim().is_empty()) {
            return Ok(failed("requires a session and an effort level"));
        }
        if method == "claude.setModel"
            && text(&p, "model").is_empty()
            && text(&p, "modelIdentity").is_empty()
            && p["contextWindow"].is_null()
            && text(&p, "effort").trim().is_empty()
        {
            bail!("claude.setModel requires {{ sessionId, model and/or effort }}");
        }
        segment(id)?;
        let generation = self.generation(id);
        let stamp = self
            .rows
            .read()
            .unwrap()
            .get(id)
            .and_then(|row| self.confirmed.observe(row));
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
                        let bookkeeping=self.note(id,&generation,stamp,json!({"settings":{"permissionMode":mode},"livePermissionMode":mode})).await;
                        Ok(acknowledged(json!({"ok":true,"mode":mode}), bookkeeping))
                    }
                }
            }
            "claude.setEffort" => {
                let effort = text(&p, "effort").trim();
                if effort.is_empty() {
                    return Ok(failed("requires a session and an effort level"));
                }
                let response = self
                    .engine
                    .request(effort_command(&self.provider(id).await, id, effort))
                    .await;
                match response {
                    Err(error) => Ok(failed(error)),
                    Ok(value) if value["ok"] == false => {
                        Ok(failed("this session can't take input right now (ended)"))
                    }
                    Ok(_) => {
                        let bookkeeping = self
                            .note(
                                id,
                                &generation,
                                stamp,
                                json!({"settings":{"effort":effort}}),
                            )
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
                        let (result, patch) =
                            model_receipt(&reply, &payload, text(&p, "effort").trim());
                        Ok(match patch {
                            Some(patch) => {
                                acknowledged(result, self.note(id, &generation, stamp, patch).await)
                            }
                            None => result,
                        })
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
        handoff::authored(
            &self.home,
            Duration::from_secs(150),
            Duration::from_secs(1),
            |instruction| async move {
                self.engine
                    .request(Command::Message {
                        id: id.into(),
                        text: instruction,
                    })
                    .await
                    .map(|value| value["ok"] != false)
            },
            || async move {
                self.request("POST", format!("/sessions/{id}/handoff"), Some(json!({})))
                    .await
            },
        )
        .await
    }
}
fn effort_command(provider: &str, id: &str, effort: &str) -> Command {
    if provider == "claude" {
        Command::Message {
            id: id.into(),
            text: format!("/effort {effort}"),
        }
    } else {
        Command::Request {
            method: "POST".into(),
            path: format!("/sessions/{id}/model"),
            payload: Some(json!({"effort":effort})),
        }
    }
}
fn model_receipt(reply: &Value, payload: &Value, effort: &str) -> (Value, Option<Value>) {
    let model = reply["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| text(payload, "model"));
    let selection = owner_selection(reply, payload);
    let mut result = json!({"ok":true});
    if !model.is_empty() {
        result["model"] = model.into();
    }
    if let Some(selection) = &selection {
        result["requestedSelection"] = selection.clone();
    }
    let patch = if reply["queued"] == true {
        result["queued"] = true.into();
        None
    } else {
        let mut patch = json!({"settings":{}});
        if !model.is_empty() {
            patch["settings"]["model"] = model.into();
        }
        if !effort.is_empty() {
            patch["settings"]["effort"] = effort.into();
        }
        if let Some(selection) = selection {
            patch["requestedSelection"] = selection;
        }
        Some(patch)
    };
    if !text(reply, "disposition").is_empty() {
        result["disposition"] = reply["disposition"].clone();
    }
    (result, patch)
}
fn owner_selection(reply: &Value, payload: &Value) -> Option<Value> {
    let (source, model_key) = match reply.get("requested_selection").filter(|v| v.is_object()) {
        Some(source) => (source, "model"),
        None if payload["model_identity"].is_string() => (payload, "model_identity"),
        None => return None,
    };
    let mut selection = json!({});
    if let Some(model) = source.get(model_key) {
        selection["model"] = model.clone();
    }
    if let Some(window) = source.get("context_window") {
        selection["contextWindow"] = window.clone();
    }
    Some(selection)
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
    let Some(source) = raw.as_array() else {
        return json!([]);
    };
    // The legacy typed daemon decoder rejects the whole malformed response;
    // do not manufacture resumable rows by coercing known scalar fields.
    if source.iter().any(|row| {
        !row.is_null()
            && (!row.is_object()
                || [
                    "session_id",
                    "cwd",
                    "mode",
                    "provider",
                    "transport",
                    "updated_at",
                    "started_at",
                ]
                .iter()
                .any(|key| !row[*key].is_null() && !row[*key].is_string())
                || (!row["archived"].is_null() && !row["archived"].is_boolean()))
    }) {
        return json!([]);
    }
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
        confirmed: options.confirmed_controls.clone(),
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
    fn accepted_owner_selection_preserves_unknown_window_absence_and_explicit_null() {
        assert_eq!(
            owner_selection(
                &json!({"requested_selection":{"model":"owner"}}),
                &json!({"model_identity":"request","context_window":1000000})
            ),
            Some(json!({"model":"owner"}))
        );
        assert_eq!(
            owner_selection(
                &json!({"requested_selection":{"model":"owner","context_window":null}}),
                &json!({})
            ),
            Some(json!({"model":"owner","contextWindow":null}))
        );
        assert_eq!(
            owner_selection(&json!({}), &json!({"model_identity":"request"})),
            Some(json!({"model":"request"}))
        );
        for invalid in ["opus\n/help", "opus\u{001b}[201~/help"] {
            assert!(model_payload("claude", &json!({"model":invalid})).is_err());
        }
        assert_eq!(
            model_payload("claude", &json!({"model":"opus","effort":"  \t "})).unwrap(),
            json!({"model":"opus","model_identity":"opus"})
        );
    }
    #[test]
    fn effort_uses_provider_route_and_queued_model_ack_cannot_become_live_metadata() {
        match effort_command("claude", "s1", "high") {
            Command::Message { id, text } => {
                assert_eq!(id, "s1");
                assert_eq!(text, "/effort high");
            }
            _ => panic!("Claude effort must be a normal queued slash message"),
        }
        match effort_command("codex", "s2", "xhigh") {
            Command::Request {
                method,
                path,
                payload,
            } => {
                assert_eq!(method, "POST");
                assert_eq!(path, "/sessions/s2/model");
                assert_eq!(payload, Some(json!({"effort":"xhigh"})));
            }
            _ => panic!("managed effort must use the settings endpoint"),
        }
        let payload = json!({"model":"opus","model_identity":"opus","effort":"high"});
        let owner = json!({"model":"accepted","requested_selection":{"model":"accepted","context_window":1000000},"queued":true,"disposition":"queued"});
        let (result, patch) = model_receipt(&owner, &payload, "high");
        assert_eq!(
            result,
            json!({"ok":true,"model":"accepted","requestedSelection":{"model":"accepted","contextWindow":1000000},"queued":true,"disposition":"queued"})
        );
        assert!(
            patch.is_none(),
            "queued acknowledgment must never note a live switch"
        );
        let mut owner = owner;
        owner["queued"] = false.into();
        let (_, patch) = model_receipt(&owner, &payload, "high");
        assert_eq!(
            patch.unwrap(),
            json!({"settings":{"model":"accepted","effort":"high"},"requestedSelection":{"model":"accepted","contextWindow":1000000}})
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
    #[test]
    fn recent_response_types_defaults_and_stable_timestamp_order_match_daemon_contract() {
        let raw = json!([
            {"session_id":"first-tie","updated_at":"2026-09-29T00:00:00.123456789Z","started_at":"2026-09-28T00:00:00Z","cwd":"/work","provider":"codex","transport":"stream","mode":"stopped","archived":true},
            {"session_id":"second-tie","updated_at":"2026-09-29T00:00:00.123999999Z"},
            {"session_id":"invalid-time","updated_at":"bad"},
            {"session_id":"missing-time"},
            {"session_id":"agent-synthetic"}, null, {}
        ]);
        let rows = recent(&raw, |id, cwd| {
            if id == "first-tie" {
                format!("Label {cwd}")
            } else {
                String::new()
            }
        });
        let ids: Vec<_> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["sessionId"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["first-tie", "second-tie", "invalid-time", "missing-time"]
        );
        assert_eq!(rows[0]["provider"], "codex");
        assert_eq!(rows[0]["transport"], "stream");
        assert_eq!(rows[0]["name"], "Label /work");
        assert_eq!(rows[0]["updatedAt"], rows[1]["updatedAt"]);
        assert_eq!(rows[1]["mode"], "unknown");
        assert_eq!(rows[1]["transport"], "pty");
        assert_eq!(rows[1]["startedAt"], 0);
        for row in rows.as_array().unwrap() {
            assert!(row.get("costUSD").is_none() && row.get("billedTokens").is_none());
            assert_eq!(row["model"], "");
            assert_eq!(row["title"], "");
        }
        for key in [
            "session_id",
            "cwd",
            "mode",
            "provider",
            "transport",
            "updated_at",
            "started_at",
            "archived",
        ] {
            let mut malformed = raw.clone();
            malformed[1][key] = json!(42);
            assert_eq!(recent(&malformed, |_, _| String::new()), json!([]), "{key}");
        }
        assert_eq!(recent(&json!({}), |_, _| String::new()), json!([]));
    }
    #[tokio::test]
    async fn manual_hook_rows_keep_confirmed_controls_until_the_observed_life_ends() -> Result<()> {
        use claudemon::daemon::{
            ServeConfig,
            embedded::{EmbeddedDaemon, Options as EngineOptions},
        };
        let _single_engine = crate::backend::ENGINE_TEST_LOCK.lock().await;
        let dir = tempfile::tempdir()?;
        let mut engine = EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: dir.path().join("state.db"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
        )?;
        let ready = engine.ready().await?;
        let mut options = Options::default();
        options.engine = Some(engine.client());
        let confirmed = options.confirmed_controls.clone();
        let rows = options.session_snapshots.clone();
        let hub = crate::Hub::start(options)?;
        hub.ready().await?;
        let client = crate::client::Client::connect(&hub.handle()).await?;
        let mut events = client.events();
        client.topics(["agent.snapshot".into()].into()).await?;
        let controls = Controls {
            engine: engine.client(),
            confirmed,
            lifecycle: None,
            rows: rows.clone(),
            hub: hub.handle(),
            home: dir.path().into(),
            config: dir.path().into(),
        };
        for invalid in ["../../outside".to_owned(), String::new(), "a".repeat(129)] {
            assert!(
                controls
                    .call("claude.handoffAgentBrief", json!({"sessionId":invalid}))
                    .await
                    .is_err()
            );
        }
        assert!(
            !dir.path().join(".workspacer/handoffs").exists(),
            "invalid session identity must be refused before preparing a target"
        );
        assert_eq!(
            controls.call("claude.setEffort", json!({})).await?["ok"],
            false
        );
        assert!(
            controls
                .call("claude.setModel", json!({"sessionId":"manual"}))
                .await
                .is_err()
        );
        let http = reqwest::Client::new();
        for event in ["SessionStart", "Stop"] {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":event,"session_id":"manual","cwd":dir.path()}))
                .send()
                .await?
                .error_for_status()?;
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while rows
                .read()
                .unwrap()
                .get("manual")
                .is_none_or(|r| r["ambientState"] != "idle")
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        let history = controls
            .call(
                "sessions.recent",
                json!({"limit":0,"cwd":"ignored-caller-path"}),
            )
            .await?;
        let manual = history
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["sessionId"] == "manual")
            .unwrap();
        assert_eq!(manual["provider"], "claude");
        assert!(manual.get("costUSD").is_none() && manual.get("billedTokens").is_none());
        let cwd = manual["cwd"].as_str().unwrap();
        std::fs::write(
            dir.path().join("tui-names.json"),
            serde_json::to_vec(&json!({cwd:"Renamed project"}))?,
        )?;
        let history = controls.call("sessions.recent", json!({})).await?;
        assert_eq!(
            history
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["sessionId"] == "manual")
                .unwrap()["name"],
            "Renamed project"
        );
        // The engine has no paid provider. Exercise the exact bookkeeping seam
        // reached after an accepted control ACK against its real manual row.
        let stamp = controls.confirmed.observe(&rows.read().unwrap()["manual"]);
        controls
            .note(
                "manual",
                "",
                stamp,
                json!({"settings":{"effort":"high","model":"opus"},"livePermissionMode":"plan"}),
            )
            .await?;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if events
                    .recv()
                    .await?
                    .data
                    .is_some_and(|r| r["liveEffort"] == "high")
                {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await??;
        let previous_update = rows.read().unwrap()["manual"]["updated_at"].clone();
        for event in ["UserPromptSubmit", "Stop"] {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":event,"session_id":"manual","cwd":dir.path()}))
                .send()
                .await?
                .error_for_status()?;
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let row = client
                    .call("sessions.snapshot", json!({"sessionId":"manual"}))
                    .await?;
                if row["ambientState"] == "idle"
                    && row["liveEffort"] == "high"
                    && row["updated_at"] != previous_update
                {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        http.post(format!("http://{}/hook", ready.hook_addr))
            .json(&json!({"hook_event_name":"SessionStart","session_id":"manual","cwd":dir.path()}))
            .send()
            .await?
            .error_for_status()?;
        tokio::time::timeout(Duration::from_secs(3), async {
            while rows
                .read()
                .unwrap()
                .get("manual")
                .is_none_or(|r| r.get("liveEffort").is_some())
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        controls
            .note("manual", "", stamp, json!({"settings":{"effort":"low"}}))
            .await?;
        assert!(
            rows.read().unwrap()["manual"].get("liveEffort").is_none(),
            "late ACK cannot bind a restarted manual session"
        );
        hub.shutdown()?;
        engine.shutdown().await?;
        Ok(())
    }
}
