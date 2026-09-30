#[path = "support/sweepguard.rs"]
mod sweepguard;
use serde_json::{Value, json};
use workspacer_hub::services::snapshots::{compat, layout_ids, live, visible};

#[test]
fn missing_layout_and_explicitly_empty_curation_are_distinct() {
    for value in [
        Value::Null,
        json!({}),
        json!({"data":null}),
        json!({"data":{}}),
        json!({"data":{"agents":null}}),
        json!({"data":{"agents":{}}}),
        json!({"data":{"agents":[{"sessionId":17}]}}),
        json!({"data":{"agents":[{"global":true,"tabs":false}]}}),
        json!({"data":{"agents":[{"tabs":[{"panes":[{"attachSessionId":false}]}]}]}}),
    ] {
        assert_eq!(layout_ids(&value), (Default::default(), false), "{value}");
    }
    assert_eq!(
        layout_ids(&json!({"version":1,"data":{"agents":[]}})),
        (Default::default(), true)
    );
    let layout = json!({"version":7,"data":{"agents":[{"global":true,"lastSessionId":"global-last"},{"sessionId":"live-1","tabs":[{"panes":[{"attachSessionId":"attached-1"}]}]},{"lastSessionId":"stopped-2"},null]}});
    assert_eq!(
        layout_ids(&layout),
        (
            ["live-1".into(), "attached-1".into(), "stopped-2".into()].into(),
            true
        )
    );
    let now = time::OffsetDateTime::parse(
        "2026-07-10T12:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let stopped =
        json!({"session_id":"recent","mode":"stopped","updated_at":"2026-07-10T11:00:00Z"});
    assert!(visible(&stopped, &json!({"data":{}}), now));
    assert!(!visible(&stopped, &json!({"data":{"agents":[]}}), now));
}

#[test]
fn legacy_visibility_and_process_liveness_vectors_remain_separate() {
    let now = time::OffsetDateTime::parse(
        "2026-07-10T12:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let layout = json!({"data":{"agents":[{"sessionId":"cur"}]}});
    for (mode, id, date, archived, has_layout, expected) in [
        ("input", "a", "2026-07-10T11:00:00Z", false, false, true),
        ("responding", "a", "2026-07-10T11:00:00Z", false, true, true),
        ("approval", "a", "2026-07-08T12:00:00Z", false, true, true),
        ("unknown", "a", "2026-07-10T11:00:00Z", false, true, false),
        ("unknown", "a", "2026-07-10T11:00:00Z", false, false, false),
        ("stopped", "cur", "2026-07-08T12:00:00Z", false, true, true),
        ("stopped", "a", "2026-07-10T11:00:00Z", false, true, false),
        ("stopped", "a", "2026-07-10T11:00:00Z", false, false, true),
        ("stopped", "a", "2026-07-08T12:00:00Z", false, false, false),
        ("stopped", "a", "2026-07-10T11:00:00Z", true, false, false),
        ("stopped", "a", "", false, false, false),
    ] {
        let row = json!({"session_id":id,"mode":mode,"updated_at":date,"archived":archived});
        assert_eq!(
            visible(&row, if has_layout { &layout } else { &Value::Null }, now),
            expected,
            "{row}"
        );
    }
    for (row, expected) in [
        (json!({"cwd":"/w/p","status":"active"}), true),
        (json!({"cwd":"/w/p"}), true),
        (json!({"status":"ended"}), false),
        (json!({"mode":"stopped"}), false),
        (json!({"mode":"running","archived":true}), false),
        (json!({"mode":"unknown"}), false),
        (json!({"mode":"running","status":"ended"}), true),
        (json!({"mode":"running"}), true),
        (json!("broken"), false),
    ] {
        assert_eq!(live(&row), expected, "{row}");
    }
    let curated = json!({"session_id":"cur","mode":"stopped","archived":true});
    assert!(visible(&curated, &layout, now));
    assert!(!live(&curated));
    for field in ["session_id", "mode", "status", "updated_at", "archived"] {
        let mut row = json!({"session_id":"s","mode":"input"});
        row[field] = json!(17);
        assert!(!visible(&row, &Value::Null, now), "{field}");
    }
    for row in [
        Value::Null,
        json!([]),
        json!({"mode":false}),
        json!({"status":17}),
        json!({"archived":"false"}),
    ] {
        assert!(!live(&row), "{row}");
    }
    assert!(live(
        &json!({"mode":null,"status":"active","archived":null})
    ));
}

#[test]
fn shared_snapshot_projection_matches_go_reference() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/hub-snapshot-cases.json")).unwrap();
    assert!(fixture["vocabulary"]["blocks"]["cases"]["loaders"].as_array().unwrap().iter().any(|v| v == "services/hub-rs/tests/snapshots.rs::shared_snapshot_projection_matches_go_reference"));
    let mut tally = sweepguard::Tally::default();
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            compat(case["raw"].clone()),
            case["expected"],
            "{}",
            case["name"]
        );
        tally.ran("other");
    }
    tally
        .require_every("shared snapshot projection", 4)
        .unwrap();
}
#[test]
fn visibility_distinguishes_live_stopped_curated_and_unknown_sessions() {
    let now = time::OffsetDateTime::parse(
        "2026-09-28T00:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let layout = json!({"version":1,"data":{"agents":[{"sessionId":"curated"},{"global":true,"sessionId":"overview"},{"tabs":[{"panes":[{"attachSessionId":"pane"}]}]}]}});
    assert_eq!(
        layout_ids(&layout).0,
        ["curated".into(), "pane".into()].into()
    );
    let row = json!({"session_id":"old","mode":"input","updated_at":"2020-01-01T00:00:00Z"});
    assert!(visible(&row, &layout, now));
    assert!(!visible(
        &json!({"session_id":"curated","mode":"unknown"}),
        &layout,
        now
    ));
    let recent =
        json!({"session_id":"recent","mode":"stopped","updated_at":"2026-09-27T23:00:00Z"});
    assert!(visible(&recent, &Value::Null, now));
    assert!(!visible(&recent, &layout, now));
    assert!(visible(
        &json!({"session_id":"curated","mode":"stopped","archived":true}),
        &layout,
        now
    ));
}

#[test]
fn cwd_display_names_are_fallbacks_not_agent_identity_or_remote_metadata() {
    use workspacer_hub::services::snapshots::with_cwd_name;
    let names = json!({"/project":"Renamed","/empty":""});
    assert_eq!(
        with_cwd_name(json!({"sessionId":"one","cwd":"/project"}), &names)["label"],
        "Renamed"
    );
    assert_eq!(
        with_cwd_name(
            json!({"sessionId":"one","cwd":"/project","label":"Spawned"}),
            &names
        )["label"],
        "Spawned"
    );
    for row in [
        json!({"cwd":"/missing"}),
        json!({"cwd":"/empty"}),
        json!({"cwd":"/project","hub":"peer"}),
    ] {
        assert!(with_cwd_name(row, &names).get("label").is_none());
    }
    assert_eq!(
        with_cwd_name(json!({"cwd":"/project","label":null}), &names)["label"],
        Value::Null
    );
    assert!(
        with_cwd_name(json!({"cwd":"/project"}), &Value::Null)
            .get("label")
            .is_none()
    );
}
#[test]
fn retained_projection_preserves_cache_and_does_not_invent_idle_or_missing_measurements() {
    let row = compat(
        json!({"session_id":"s1","usage":{"model":"claude-opus-5","context_tokens":40984,"context_limit":200000,"cost_usd":1.5,"cache":{"fresh":2,"write":23393,"read":17589}}}),
    );
    assert_eq!(
        row["usage"]["cache"],
        json!({"fresh":2,"write":23393,"read":17589})
    );
    assert_eq!(row["usage"]["contextTokens"], 40984);
    let row =
        compat(json!({"session_id":"s1","usage":{"model":"gpt-5-codex","context_tokens":10}}));
    assert!(row["usage"].get("cache").is_none());
    assert!(row["usage"].get("contextLimit").is_none());
    let row = compat(
        json!({"session_id":"s1","status_line":{"model_display":"gpt-5-codex","total_input_tokens":4402946,"cached_input_tokens":3733376,"context_usage_state":"waiting_for_runtime_usage"}}),
    );
    assert_eq!(row["statusLine"]["cachedInputTokens"], 3733376);
    assert_eq!(
        row["statusLine"]["contextUsageState"],
        "waitingForRuntimeUsage"
    );
    for mode in ["unknown", "", "future-daemon-mode"] {
        let row = compat(json!({"session_id":"s1","mode":mode}));
        assert_eq!(row["status"], "active");
        assert!(row.get("ambientState").is_none());
    }
    for (mode, expected) in [
        ("responding", "streaming"),
        ("approval", "waiting_approval"),
        ("question", "waiting_input"),
        ("input", "idle"),
        ("stopped", "idle"),
    ] {
        assert_eq!(
            compat(json!({"session_id":"s1","mode":mode}))["ambientState"],
            expected
        );
        let row = compat(json!({"session_id":"s1","mode":mode,"background_tasks":3}));
        assert_eq!(
            row["ambientState"],
            if mode == "input" {
                "background"
            } else {
                expected
            }
        );
        assert_eq!(row["backgroundTasks"], 3);
    }
    for count in [Value::Null, json!(0), json!(-1)] {
        let row = compat(json!({"session_id":"s1","mode":"input","background_tasks":count}));
        assert_eq!(row["ambientState"], "idle");
        assert!(row.get("backgroundTasks").is_none());
    }
}
#[test]
fn approval_projection_unwraps_current_legacy_and_unknown_payloads() {
    for (raw, expected) in [
        (
            json!({"hook_event_name":"PermissionRequest","tool_input":{"command":"ls -la"}}),
            json!({"command":"ls -la"}),
        ),
        (
            json!({"input":{"command":"old shape"}}),
            json!({"command":"old shape"}),
        ),
        (
            json!({"detail":"no tool_input here"}),
            json!({"detail":"no tool_input here"}),
        ),
    ] {
        let row = compat(
            json!({"session_id":"s1","pending":{"kind":"approval","tool":"Bash","raw":raw}}),
        );
        assert_eq!(
            row["pendingApproval"],
            json!({"toolName":"Bash","toolInput":expected})
        );
        assert_eq!(row["pendingQuestions"], Value::Null);
    }
}

#[test]
fn owner_selection_remains_sparse_and_preserves_contradictory_provider_evidence() {
    let old = compat(
        json!({"session_id":"old","mode":"input","usage":{"model":"opus","context_tokens":10}}),
    );
    for key in [
        "requestedSelection",
        "requested_selection",
        "resolvedContextWindow",
        "resolved_context_window",
    ] {
        assert!(old.get(key).is_none());
    }
    for (selection, resolved) in [
        (
            Some(json!({"model":"opus","context_window":1000000})),
            Some(json!(1000000)),
        ),
        (
            Some(json!({"model":"opus","context_window":null})),
            Some(json!(200000)),
        ),
        (
            Some(json!({"model":"sonnet","context_window":1000000})),
            None,
        ),
        (None, Some(json!(200000))),
        (None, Some(Value::Null)),
    ] {
        let mut raw = json!({"session_id":"s","mode":"input","requested_model":"opus[1m]"});
        if let Some(value) = &selection {
            raw["requested_selection"] = value.clone();
        }
        if let Some(value) = &resolved {
            raw["resolved_context_window"] = value.clone();
        }
        let row = compat(raw.clone());
        assert_eq!(
            row.get("requested_selection"),
            raw.get("requested_selection")
        );
        assert_eq!(
            row.get("resolved_context_window"),
            raw.get("resolved_context_window")
        );
        assert_eq!(row["requested_model"], "opus[1m]");
        if let Some(selection) = selection {
            assert_eq!(
                row["requestedSelection"],
                json!({"model":selection["model"],"contextWindow":selection["context_window"]})
            );
        } else {
            assert!(row.get("requestedSelection").is_none());
        }
        if let Some(value) = resolved.filter(|value| !value.is_null()) {
            assert_eq!(row["resolvedContextWindow"], value);
        } else {
            assert!(row.get("resolvedContextWindow").is_none());
        }
    }
    let raw = json!({"session_id":"s","requested_selection":{"model":"opus","context_window":1000000},"resolved_context_window":1000000,"usage":{"model":"claude-opus-5","context_tokens":356380,"context_limit":1000000},"status_line":{"model_display":"Opus","context_used_pct":178.19,"context_window_size":200000}});
    let row = compat(raw.clone());
    assert_eq!(row["requestedSelection"]["contextWindow"], 1000000);
    assert_eq!(row["resolvedContextWindow"], 1000000);
    assert_eq!(row["usage"]["contextLimit"], 1000000);
    assert_eq!(row["usage"]["contextTokens"], 356380);
    assert_eq!(row["statusLine"]["contextWindowSize"], 200000);
    assert_eq!(row["statusLine"]["contextUsedPct"], 178.19);
    assert_eq!(row["status_line"], raw["status_line"]);
}

#[test]
fn execution_engine_projection_keeps_exact_owner_metadata_without_inventing_support() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/execution-engine-v1.json")).unwrap();
    for readiness in ["ready", "unavailable"] {
        let mut metadata = fixture["metadata"].clone();
        metadata["readiness"] = json!(readiness);
        let row =
            compat(json!({"session_id":"engine","mode":"stopped","execution_engine":metadata}));
        assert_eq!(row["executionEngine"], metadata);
        assert_eq!(row["execution_engine"], metadata);
    }
    assert!(
        compat(json!({"session_id":"old","mode":"input"}))
            .get("executionEngine")
            .is_none()
    );
}

