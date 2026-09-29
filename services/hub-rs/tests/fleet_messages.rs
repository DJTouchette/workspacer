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
