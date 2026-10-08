//! The existing desktop/headless snapshot projection. Provider facts are
//! retained alongside camelCase fields; unknown measurements stay unknown.
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn compat(mut snapshot: Value) -> Value {
    if snapshot["session_id"].as_str().unwrap_or("").is_empty() {
        return snapshot;
    }
    snapshot["sessionId"] = snapshot["session_id"].clone();
    snapshot["sparse"] = json!(true);
    if let Some(value) = snapshot.get("execution_engine").cloned() {
        snapshot["executionEngine"] = value;
    }
    let mode = snapshot["mode"].as_str().unwrap_or("").to_owned();
    snapshot["status"] = json!(if mode == "stopped" { "ended" } else { "active" });
    let background = snapshot["background_tasks"]
        .as_f64()
        .unwrap_or(0.0)
        .max(0.0) as u64;
    if background > 0 {
        snapshot["backgroundTasks"] = json!(background);
    }
    let ambient = match mode.as_str() {
        "responding" => Some("streaming"),
        "approval" => Some("waiting_approval"),
        "question" => Some("waiting_input"),
        "input" if background > 0 => Some("background"),
        "input" | "stopped" => Some("idle"),
        _ => None,
    };
    if let Some(ambient) = ambient {
        snapshot["ambientState"] = json!(ambient);
    }
    if let Some(at) = snapshot["updated_at"].as_str().and_then(parse_time) {
        snapshot["lastActivity"] = json!(at.unix_timestamp_nanos() / 1_000_000);
    }
    if let Some(usage) = snapshot["usage"].as_object() {
        let mut mapped = json!({"model":usage.get("model"),"contextTokens":usage.get("context_tokens"),"costUSD":usage.get("cost_usd")});
        for (from, to) in [("context_limit", "contextLimit"), ("cache", "cache")] {
            if let Some(value) = usage.get(from).filter(|v| !v.is_null()) {
                mapped[to] = value.clone();
            }
        }
        snapshot["usage"] = mapped;
    }
    // When the prompt cache expires (claudemon `session::prompt_cache`).
    // Clients judge warm/cold against their own clock, so it is copied whole.
    if let Some(cache) = snapshot["prompt_cache"].as_object() {
        let mut mapped = json!({});
        for (from, to) in [
            ("ttl_seconds", "ttlSeconds"),
            ("last_request_at", "lastRequestAt"),
            ("expires_at", "expiresAt"),
            ("context_tokens", "contextTokens"),
            ("estimated", "estimated"),
            ("model", "model"),
            ("cold_cost_usd", "coldCostUSD"),
            ("warm_cost_usd", "warmCostUSD"),
        ] {
            if let Some(value) = cache.get(from).filter(|v| !v.is_null()) {
                mapped[to] = value.clone();
            }
        }
        snapshot["promptCache"] = mapped;
    }
    if let Some(value) = snapshot.get("tool_calls").cloned() {
        snapshot["totalToolCalls"] = value;
    }
    if let Some(selection) = snapshot["requested_selection"].as_object() {
        let mut mapped = json!({});
        for (from, to) in [("model", "model"), ("context_window", "contextWindow")] {
            if let Some(value) = selection.get(from) {
                mapped[to] = value.clone();
            }
        }
        snapshot["requestedSelection"] = mapped;
    }
    if let Some(value) = snapshot
        .get("resolved_context_window")
        .filter(|v| !v.is_null())
        .cloned()
    {
        snapshot["resolvedContextWindow"] = value;
    }
    if let Some(status) = snapshot["status_line"].as_object() {
        let mut mapped = json!({});
        for (from, to) in [
            ("model_display", "modelDisplay"),
            ("effort", "effort"),
            ("context_used_pct", "contextUsedPct"),
            ("context_window_size", "contextWindowSize"),
            ("total_input_tokens", "totalInputTokens"),
            ("total_output_tokens", "totalOutputTokens"),
            ("cached_input_tokens", "cachedInputTokens"),
            ("cost_usd", "costUSD"),
            ("five_hour_pct", "fiveHourPct"),
            ("five_hour_resets_at", "fiveHourResetsAt"),
            ("five_hour_window_minutes", "fiveHourWindowMins"),
            ("seven_day_pct", "sevenDayPct"),
            ("seven_day_resets_at", "sevenDayResetsAt"),
            ("seven_day_window_minutes", "sevenDayWindowMins"),
            ("monthly_pct", "monthlyPct"),
            ("monthly_resets_at", "monthlyResetsAt"),
            ("monthly_window_minutes", "monthlyWindowMins"),
            ("rate_limit_warning", "rateLimitWarning"),
            ("overage_out_of_credits", "overageOutOfCredits"),
            ("capabilities", "capabilities"),
            ("received_at", "receivedAt"),
        ] {
            mapped[to] = status.get(from).cloned().unwrap_or(Value::Null);
        }
        if status.get("context_usage_state") == Some(&json!("waiting_for_runtime_usage")) {
            mapped["contextUsageState"] = json!("waitingForRuntimeUsage");
        }
        if let Some(health) = status.get("context_health").and_then(Value::as_object) {
            let mut mapped_health = json!({});
            for (from, to) in [
                ("used_tokens", "usedTokens"),
                ("window_tokens", "windowTokens"),
                ("used_pct", "usedPct"),
                ("window_source", "windowSource"),
                ("observed_at", "observedAt"),
                ("epoch", "epoch"),
                ("provider", "provider"),
            ] {
                mapped_health[to] = health.get(from).cloned().unwrap_or(Value::Null);
            }
            mapped["contextHealth"] = mapped_health;
        }
        snapshot["statusLine"] = mapped;
    }
    snapshot["pendingApproval"] = Value::Null;
    snapshot["pendingQuestions"] = Value::Null;
    let pending = snapshot["pending"].clone();
    match pending["kind"].as_str() {
        Some("approval") => {
            let raw = &pending["raw"];
            let input = raw
                .get("tool_input")
                .or_else(|| raw.get("input"))
                .unwrap_or(raw);
            snapshot["pendingApproval"] = json!({"toolName":pending["tool"],"toolInput":input});
        }
        Some("question") => snapshot["pendingQuestions"] = pending["questions"].clone(),
        _ => (),
    }
    snapshot
}

