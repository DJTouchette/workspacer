use serde_json::Value;
use workspacer_hub::services::fleet_messages;
#[test]
fn shared_fleet_message_contracts() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/fleet-message-cases.json")).unwrap();
    assert!(corpus["cases"].as_array().unwrap().len() >= 18);
    for case in corpus["cases"].as_array().unwrap() {
        assert_eq!(
            fleet_messages::build(
                case["kind"].as_str().unwrap(),
                case["entries"].as_array().unwrap(),
                case["audience"] == "ordinary-parent"
            )
            .unwrap(),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
    assert!(corpus["excerpts"].as_array().unwrap().len() >= 3);
    for case in corpus["excerpts"].as_array().unwrap() {
        assert_eq!(
            fleet_messages::excerpt(case["input"].as_str().unwrap()),
            case["expected"].as_str().unwrap()
        );
    }
}

#[test]
fn labelled_and_unlabelled_sender_headers_match_retained_wire_text() {
    assert_eq!(
        fleet_messages::sender_header("worker1", "Rust Worker"),
        "[fleet] session:worker1 (Rust Worker) says:\n"
    );
    assert_eq!(
        fleet_messages::sender_header("worker2", ""),
        "[fleet] session:worker2 says:\n"
    );
}

#[test]
fn workflow_instructions_follow_full_result_and_are_manager_only() {
    let entries = [
        serde_json::json!({"sessionId":"worker","label":"Worker","cwd":"/project",
        "fullReply":"complete reply","workflowInstructions":"NEXT PINNED STEP"}),
    ];
    let manager = fleet_messages::build("worker-finished", &entries, false).unwrap();
    assert!(manager.find("complete reply").unwrap() < manager.find("NEXT PINNED STEP").unwrap());
    let ordinary = fleet_messages::build("worker-finished", &entries, true).unwrap();
    assert!(ordinary.contains("complete reply"));
    assert!(!ordinary.contains("NEXT PINNED STEP"));
}