#[test]
fn shipping_mobile_reads_are_backed_by_production_snapshot_and_metadata_projections() {
    let mobile = include_str!("../assets/web/mobile.html");
    let row = compat(json!({
        "session_id":"s1","mode":"responding","cwd":"/tmp","provider":"claude",
        "transport":"stream","archived":false,"updated_at":"2026-07-10T12:00:00Z",
        "usage":{"model":"m","context_tokens":1,"context_limit":2,"cost_usd":0.1},
        "tool_calls":7,"pending":null,
        "status_line":{"model_display":"Opus","context_used_pct":12.5,"context_window_size":200000,
            "total_input_tokens":100,"total_output_tokens":50,"cost_usd":0.2,
            "five_hour_pct":10,"five_hour_resets_at":1234,"five_hour_window_minutes":300,
            "seven_day_pct":20,"seven_day_resets_at":5678,"seven_day_window_minutes":10080,
            "monthly_pct":30,"monthly_resets_at":9012,"rate_limit_warning":"careful",
            "received_at":"2026-07-10T12:00:00Z"}
    }));
    let required = [
        "sessionId",
        "status",
        "ambientState",
        "lastActivity",
        "cwd",
        "transport",
        "provider",
        "usage",
        "pendingApproval",
        "pendingQuestions",
        "statusLine",
        "totalToolCalls",
    ];
    assert_eq!(
        required.len(),
        12,
        "retained required-field coverage changed"
    );
    for field in required {
        assert!(
            mobile.contains(field),
            "shipping mobile no longer reads {field}; review the field contract"
        );
        assert!(
            row.get(field).is_some(),
            "production projection omitted {field}"
        );
    }
    for (field, reason) in [
        (
            "conversation",
            "transcripts are fetched through sessions.conversation rather than repeated in every snapshot",
        ),
        (
            "liveCwd",
            "desktop-only live cwd remains optional; mobile falls back to cwd",
        ),
    ] {
        assert!(!reason.is_empty());
        assert!(mobile.contains(field), "stale declined field {field}");
        assert!(
            row.get(field).is_none(),
            "review newly projected field {field}"
        );
    }
    for (field, expected) in [
        ("sessionId", json!("s1")),
        ("status", json!("active")),
        ("ambientState", json!("streaming")),
        ("lastActivity", json!(1783684800000_i64)),
        ("cwd", json!("/tmp")),
        ("transport", json!("stream")),
        ("provider", json!("claude")),
        ("totalToolCalls", json!(7)),
        ("pendingApproval", Value::Null),
        ("pendingQuestions", Value::Null),
    ] {
        assert_eq!(row[field], expected, "{field}");
    }
    let usage = json!({"model":"m","contextTokens":1,"contextLimit":2,"costUSD":0.1});
    assert_eq!(usage.as_object().unwrap().len(), 4);
    for (field, expected) in usage.as_object().unwrap() {
        assert!(
            mobile.contains(field),
            "mobile usage field removed: {field}"
        );
        assert_eq!(row["usage"][field], *expected, "usage.{field}");
    }
    let status = json!({"modelDisplay":"Opus","contextUsedPct":12.5,"contextWindowSize":200000,
        "totalInputTokens":100,"totalOutputTokens":50,"costUSD":0.2,
        "fiveHourPct":10,"fiveHourResetsAt":1234,"fiveHourWindowMins":300,
        "sevenDayPct":20,"sevenDayResetsAt":5678,"sevenDayWindowMins":10080,
        "monthlyPct":30,"monthlyResetsAt":9012,"monthlyWindowMins":null,
        "rateLimitWarning":"careful","receivedAt":"2026-07-10T12:00:00Z"});
    assert_eq!(status.as_object().unwrap().len(), 17);
    for (field, expected) in status.as_object().unwrap() {
        assert!(
            mobile.contains(field),
            "mobile status field removed: {field}"
        );
        assert_eq!(
            row["statusLine"].get(field),
            Some(expected),
            "statusLine.{field}"
        );
    }
    // Use the actual durable metadata owner that with_host_metadata invokes.
    // A literal enriched JSON fixture would not detect a broken production overlay.
    use workspacer_hub::services::manager_replacements::ReplacementState;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("manager-replacements.json");
    let metadata = ReplacementState::open(path.clone()).unwrap();
    metadata
        .remember_child(json!({"sessionId":"s1","cwd":"/tmp","label":"proj: a task",
        "parentSessionId":"mgr","isWakeTarget":true}))
        .unwrap();
    drop(metadata);
    let metadata = ReplacementState::open(path).unwrap();
    let enriched = metadata.enrich(row.clone());
    for (field, expected) in [
        ("parentSessionId", json!("mgr")),
        ("isWakeTarget", json!(true)),
        ("label", json!("proj: a task")),
    ] {
        assert!(
            mobile.contains(field),
            "mobile nesting field removed: {field}"
        );
        assert_eq!(enriched[field], expected, "nesting.{field}");
        assert!(
            row.get(field).is_none(),
            "fixture must require actual enrichment"
        );
    }
    let unknown = json!({"session_id":"unknown","cwd":"/tmp"});
    assert_eq!(metadata.enrich(unknown.clone()), unknown);
    let remote = json!({"session_id":"s1","cwd":"/remote","hub":"peer"});
    assert_eq!(metadata.enrich(remote.clone()), remote);
}