/// Optional TUI cwd display names never replace a recorded/present label and
/// never label another hub's row using this host's local configuration.
pub fn with_cwd_name(mut row: Value, names: &Value) -> Value {
    if row.get("label").is_none() && row["hub"].as_str().unwrap_or("").is_empty() {
        if let Some(cwd) = row["cwd"].as_str().filter(|cwd| !cwd.is_empty()) {
            if let Some(name) = names[cwd].as_str().filter(|name| !name.is_empty()) {
                row["label"] = json!(name);
            }
        }
    }
    row
}

/// Lowest-precedence name: a hub-written automatic title (see
/// `sessions::titles`). A launch label and a user's cwd rename both win, so a
/// late title can never replace a name a person gave.
pub fn with_auto_title(mut row: Value) -> Value {
    let unnamed = row["label"]
        .as_str()
        .is_none_or(|label| label.trim().is_empty());
    if unnamed && row["hub"].as_str().unwrap_or("").is_empty() {
        if let Some(title) = row["autoTitle"]["title"]
            .as_str()
            .filter(|title| !title.trim().is_empty())
        {
            row["label"] = json!(title);
        }
    }
    row
}

fn parse_time(raw: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).ok()
}

pub fn layout_ids(layout: &Value) -> (BTreeSet<String>, bool) {
    fn text<'a>(row: &'a Value, key: &str) -> Result<Option<&'a str>, ()> {
        match row.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value)),
            _ => Err(()),
        }
    }
    fn rows(value: &Value) -> Result<&[Value], ()> {
        if value.is_null() {
            Ok(&[])
        } else {
            value.as_array().map(Vec::as_slice).ok_or(())
        }
    }
    fn object(value: &Value) -> Result<(), ()> {
        if value.is_object() || value.is_null() {
            Ok(())
        } else {
            Err(())
        }
    }
    let parsed = (|| -> Result<BTreeSet<String>, ()> {
        // Only a present agents array is a curation decision. Empty data or a
        // null/malformed agents field retains the no-layout fallback.
        let agents = layout
            .get("data")
            .and_then(|data| data.get("agents"))
            .and_then(Value::as_array)
            .ok_or(())?;
        let mut ids = BTreeSet::new();
        for agent in agents {
            object(agent)?;
            let global = match agent.get("global") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(global)) => *global,
                _ => return Err(()),
            };
            let mut selected = Vec::new();
            for key in ["sessionId", "lastSessionId"] {
                if let Some(id) = text(agent, key)?.filter(|id| !id.is_empty()) {
                    selected.push(id);
                }
            }
            for tab in rows(&agent["tabs"])? {
                object(tab)?;
                for pane in rows(&tab["panes"])? {
                    object(pane)?;
                    if let Some(id) = text(pane, "attachSessionId")?.filter(|id| !id.is_empty()) {
                        selected.push(id);
                    }
                }
            }
            // Validate even ignored Overview rows, as the Go typed decoder did.
            if !global {
                ids.extend(selected.into_iter().map(str::to_owned));
            }
        }
        Ok(ids)
    })();
    match parsed {
        Ok(ids) => (ids, true),
        Err(()) => (BTreeSet::new(), false),
    }
}
fn state_fields_valid(snapshot: &Value, strings: &[&str]) -> bool {
    snapshot.is_object()
        && strings
            .iter()
            .all(|key| snapshot[*key].is_null() || snapshot[*key].is_string())
        && (snapshot["archived"].is_null() || snapshot["archived"].is_boolean())
}
/// Process liveness without UI curation or a caller-specific locality rule.
pub fn live(snapshot: &Value) -> bool {
    if !state_fields_valid(snapshot, &["mode", "status"]) || snapshot["archived"] == true {
        return false;
    }
    let mode = snapshot["mode"].as_str().unwrap_or("");
    !matches!(mode, "unknown" | "stopped") && !(mode.is_empty() && snapshot["status"] == "ended")
}

