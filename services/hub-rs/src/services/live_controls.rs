//! Live switches and handoff/history adapters over the owned daemon.
mod confirmation;
mod handoff;
mod summary;
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
    /// The shared sidebar set (and through it the archive), for history rows.
    open: Option<Arc<super::session_open::SessionOpen>>,
    hub: Handle,
    home: PathBuf,
    config: PathBuf,
    utilities: Option<Arc<super::provider_utilities::Service>>,
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
            .and_then(|l| l.generation(id))
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
            } else if lifecycle.contains(id) {
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
        self.provider_transport(id).await.0
    }
    /// `(provider, transport)` for a session: the live row first, then the
    /// daemon's own row. Unknown provider reads as Claude and unknown
    /// transport as PTY, the daemon's legacy defaults.
    async fn provider_transport(&self, id: &str) -> (String, String) {
        let field = |row: &Value, key: &str| {
            row[key]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let cached = self
            .rows
            .read()
            .unwrap()
            .get(id)
            .map(|row| (field(row, "provider"), field(row, "transport")));
        let (provider, transport) = match cached {
            Some((Some(provider), Some(transport))) => (Some(provider), Some(transport)),
            cached => {
                let fetched = self
                    .request("GET", format!("/sessions/{id}"), None)
                    .await
                    .ok();
                let (cached_provider, cached_transport) = cached.unwrap_or_default();
                (
                    cached_provider.or_else(|| fetched.as_ref().and_then(|r| field(r, "provider"))),
                    cached_transport
                        .or_else(|| fetched.as_ref().and_then(|r| field(r, "transport"))),
                )
            }
        };
        (
            provider.unwrap_or_else(|| "claude".into()),
            transport.unwrap_or_else(|| "pty".into()),
        )
    }
    /// The display name precedence shared by recents and history: launch
    /// label, then the user's cwd rename, then the hub's automatic title.
    fn namer(&self) -> impl Fn(&str, &str) -> String + use<> {
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
        move |id, cwd| {
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
        }
    }
    async fn call(&self, method: &str, p: Value) -> Result<Value> {
        if method == "sessions.recent" {
            let mut raw = self
                .request("GET", "/sessions?include_archived=true".into(), None)
                .await
                .unwrap_or(json!([]));
            // Older sessions come from the permanent history store: claudemon
            // drops stopped rows from memory after seven days, History keeps
            // them. In-memory rows win (they carry live state).
            if let (Some(rows), Ok(page)) = (
                raw.as_array_mut(),
                self.request("GET", format!("/history?limit={RECENT_HISTORY_PAGE}"), None)
                    .await,
            ) {
                let known: std::collections::BTreeSet<String> = rows
                    .iter()
                    .filter_map(|row| row["session_id"].as_str().map(str::to_owned))
                    .collect();
                rows.extend(
                    page["sessions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|row| {
                            row["session_id"]
                                .as_str()
                                .is_some_and(|id| !known.contains(id))
                        })
                        .map(recent_history_row),
                );
            }
            let name = self.namer();
            return Ok(recent(&raw, |id, cwd| name(id, cwd)));
        }
        if method == "sessions.history" {
            let query = history_query(&p)?;
            let page = self
                .request("GET", format!("/history{query}"), None)
                .await?;
            let name = self.namer();
            let (hidden, sidebar) = match &self.open {
                Some(open) => (open.hidden_ids(), open.ids()),
                None => Default::default(),
            };
            let live = self.rows.read().unwrap().clone();
            let sessions: Vec<Value> = page["sessions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| history_row(row, &name, &hidden, &sidebar, &live))
                .collect();
            let mut reply = json!({ "sessions": sessions });
            if let Some(next) = page["next"].as_str().filter(|s| !s.is_empty()) {
                reply["next"] = next.into();
            }
            return Ok(reply);
        }
        if method == "sessions.forget" {
            let id = text(&p, "sessionId");
            segment(id)?;
            let receipt = match self
                .request("POST", format!("/sessions/{id}/forget"), Some(json!({})))
                .await
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    let status = error
                        .downcast_ref::<claudemon::daemon::embedded::CommandRejected>()
                        .map(|rejected| rejected.status);
                    match status {
                        Some(409) => bail!(
                            "This session is still running; stop it before deleting it from History"
                        ),
                        Some(404) => bail!("History has no session {id}"),
                        _ => return Err(error),
                    }
                }
            };
            // Deleted for good: no sidebar or archive entry may keep naming it.
            if let Some(open) = self.open.clone() {
                let owned = id.to_owned();
                tokio::task::spawn_blocking(move || open.forget_history(&owned)).await??;
            }
            return Ok(json!({
                "ok": true,
                "sessionId": id,
                "transcriptCopyRemoved": receipt["transcript_copy_removed"] == true,
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
                let (provider, transport) = self.provider_transport(id).await;
                let response = self
                    .engine
                    .request(effort_command(&provider, &transport, id, effort))
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
            "claude.handoffSummaryBrief" => self.summary_brief(id).await,
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
                        delivery: Default::default(),
                        sender: Default::default(),
                        images: Vec::new(),
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
    /// A cheap model writes the brief from the digest and the conversation's
    /// tail; nothing is sent to the source, so its cold context stays cold.
    async fn summary_brief(&self, id: &str) -> Result<Value> {
        let digest = match self
            .request(
                "POST",
                format!("/sessions/{id}/handoff"),
                Some(json!({"no_persist":true})),
            )
            .await
        {
            Ok(reply) => text(&reply, "markdown").to_owned(),
            Err(error) => return Ok(failed(error)),
        };
        // The tail is an enrichment: the digest alone still makes a brief.
        let items = self
            .request(
                "GET",
                format!("/sessions/{id}/conversation?limit={}", summary::TAIL_ITEMS),
                None,
            )
            .await
            .ok()
            .and_then(|reply| reply["items"].as_array().cloned())
            .unwrap_or_default();
        let provider = self.provider(id).await;
        summary::summarized(
            &self.home,
            id,
            &digest,
            &items,
            summary::DEADLINE,
            |prompt| async move {
                match &self.utilities {
                    Some(utilities) => {
                        let outcome = utilities.summarize_handoff(&provider, &prompt).await;
                        summary::Attempt {
                            provider: outcome.provider,
                            model: outcome.model,
                            text: outcome.text,
                        }
                    }
                    None => summary::Attempt {
                        provider,
                        model: None,
                        text: Err("unsupported"),
                    },
                }
            },
            || async move {
                self.request("POST", format!("/sessions/{id}/handoff"), Some(json!({})))
                    .await
            },
        )
        .await
    }
}
/// The live effort switch. Only a Claude PTY session takes the interactive
/// `/effort <level>` command, typed into its TUI. Every managed session —
/// Claude stream included — switches through the daemon's settings endpoint:
/// the Claude stream driver turns it into the CLI's `apply_flag_settings
/// {effortLevel}` control request (applied from the next turn) instead of a
/// chat message. A `/effort` chat message on a stream session queues behind a
/// running turn, echoes a command card, and its only confirmation is a
/// synthetic local-command reply the stream view drops — so it was neither
/// prompt nor observable.
fn effort_command(provider: &str, transport: &str, id: &str, effort: &str) -> Command {
    if provider == "claude" && transport != "stream" {
        Command::Message {
            id: id.into(),
            text: format!("/effort {effort}"),
            delivery: Default::default(),
            sender: Default::default(),
            images: Vec::new(),
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
/// History rows merged into `sessions.recent` (beyond claudemon's memory).
const RECENT_HISTORY_PAGE: usize = 200;
const MAX_HISTORY_LIMIT: u64 = 200;

/// A history row in the in-memory row shape `recent` reads.
fn recent_history_row(row: &Value) -> Value {
    let mut out = json!({});
    for key in [
        "session_id",
        "cwd",
        "mode",
        "provider",
        "transport",
        "updated_at",
        "started_at",
    ] {
        if row[key].is_string() {
            out[key] = row[key].clone();
        }
    }
    out
}

/// `sessions.history` params as claudemon's `/history` query string.
fn history_query(p: &Value) -> Result<String> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    let optional = |key: &str| -> Result<Option<&str>> {
        match &p[key] {
            Value::Null => Ok(None),
            Value::String(value) => Ok(Some(value.as_str())),
            _ => bail!("sessions.history: {key} must be text"),
        }
    };
    if let Some(cursor) = optional("cursor")?.filter(|s| !s.is_empty()) {
        let (at, id) = cursor
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("sessions.history: invalid cursor"))?;
        anyhow::ensure!(
            cursor.len() <= 160
                && !at.is_empty()
                && at.bytes().all(|b| b.is_ascii_digit() || b == b'-')
                && segment(id).is_ok(),
            "sessions.history: invalid cursor"
        );
        query.append_pair("before", cursor);
    }
    match &p["limit"] {
        Value::Null => {}
        value => {
            let limit = value
                .as_u64()
                .filter(|n| (1..=MAX_HISTORY_LIMIT).contains(n))
                .ok_or_else(|| {
                    anyhow::anyhow!("sessions.history: limit must be 1-{MAX_HISTORY_LIMIT}")
                })?;
            query.append_pair("limit", &limit.to_string());
        }
    }
    for (key, wire, max) in [("query", "q", 200), ("cwd", "cwd", 4096)] {
        if let Some(value) = optional(key)?.filter(|s| !s.is_empty()) {
            anyhow::ensure!(
                value.len() <= max && !value.chars().any(char::is_control),
                "sessions.history: invalid {key}"
            );
            query.append_pair(wire, value);
        }
    }
    let query = query.finish();
    Ok(if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    })
}

/// One claudemon history row as a `sessions.history` row: the recent-row
/// shape plus Hide/sidebar state and the permanent record's fields. A live
/// session reports its live mode (the store only records it as stopped).
fn history_row(
    row: &Value,
    name: &impl Fn(&str, &str) -> String,
    hidden: &std::collections::BTreeSet<String>,
    sidebar: &std::collections::BTreeSet<String>,
    live: &BTreeMap<String, Value>,
) -> Option<Value> {
    let id = row["session_id"].as_str().filter(|id| !id.is_empty())?;
    let field = |key: &str, fallback: &str| {
        row[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback)
            .to_owned()
    };
    let millis = |key: &str| {
        row[key]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis())
    };
    let seconds = |key: &str| row[key].as_i64().map(|s| s.saturating_mul(1000));
    let mode = live
        .get(id)
        .and_then(|row| row["mode"].as_str())
        .filter(|mode| !mode.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| field("mode", "stopped"));
    let model = row["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| row["requested_model"].as_str())
        .unwrap_or("");
    let mut out = json!({
        "sessionId": id,
        "provider": field("provider", "claude"),
        "cwd": text(row, "cwd"),
        "mode": mode,
        "transport": field("transport", "pty"),
        "archived": row["archived"] == true,
        "updatedAt": millis("updated_at").or_else(|| seconds("last_event_at")).unwrap_or(0),
        "startedAt": millis("started_at").or_else(|| seconds("created_at")).unwrap_or(0),
        "name": name(id, text(row, "cwd")),
        "title": text(row, "title"),
        "model": model,
        "hidden": hidden.contains(id),
        "onSidebar": sidebar.contains(id),
        "transcriptCopy": row["transcript_copy"] == true,
    });
    if let Some(ended) = seconds("ended_at") {
        out["endedAt"] = ended.into();
    }
    if let Some(prompt) = row["first_prompt"].as_str().filter(|s| !s.is_empty()) {
        out["firstPrompt"] = prompt.into();
    }
    for (wire, key) in [("userPrompts", "user_prompts"), ("toolCalls", "tool_calls")] {
        if let Some(count) = row[key].as_u64() {
            out[wire] = count.into();
        }
    }
    Some(out)
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
        open: options.session_open.clone(),
        hub,
        home: options.home_dir.clone().unwrap_or_default(),
        config: options.config_dir.clone().unwrap_or_default(),
        utilities: options.provider_utilities.clone(),
    });
    for method in [
        "claude.setModel",
        "claude.setEffort",
        "claude.setPermissionMode",
        "claude.handoffBrief",
        "claude.handoffAgentBrief",
        "claude.handoffSummaryBrief",
        "sessions.recent",
        "sessions.history",
        "sessions.forget",
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
        match effort_command("claude", "pty", "s1", "high") {
            Command::Message { id, text, .. } => {
                assert_eq!(id, "s1");
                assert_eq!(text, "/effort high");
            }
            _ => panic!("Claude PTY effort must be a normal queued slash message"),
        }
        for (provider, transport) in [("codex", "stream"), ("claude", "stream")] {
            match effort_command(provider, transport, "s2", "xhigh") {
                Command::Request {
                    method,
                    path,
                    payload,
                } => {
                    assert_eq!(method, "POST");
                    assert_eq!(path, "/sessions/s2/model");
                    assert_eq!(payload, Some(json!({"effort":"xhigh"})));
                }
                _ => panic!("{provider} {transport} effort must use the settings endpoint"),
            }
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
            open: None,
            hub: hub.handle(),
            home: dir.path().into(),
            config: dir.path().into(),
            utilities: None,
        };
        for invalid in ["../../outside".to_owned(), String::new(), "a".repeat(129)] {
            for method in ["claude.handoffAgentBrief", "claude.handoffSummaryBrief"] {
                assert!(
                    controls
                        .call(method, json!({"sessionId":invalid.clone()}))
                        .await
                        .is_err()
                );
            }
        }
        // No conversation: nothing to summarize, and no model is asked.
        let unknown = controls
            .call("claude.handoffSummaryBrief", json!({"sessionId":"unknown"}))
            .await?;
        assert_eq!(unknown["ok"], false, "{unknown}");
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

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn history_params_map_to_the_daemon_query_and_reject_bad_input() {
        assert_eq!(history_query(&json!({})).unwrap(), "");
        assert_eq!(
            history_query(
                &json!({"cursor":"1700000000:abc-1","limit":25,"query":"fix bug","cwd":"/w/x"})
            )
            .unwrap(),
            "?before=1700000000%3Aabc-1&limit=25&q=fix+bug&cwd=%2Fw%2Fx"
        );
        assert_eq!(
            history_query(&json!({"cursor":"","query":"","cwd":null})).unwrap(),
            ""
        );
        for bad in [
            json!({"limit":0}),
            json!({"limit":201}),
            json!({"limit":"5"}),
            json!({"cursor":"nocolon"}),
            json!({"cursor":"12:../x"}),
            json!({"cursor":"ab:id"}),
            json!({"query":"a\nb"}),
            json!({"query":"x".repeat(201)}),
            json!({"cwd":7}),
        ] {
            assert!(history_query(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn history_rows_carry_hide_sidebar_live_mode_and_record_fields() {
        let hidden = ["h".to_owned()].into();
        let sidebar = ["h".to_owned(), "l".to_owned()].into();
        let live = BTreeMap::from([("l".to_owned(), json!({"mode":"responding"}))]);
        let name = |id: &str, _: &str| {
            if id == "h" {
                "Label".into()
            } else {
                String::new()
            }
        };
        let row = history_row(
            &json!({"session_id":"h","cwd":"/w","mode":"stopped","provider":"codex","transport":"stream",
                    "updated_at":"2026-11-01T00:00:00Z","started_at":"2026-10-31T00:00:00Z","title":"Thread",
                    "requested_model":"gpt-5","ended_at":1_700_000_000,"first_prompt":"fix it",
                    "user_prompts":3,"tool_calls":9,"transcript_copy":true,"history":true}),
            &name,
            &hidden,
            &sidebar,
            &live,
        )
        .unwrap();
        assert_eq!(
            row,
            json!({"sessionId":"h","provider":"codex","cwd":"/w","mode":"stopped","transport":"stream",
                   "archived":false,"updatedAt":1_793_491_200_000_i64,"startedAt":1_793_404_800_000_i64,
                   "name":"Label","title":"Thread","model":"gpt-5","hidden":true,"onSidebar":true,
                   "transcriptCopy":true,"endedAt":1_700_000_000_000_i64,"firstPrompt":"fix it",
                   "userPrompts":3,"toolCalls":9})
        );
        // A live session reports its live mode; seconds-only rows still date.
        let row = history_row(
            &json!({"session_id":"l","mode":"stopped","last_event_at":10,"created_at":5}),
            &name,
            &hidden,
            &sidebar,
            &live,
        )
        .unwrap();
        assert_eq!(row["mode"], "responding");
        assert_eq!(row["provider"], "claude");
        assert_eq!(row["transport"], "pty");
        assert_eq!(row["updatedAt"], 10_000);
        assert_eq!(row["startedAt"], 5_000);
        assert_eq!(
            (row["hidden"].clone(), row["onSidebar"].clone()),
            (json!(false), json!(true))
        );
        assert!(row.get("endedAt").is_none() && row.get("firstPrompt").is_none());
        assert!(history_row(&json!({"cwd":"/w"}), &name, &hidden, &sidebar, &live).is_none());
    }

    /// The real engine: hook-recorded sessions reach `sessions.history`
    /// (paged, searchable, labelled), `sessions.recent` still lists live rows,
    /// and `sessions.forget` refuses a live session and deletes a stopped one
    /// from History, the sidebar and the archive.
    #[tokio::test]
    async fn history_and_forget_over_the_embedded_engine() -> Result<()> {
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
        let rows = options.session_snapshots.clone();
        let hub = crate::Hub::start(options)?;
        hub.ready().await?;
        let archive = Arc::new(super::super::session_archive::SessionArchive::open(
            None, None,
        ));
        let open = Arc::new(super::super::session_open::SessionOpen::open(
            None,
            None,
            Some(archive.clone()),
            None,
        ));
        let controls = Controls {
            engine: engine.client(),
            confirmed: Default::default(),
            lifecycle: None,
            rows: rows.clone(),
            open: Some(open.clone()),
            hub: hub.handle(),
            home: dir.path().into(),
            config: dir.path().into(),
            utilities: None,
        };
        let http = reqwest::Client::new();
        let hook = |event: &str, id: &str| {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":event,"session_id":id,"cwd":dir.path(),"prompt":"history probe"}))
                .send()
        };
        for (event, id) in [
            ("SessionStart", "done"),
            ("UserPromptSubmit", "done"),
            ("SessionEnd", "done"),
            ("SessionStart", "live"),
        ] {
            hook(event, id).await?.error_for_status()?;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = controls.call("sessions.history", json!({})).await?;
                let ids: Vec<&str> = page["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|r| r["sessionId"].as_str())
                    .collect();
                if ids.contains(&"done")
                    && ids.contains(&"live")
                    && rows.read().unwrap().contains_key("live")
                {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await??;
        let cwd = dir.path().to_string_lossy().into_owned();
        std::fs::write(
            dir.path().join("tui-names.json"),
            serde_json::to_vec(&json!({&cwd:"Probe project"}))?,
        )?;
        archive.set(json!({"sessionId":"done","archived":true}))?;
        open.set(json!({"sessionId":"live","open":true}))?;
        let page = controls.call("sessions.history", json!({})).await?;
        let row = |id: &str| {
            page["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["sessionId"] == id)
                .cloned()
                .unwrap()
        };
        let (done, live) = (row("done"), row("live"));
        assert_eq!(done["hidden"], true);
        assert_eq!(done["onSidebar"], false);
        assert_eq!(done["name"], "Probe project");
        assert_eq!(done["mode"], "stopped");
        assert_eq!(live["hidden"], false);
        assert_eq!(live["onSidebar"], true);
        assert_ne!(
            live["mode"], "stopped",
            "a live session reports its live mode"
        );

        // Paging: one row per page, then the cursor reaches the other.
        let first = controls
            .call("sessions.history", json!({"limit":1}))
            .await?;
        assert_eq!(first["sessions"].as_array().unwrap().len(), 1);
        let cursor = first["next"].as_str().unwrap().to_owned();
        let second = controls
            .call("sessions.history", json!({"limit":1,"cursor":cursor}))
            .await?;
        assert_eq!(second["sessions"].as_array().unwrap().len(), 1);
        assert_ne!(
            first["sessions"][0]["sessionId"],
            second["sessions"][0]["sessionId"]
        );
        // Search passes through to the store.
        let found = controls
            .call("sessions.history", json!({"query":"don"}))
            .await?;
        let found: Vec<&Value> = found["sessions"].as_array().unwrap().iter().collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["sessionId"], "done");
        assert!(
            controls
                .call("sessions.history", json!({"limit":0}))
                .await
                .is_err()
        );

        // Recents still list the live in-memory row.
        let recent = controls.call("sessions.recent", json!({})).await?;
        assert!(
            recent
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["sessionId"] == "live")
        );

        // Forget: a live session is refused with a clear reason.
        let refused = controls
            .call("sessions.forget", json!({"sessionId":"live"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("still running"), "{refused}");
        assert!(open.ids().contains("live"));
        // A stopped one is deleted from History, the sidebar and the archive.
        open.set(json!({"sessionId":"done","open":true})).ok();
        let receipt = controls
            .call("sessions.forget", json!({"sessionId":"done"}))
            .await?;
        assert_eq!(receipt["ok"], true);
        assert_eq!(receipt["sessionId"], "done");
        assert!(archive.get()["archived"].get("done").is_none());
        assert!(!open.ids().contains("done"));
        let page = controls.call("sessions.history", json!({})).await?;
        assert!(
            page["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["sessionId"] != "done")
        );
        let unknown = controls
            .call("sessions.forget", json!({"sessionId":"done"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("no session"), "{unknown}");
        assert!(
            controls
                .call("sessions.forget", json!({"sessionId":"../x"}))
                .await
                .is_err()
        );
        hub.shutdown()?;
        engine.shutdown().await?;
        Ok(())
    }
}
