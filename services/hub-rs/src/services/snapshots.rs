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

fn parse_time(raw: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).ok()
}

pub fn layout_ids(layout: &Value) -> (BTreeSet<String>, bool) {
    let Some(data) = layout.get("data").filter(|v| v.is_object()) else {
        return (BTreeSet::new(), false);
    };
    let mut ids = BTreeSet::new();
    if let Some(agents) = data["agents"].as_array() {
        for agent in agents {
            if agent["global"] == true {
                continue;
            }
            for key in ["sessionId", "lastSessionId"] {
                if let Some(id) = agent[key].as_str().filter(|s| !s.is_empty()) {
                    ids.insert(id.into());
                }
            }
            if let Some(tabs) = agent["tabs"].as_array() {
                for tab in tabs {
                    if let Some(panes) = tab["panes"].as_array() {
                        for pane in panes {
                            if let Some(id) =
                                pane["attachSessionId"].as_str().filter(|s| !s.is_empty())
                            {
                                ids.insert(id.into());
                            }
                        }
                    }
                }
            }
        }
    }
    (ids, true)
}
pub fn visible(snapshot: &Value, layout: &Value, now: time::OffsetDateTime) -> bool {
    if !snapshot.is_object() || snapshot["mode"] == "unknown" {
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