pub fn visible(snapshot: &Value, layout: &Value, now: time::OffsetDateTime) -> bool {
    if !state_fields_valid(snapshot, &["session_id", "mode", "status", "updated_at"])
        || snapshot["mode"] == "unknown"
    {
        return false;
    }
    let mode = snapshot["mode"].as_str().unwrap_or("");
    if mode != "stopped" && !(mode.is_empty() && snapshot["status"] == "ended") {
        return true;
    }
    let (ids, has_layout) = layout_ids(layout);
    if ids.contains(snapshot["session_id"].as_str().unwrap_or("")) {
        return true;
    }
    if has_layout || snapshot["archived"] == true {
        return false;
    }
    snapshot["updated_at"]
        .as_str()
        .and_then(parse_time)
        .is_some_and(|updated| now - updated <= time::Duration::hours(24))
}
pub(crate) fn with_host_metadata(
    mut row: serde_json::Value,
    lifecycle: Option<&super::agent_lifecycle::Lifecycle>,
    replacements: Option<&super::manager_replacements::ReplacementState>,
) -> serde_json::Value {
    if let Some(lifecycle) = lifecycle {
        row = lifecycle.enrich(row);
    }
    // Replacement journals restore lineage and fill launch settings, but their
    // saved launch tuple must not roll back an acknowledged live control.
    let controls: Vec<_> = ["model", "effort", "permissionMode"]
        .into_iter()
        .filter_map(|key| {
            row["settings"][key]
                .as_str()
                .map(|value| (key, value.to_owned()))
        })
        .collect();
    if let Some(replacements) = replacements {
        row = replacements.enrich(row);
    }
    if !controls.is_empty() {
        if !row["settings"].is_object() {
            row["settings"] = json!({});
        }
        for (key, value) in controls {
            row["settings"][key] = value.into();
        }
    }
    row
}

