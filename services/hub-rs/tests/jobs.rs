use chrono::{DateTime, FixedOffset, Utc};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{
    Caller, Hub, Options,
    client::Client,
    services::jobs::{Job, Service, Trigger, empty_output, fill_prompt, next_run, validate},
};

fn owner() -> Caller {
    Caller {
        call_id: 0,
        activity_seq:0,
        federated: false,
        connection_id: 1,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: "owner".into(),
    }
}
fn spec() -> Value {
    json!({"name":"fixture","enabled":true,"trigger":{"kind":"manual"},"action":{"kind":"call","call":{"method":"fixture.job","params":{"value":42}}}})
}
fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn shared_job_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/hub-job-cases.json")).unwrap();
    assert!(fixture["scheduleCases"].as_array().unwrap().len() >= 5);
    assert!(fixture["promptCases"].as_array().unwrap().len() >= 4);
    for case in fixture["scheduleCases"].as_array().unwrap() {
        let trigger: Trigger = serde_json::from_value(case["trigger"].clone()).unwrap();
        let after = instant(case["after"].as_str().unwrap());
        let zone = FixedOffset::east_opt(case["offsetSeconds"].as_i64().unwrap() as i32).unwrap();
        assert_eq!(
            next_run(&trigger, after, &zone),
            case["expected"].as_str().map(instant),
            "{}",
            case["name"]
        );
    }
    for case in fixture["promptCases"].as_array().unwrap() {
        let outputs: Vec<String> = serde_json::from_value(case["outputs"].clone()).unwrap();
        assert_eq!(
            fill_prompt(case["prompt"].as_str().unwrap(), &outputs),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn scheduling_and_context_rules_match_the_existing_job_contract() {
    let after = instant("2026-09-28T10:00:00Z");
    let zone = FixedOffset::east_opt(2 * 3600).unwrap();
    assert_eq!(
        next_run(
            &Trigger {
                kind: "interval".into(),
                every_minutes: 5,
                ..Default::default()
            },
            after,
            &zone
        ),
        Some(instant("2026-09-28T10:05:00Z"))
    );
    assert_eq!(
        next_run(
            &Trigger {
                kind: "daily".into(),
                at: "12:00".into(),
                ..Default::default()
            },
            after,
            &zone
        ),
        Some(instant("2026-09-29T10:00:00Z"))
    );
    assert_eq!(
        next_run(
            &Trigger {
                kind: "daily".into(),
                at: "12:00".into(),
                days: vec![1],
                ..Default::default()
            },
            after,
            &zone
        ),
        Some(instant("2026-10-05T10:00:00Z"))
    );
    assert_eq!(
        next_run(
            &Trigger {
                kind: "once".into(),
                once: "2026-09-27T00:00:00Z".into(),
                ..Default::default()
            },
            after,
            &zone
        ),
        Some(instant("2026-09-27T00:00:00Z"))
    );
    assert!(
        next_run(
            &Trigger {
                kind: "manual".into(),
                ..Default::default()
            },
            after,
            &zone
        )
        .is_none()
    );
    assert_eq!(
        fill_prompt(
            "{{output.1}} / {{output}}",
            &["first".into(), "last".into()]
        ),
        "first / last"
    );
    assert_eq!(
        fill_prompt("prompt", &["out".into()]),
        "prompt\n\n--- context step 1 ---\n```\nout\n```"
    );
    for empty in ["", " {} ", "[]", "null", "\"\""] {
        assert!(empty_output(empty));
    }
    assert!(!empty_output("0"));
    for method in ["jobs.run", "hub:peer/agents.spawn"] {
        let mut value = spec();
        value["action"]["call"]["method"] = json!(method);
        assert!(validate(&serde_json::from_value::<Job>(value).unwrap()).is_err());
    }
    let mut value = spec();
    value["action"] = json!({"kind":"spawn","spawn":{"cwd":"/fixture","prompt":"task","context":[{"kind":"shell","shell":{"command":"echo ok"},"skipIfEmpty":"false"}]}});
    assert!(validate(&serde_json::from_value::<Job>(value).unwrap()).is_err());
}

#[tokio::test]
async fn ambiguous_job_fields_cannot_arm_or_bypass_a_context_guard() {
    let directory = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(directory.path().join("jobs.json"), hub.handle());
    let mut job = spec();
    job["Enabled"] = json!(false);
    assert!(service.call(&owner(), "jobs.upsert", job).is_err());
    let mut job = spec();
    job["action"] = json!({"kind":"spawn","spawn":{"cwd":"/project","prompt":"task","Context":[{"kind":"shell","shell":{"command":"echo no"},"skipIfEmpty":true}]}});
    assert!(service.call(&owner(), "jobs.upsert", job).is_err());
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn proposals_remain_disarmed_and_hand_edits_are_content_checked() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.json");
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _receiver) = Service::open(path.clone(), hub.handle());
    let proposal = service.call(&owner(), "jobs.propose", spec()).unwrap();
    assert_eq!(proposal["enabled"], false);
    assert_eq!(proposal["proposedBy"], "an agent");
    assert!(
        service
            .call(&owner(), "jobs.run", json!({"id":proposal["id"]}))
            .is_err()
    );
    let mut edited = proposal.clone();
    edited["enabled"] = json!(true);
    std::fs::write(&path, json!({"jobs":[edited]}).to_string()).unwrap();
    let listed = service.call(&owner(), "jobs.list", json!({})).unwrap();
    assert!(listed["jobs"][0].get("nextRunAt").is_none());
    assert!(
        service
            .call(&owner(), "jobs.run", json!({"id":proposal["id"]}))
            .is_err()
    );
    std::fs::write(&path, b"{broken").unwrap();
    assert_eq!(
        service.call(&owner(), "jobs.list", json!({})).unwrap()["jobs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    std::fs::write(&path, json!({"jobs":[]}).to_string()).unwrap();
    assert_eq!(
        service.call(&owner(), "jobs.list", json!({})).unwrap(),
        json!({"jobs":[]})
    );
    let mut operator = owner();
    operator.authenticated_host = false;
    assert!(service.call(&operator, "jobs.list", json!({})).is_err());
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn real_job_runner_records_calls_skips_context_vetoes_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.json");
    let mut options =
        Options::default().handler("fixture.job", |_, params| async move { Ok(params) });
    options.jobs_file = Some(path.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let job = client.call("jobs.upsert", spec()).await.unwrap();
    assert_eq!(
        client
            .call("jobs.run", json!({"id":job["id"]}))
            .await
            .unwrap()["started"],
        true
    );
    let history = wait_history(&client, &job["id"]).await;
    assert_eq!(history["runs"][0]["status"], "ok");
    assert_eq!(history["runs"][0]["detail"], "{\"value\":42}");
    let veto = json!({"name":"no model should start","enabled":false,"trigger":{"kind":"manual"},"action":{"kind":"spawn","spawn":{"cwd":directory.path(),"prompt":"task {{output}}","context":[{"kind":"call","call":{"method":"fixture.job","params":null},"skipIfEmpty":true}]}}});
    let veto = client.call("jobs.upsert", veto).await.unwrap();
    client
        .call("jobs.run", json!({"id":veto["id"]}))
        .await
        .unwrap();
    assert_eq!(
        wait_history(&client, &veto["id"]).await["runs"][0]["status"],
        "skipped"
    );
    hub.shutdown().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(path, hub.handle());
    assert_eq!(
        service
            .call(&owner(), "jobs.history", json!({"id":job["id"]}))
            .unwrap()["runs"][0]["status"],
        "ok"
    );
    hub.shutdown().unwrap();
}
async fn wait_history(client: &Client, id: &Value) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let history = client.call("jobs.history", json!({"id":id})).await.unwrap();
            if !history["runs"].as_array().unwrap().is_empty() {
                return history;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn overlapping_manual_runs_are_not_queued_and_scoped_operators_are_not_owners() {
    let directory = tempfile::tempdir().unwrap();
    let token_path = directory.path().join("tokens.json");
    let record =
        workspacer_hub::auth::mint(&token_path, workspacer_hub::auth::Scope::Operator, "remote")
            .unwrap();
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let started = entered.clone();
    let resume = release.clone();
    let mut options = Options::default().handler("fixture.job", move |_, _| {
        let started = started.clone();
        let resume = resume.clone();
        async move {
            started.notify_one();
            resume.notified().await;
            Ok(json!({"done":true}))
        }
    });
    options.jobs_file = Some(directory.path().join("jobs.json"));
    options.scoped_tokens = Some(token_path);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let remote = Client::from_connection(
        hub.handle()
            .connect_authenticated(record.token, false)
            .await
            .unwrap(),
    );
    assert!(remote.call("jobs.list", json!({})).await.is_err());
    let job = client.call("jobs.upsert", spec()).await.unwrap();
    client
        .call("jobs.run", json!({"id":job["id"]}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        client
            .call("jobs.run", json!({"id":job["id"]}))
            .await
            .unwrap(),
        json!({"started":false,"reason":"already running"})
    );
    release.notify_one();
    assert_eq!(
        wait_history(&client, &job["id"]).await["runs"][0]["status"],
        "ok"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn shell_context_can_ignore_an_exit_code_and_veto_without_starting_a_model() {
    let directory = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.jobs_file = Some(directory.path().join("jobs.json"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let job=client.call("jobs.upsert",json!({"name":"context veto","enabled":false,"trigger":{"kind":"manual"},"action":{"kind":"spawn","spawn":{"cwd":directory.path(),"prompt":"Never spawn","context":[{"kind":"shell","shell":{"command":"exit 1"},"ignoreExitCode":true,"skipIfEmpty":true}]}}})).await.unwrap();
    client
        .call("jobs.run", json!({"id":job["id"]}))
        .await
        .unwrap();
    let history = wait_history(&client, &job["id"]).await;
    assert_eq!(history["runs"][0]["status"], "skipped");
    assert!(
        history["runs"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("no output")
    );
    hub.shutdown().unwrap();
}
