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
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            compat(case["raw"].clone()),
            case["expected"],
            "{}",
            case["name"]
        );
    }
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
