//! Author-facing contract pins; examples run through the production validator.
use super::*;
const DOCS: &str = include_str!("../../../../../landing/docs.html");
fn examples() -> Vec<Job> {
    let pattern = regex::Regex::new(r"(?s)<pre data-job-example><code>(.*?)</code></pre>").unwrap();
    let values: Vec<_> = pattern
        .captures_iter(DOCS)
        .map(|capture| {
            // The documentation's JSON examples use HTML's XML entities. Refuse
            // unfamiliar encodings instead of silently validating different text.
            let value = capture[1]
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&");
            assert!(
                !regex::Regex::new(r"&(?:#[0-9A-Za-z]+|[A-Za-z]+);")
                    .unwrap()
                    .is_match(&value),
                "unhandled HTML entity in job example"
            );
            serde_json::from_str::<Job>(&value).unwrap()
        })
        .collect();
    assert!(values.len() >= 2, "documented job examples disappeared");
    values
}
#[test]
fn documented_and_captured_authored_jobs_pass_the_actual_validator() {
    for example in examples() {
        validate(&example).unwrap_or_else(|error| panic!("{}: {error}", example.name));
    }
    let authored: Value = serde_json::from_str(include_str!("authored_cases.json")).unwrap();
    let cases = authored["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let job: Job = serde_json::from_value(case["spec"].clone()).unwrap();
        validate(&job).unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
    }
}
#[test]
fn disabled_power_down_preset_matches_docs_except_explicit_safety_differences() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../contracts/job-preset-power-down.json"
    ))
    .unwrap();
    let spec: Job = serde_json::from_value(fixture["spec"].clone()).unwrap();
    validate(&spec).unwrap();
    assert!(!spec.enabled);
    let documented = examples()
        .into_iter()
        .find(|job| Some(job.name.as_str()) == fixture["docsExampleName"].as_str())
        .expect("documented preset");
    assert!(
        documented.enabled,
        "the worked example is armed; only the preset is disabled"
    );
    assert_eq!(documented.trigger, spec.trigger);
    assert_eq!(documented.action["kind"], spec.action["kind"]);
    assert_eq!(
        string(&documented.action["shell"], "cwd"),
        string(&spec.action["shell"], "cwd")
    );
    let command = string(&spec.action["shell"], "command");
    let (check, script) = command.split_once(" && ").unwrap();
    assert_eq!(check, fixture["quiescenceCheck"].as_str().unwrap());
    assert_eq!(script, fixture["placeholder"].as_str().unwrap());
    let (docs_check, docs_script) = string(&documented.action["shell"], "command")
        .split_once(" && ")
        .unwrap();
    assert_eq!(docs_check, check);
    assert_ne!(docs_script, script);
    assert!(!docs_script.contains(fixture["placeholder"].as_str().unwrap()));
}
#[test]
fn canonical_spec_fields_and_kinds_remain_documented() {
    // Shared with the actual key admission guard, not a second test-only list.
    for fields in [
        JOB_FIELDS,
        TRIGGER_FIELDS,
        ACTION_FIELDS,
        SPAWN_FIELDS,
        CONTEXT_FIELDS,
        SHELL_FIELDS,
        CALL_FIELDS,
    ] {
        for name in fields {
            if !matches!(*name, "createdAt" | "updatedAt") {
                assert!(DOCS.contains(name), "undocumented job field {name}");
            }
        }
    }
    // Adding a typed Job/Trigger field must update this inventory too; neither
    // struct literal uses a default tail that could hide a new serializable key.
    let trigger = Trigger {
        kind: "daily".into(),
        every_minutes: 1,
        at: "09:00".into(),
        days: vec![1],
        once: "2026-09-01T09:00:00Z".into(),
    };
    let job = Job {
        id: "fixture".into(),
        name: "fixture".into(),
        enabled: true,
        trigger: trigger.clone(),
        action: json!({"kind":"call","call":{"method":"sessions.list"}}),
        proposed_by: "author".into(),
        created_at: 1,
        updated_at: 1,
    };
    for (value, fields) in [(json!(job), JOB_FIELDS), (json!(trigger), TRIGGER_FIELDS)] {
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            fields.iter().copied().collect()
        );
    }
    for trigger in [
        json!({"kind":"manual"}),
        json!({"kind":"interval","everyMinutes":60}),
        json!({"kind":"daily","at":"09:00","days":[1,2,3,4,5]}),
        json!({"kind":"once","once":"2026-09-01T09:00:00Z"}),
    ] {
        for action in [
            json!({"kind":"shell","shell":{"command":"true"}}),
            json!({"kind":"call","call":{"method":"sessions.list"}}),
            json!({"kind":"spawn","spawn":{"cwd":"/fixture","prompt":"{{output}}","context":[{"kind":"shell","shell":{"command":"fixture"},"skipIfEmpty":true,"skipUnlessMatch":"FAIL","ignoreExitCode":true},{"kind":"call","call":{"method":"sessions.list"},"skipIfEmpty":true}]}}),
        ] {
            assert!(DOCS.contains(trigger["kind"].as_str().unwrap()));
            assert!(DOCS.contains(action["kind"].as_str().unwrap()));
            validate(
                &serde_json::from_value::<Job>(
                    json!({"name":"docs","trigger":trigger,"action":action}),
                )
                .unwrap(),
            )
            .unwrap();
        }
    }
}
#[test]
fn hand_editing_and_storage_documentation_keeps_operational_instructions() {
    for phrase in [
        "{{output}}",
        "{{output.1}}",
        "jobs.json",
        "0600",
        "editing jobs.json by hand",
        "re-reads it by itself",
        "30-second tick",
        "nothing restarted",
        "~/.config/workspacer-hub/jobs.json",
        "workspacer jobs list",
        r#""enabled": true"#,
        "does not parse",
        "keeps the schedule it is already running",
        r#"{"jobs": []}"#,
        "re-reads the file first",
        "~/.workspacer/scripts/",
        "it is a convention, not a feature",
        "JSON has no comments",
        "equivalent to writing a crontab",
        "unattended",
        "where everything goes",
        "~/.workspacer/",
        "&lt;project&gt;/.workspacer/",
        "~/.config/workspacer-hub/",
        "per hub",
        "model-rates.json",
        r##"See <a href="#configuration">where everything goes</a>"##,
    ] {
        assert!(DOCS.contains(phrase), "missing author instruction {phrase}");
    }
    assert!(DOCS.contains("four context steps") || DOCS.contains("max 4"));
    let section = DOCS
        .split_once("<h2>jobs</h2>")
        .expect("jobs docs heading")
        .1
        .split("<h2")
        .next()
        .unwrap();
    assert!(
        !section.contains("restart the hub"),
        "job edits do not require a restart"
    );
}