#[cfg(test)]
mod confirmed_control_tests {
    use super::*;
    use crate::services::{
        agent_lifecycle::{LaunchEngine, LaunchPreparation, Lifecycle, Operation},
        manager_replacements::ReplacementState,
        spawn_plan::Plan,
    };
    use std::sync::Arc;
    struct NoProcess;
    impl LaunchEngine for NoProcess {
        fn spawn<'a>(&'a self, _: &'a Plan) -> Operation<'a, Value> {
            Box::pin(async { anyhow::bail!("fixture must not launch a process") })
        }
        fn stop<'a>(&'a self, _: &'a str) -> Operation<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    impl LaunchPreparation for NoProcess {
        fn prepare<'a>(&'a self, _: &'a mut Plan, _: &'a str) -> Operation<'a, ()> {
            Box::pin(async { anyhow::bail!("fixture must not prepare a process") })
        }
        fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Operation<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    #[test]
    fn prompt_cache_reaches_clients_in_camel_case() {
        let row = compat(json!({
            "session_id": "s", "mode": "stopped",
            "prompt_cache": {"ttl_seconds": 3600, "last_request_at": 1_000,
                "expires_at": 3_601_000, "context_tokens": 624_000, "estimated": false,
                "model": "claude-opus-4-8", "cold_cost_usd": 6.24, "warm_cost_usd": 0.312}
        }));
        assert_eq!(
            row["promptCache"],
            json!({"ttlSeconds": 3600, "lastRequestAt": 1_000, "expiresAt": 3_601_000,
                "contextTokens": 624_000, "estimated": false, "model": "claude-opus-4-8",
                "coldCostUSD": 6.24, "warmCostUSD": 0.312})
        );
        // Unknown prices stay absent rather than reading as free.
        let row = compat(
            json!({"session_id": "s", "prompt_cache": {"ttl_seconds": 600,
            "last_request_at": 1, "expires_at": 600_001, "context_tokens": 9, "estimated": true}}),
        );
        assert!(row["promptCache"].get("coldCostUSD").is_none());
        assert!(
            compat(json!({"session_id": "s"}))
                .get("promptCache")
                .is_none()
        );
    }
    #[tokio::test]
    async fn durable_live_controls_survive_lineage_recovery_without_changing_parent_role_or_profile()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("launch.json");
        std::fs::write(&path,serde_json::to_vec(&json!({"manager":{"generation":"current","provider":"claude","cwd":dir.path(),"phase":"running","revocationPending":true,"receipt":{},"metadata":{"parentSessionId":"original-parent","settings":{"model":"original","effort":"low","permissionMode":"plan"}}}})).unwrap()).unwrap();
        let lifecycle =
            Lifecycle::open(path.clone(), Arc::new(NoProcess), Arc::new(NoProcess)).unwrap();
        let state = ReplacementState::open(dir.path().join("lineage.json")).unwrap();
        state.remember_child(json!({"sessionId":"manager","cwd":dir.path(),"isWakeTarget":true,"parentSessionId":"adopted-parent","settings":{"model":"saved-model","effort":"low","permissionMode":"plan","profileId":"retained-profile"}})).unwrap();
        assert!(lifecycle.note_live_control("manager","current",&json!({"settings":{"model":"confirmed-model","effort":"high","permissionMode":"default"},"livePermissionMode":"default"})).await.unwrap());
        let lifecycle = Lifecycle::open(path, Arc::new(NoProcess), Arc::new(NoProcess)).unwrap();
        let row = with_host_metadata(
            json!({"sessionId":"manager","status":"active"}),
            Some(&lifecycle),
            Some(&state),
        );
        assert_eq!(
            row["settings"],
            json!({"model":"confirmed-model","effort":"high","permissionMode":"default","profileId":"retained-profile"})
        );
        assert_eq!(row["parentSessionId"], "adopted-parent");
        assert_eq!(row["isWakeTarget"], true);
        assert_eq!(row["livePermissionMode"], "default");
        let legacy = with_host_metadata(
            json!({"sessionId":"manager","settings":{"effort":"medium"}}),
            None,
            Some(&state),
        );
        assert_eq!(legacy["settings"]["effort"], "medium");
        assert_eq!(legacy["settings"]["model"], "saved-model");
        let remote = json!({"sessionId":"manager","hub":"peer","settings":{"model":"peer-model"}});
        assert_eq!(
            with_host_metadata(remote.clone(), Some(&lifecycle), Some(&state)),
            remote
        );
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[tokio::test]
    async fn registered_list_and_snapshots_use_the_same_live_and_curated_set() {
        let _engine_guard = crate::backend::ENGINE_TEST_LOCK.lock().await;
        use claudemon::daemon::{
            ServeConfig,
            embedded::{EmbeddedDaemon, Options as EngineOptions},
        };
        use std::sync::{Arc, RwLock};
        let root = tempfile::tempdir().unwrap();
        let mut engine = EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: root.path().join("state.db"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
        )
        .unwrap();
        engine.ready().await.unwrap();
        let mut options = crate::Options::default();
        options.engine = Some(engine.client());
        options.home_dir = Some(root.path().into());
        options.config_dir = Some(root.path().join("config"));
        let layout = Arc::new(RwLock::new(
            json!({"version":1,"data":{"agents":[{"lastSessionId":"stopped-curated"}]}}),
        ));
        options.upstream_layout = Some(layout.clone());
        let rows = options.session_snapshots.clone();
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        for (id, mode) in [
            ("live-idle", "input"),
            ("live-working", "responding"),
            ("terminal", "unknown"),
            ("stopped-curated", "stopped"),
            ("stopped-old", "stopped"),
        ] {
            rows.write().unwrap().insert(
                id.into(),
                compat(json!({"session_id":id,"mode":mode,"updated_at":"2020-01-01T00:00:00Z"})),
            );
        }
        let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
        for method in ["agents.list", "sessions.snapshots"] {
            let rows = client.call(method, json!({})).await.unwrap();
            let ids: BTreeSet<_> = rows
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["sessionId"].as_str().unwrap())
                .collect();
            assert_eq!(
                ids,
                ["live-idle", "live-working", "stopped-curated"].into(),
                "{method}"
            );
        }
        *layout.write().unwrap() = json!({"version":2,"data":{"agents":[]}});
        let rows = client.call("sessions.snapshots", json!({})).await.unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2);
        client.close();
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
        engine.shutdown().await.unwrap();
    }
}
