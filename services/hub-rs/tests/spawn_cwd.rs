use serde_json::{Value, json};
use workspacer_hub::services::{spawn_plan, terminals::normalize_cwd};
#[path = "support/sweepguard.rs"]
mod sweepguard;

#[test]
fn terminal_cwd_executes_the_entire_raw_normalization_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let home_text = home.path().to_string_lossy();
    let mut tally = sweepguard::Tally::default();
    let mut saw_tilde = false;
    for case in fixture["spawnCwds"]["cases"].as_array().unwrap() {
        let input = case["in"].as_str().unwrap();
        let expected = case["out"].as_str().unwrap().replace("${HOME}", &home_text);
        assert!(!expected.contains("${"), "unknown fixture substitution");
        assert_eq!(
            normalize_cwd(input, home.path()),
            expected,
            "{}",
            case["why"]
        );
        saw_tilde |= input.starts_with('~');
        tally.ran("other");
    }
    assert!(saw_tilde, "no tilde vector executed");
    tally.require_every("raw terminal spawn cwd", 14).unwrap();
}

#[test]
fn spawn_plans_canonicalize_absolute_selection_without_expanding_literal_tildes() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    std::fs::create_dir(&selected).unwrap();
    for input in [None, Some(json!("")), Some(json!(" \t\r\n\u{b}\u{c}"))] {
        let mut params = json!({});
        if let Some(input) = input {
            params["cwd"] = input;
        }
        let plan = spawn_plan::resolve(&params, &json!({}), None, &selected, "new", false).unwrap();
        assert_eq!(plan.request["cwd"], json!(selected.canonicalize().unwrap()));
    }
    for input in ["~", "~/proj", "~root", "\u{85}", "\u{feff}", "\u{a0}/tmp"] {
        assert!(
            spawn_plan::resolve(
                &json!({"cwd":input}),
                &json!({}),
                None,
                &selected,
                "new",
                false
            )
            .is_err(),
            "{input}"
        );
    }
    let literal = selected.join("a~b");
    std::fs::create_dir(&literal).unwrap();
    let plan = spawn_plan::resolve(
        &json!({"cwd":format!("  {}/  ",literal.display())}),
        &json!({}),
        None,
        &selected,
        "new",
        false,
    )
    .unwrap();
    assert_eq!(plan.request["cwd"], json!(literal.canonicalize().unwrap()));
    let missing = selected.join("not-created");
    let plan = spawn_plan::resolve(
        &json!({"cwd":missing}),
        &json!({}),
        None,
        &selected,
        "new",
        false,
    )
    .unwrap();
    assert_eq!(
        plan.request["cwd"],
        json!(selected.canonicalize().unwrap().join("not-created"))
    );
    assert!(!missing.exists());
}
