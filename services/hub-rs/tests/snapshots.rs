use serde_json::{Value, json};
use workspacer_hub::services::snapshots::{compat, layout_ids, visible};

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
