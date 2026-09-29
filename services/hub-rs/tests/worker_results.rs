use serde_json::{Value, json};
use workspacer_hub::services::worker_results::*;

#[test]
fn shared_worker_result_contracts() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/worker-result-cases.json")).unwrap();
    assert!(corpus["validationCases"].as_array().unwrap().len() >= 12);
    for case in corpus["validationCases"].as_array().unwrap() {
        assert_eq!(
            json!(validate(&case["value"], &case["schema"])),
            case["errors"],
            "{}",
            case["name"]
        );
    }
    assert!(corpus["resultCases"].as_array().unwrap().len() >= 6);
    for case in corpus["resultCases"].as_array().unwrap() {
        let result = read_result(case["message"].as_str().unwrap(), &case["schema"]);
        if let Some(expected) = case["errorContains"].as_str() {
            assert!(
                result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.contains(expected)),
                "{}: {:?}",
                case["name"],
                result
            );
        } else {
            assert_eq!(
                serde_json::from_str::<Value>(result.json.as_deref().unwrap()).unwrap(),
                case["value"],
                "{}",
                case["name"]
            );
        }
    }
    assert!(corpus["escalationCases"].as_array().unwrap().len() >= 6);
    for case in corpus["escalationCases"].as_array().unwrap() {
        let result = read_escalation(case["message"].as_str().unwrap());
        if case["absent"] == true {
            assert!(result.is_none(), "{}", case["name"]);
            continue;
        }
        let result = result.unwrap();
        if let Some(expected) = case["errorContains"].as_str() {
            assert!(
                result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.contains(expected)),
                "{}: {:?}",
                case["name"],
                result
            );
        } else {
            assert_eq!(result.value.unwrap(), case["value"], "{}", case["name"]);
        }
    }
}

#[test]
fn schema_and_report_caps_are_explicit_and_preserve_prose_contract() {
    assert!(check_schema(&json!([])).is_err());
    assert!(check_schema(&json!({"description":"x".repeat(4096)})).is_err());
    assert!(check_schema(&json!({"description":"😀".repeat(2039)})).is_ok());
    assert!(check_schema(&json!({"description":"😀".repeat(2040)})).is_err());
    let contract = result_contract(&json!({"type":"object"})).unwrap();
    assert!(contract.contains("summary first") && contract.contains("FINAL message"));
    let result = read_result(
        &format!("```wks-result\n{}\n```", json!({"text":"x".repeat(10000)})),
        &json!({}),
    );
    assert!(result.json.unwrap().contains("[truncated:"));
    assert!(result.error.is_none());
    assert_eq!(
        validate(
            &json!({}),
            &json!({"required":(0..20).map(|n|format!("field{n}")).collect::<Vec<_>>()})
        )
        .len(),
        8
    );
    assert!(
        read_escalation(&format!("```wks-escalation\n{}\n```", " ".repeat(4097)))
            .unwrap()
            .error
            .unwrap()
            .contains("limit is 4096")
    );
}
