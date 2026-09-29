use serde_json::{Value, json};
use std::sync::Arc;
use workspacer_hub::services::{analytics::Analytics, pricing::Pricing};
#[test]
fn persistent_headless_analytics_retains_deduplicated_model_splits_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".claude/projects");
    let project = root.join("fixture");
    std::fs::create_dir_all(&project).unwrap();
    let main = project.join("claude-session.jsonl");
    let sub = project.join("claude-session/subagents");
    std::fs::create_dir_all(&sub).unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/analytics-history-cases.json"
    ))
    .unwrap();
    let case = &fixture["cases"][0];
    std::fs::write(&main, case["mainJsonl"].as_str().unwrap()).unwrap();
    std::fs::write(
        sub.join("agent-sub.jsonl"),
        case["subagentJsonl"].as_str().unwrap(),
    )
    .unwrap();
    let pricing = Arc::new(Pricing::new(dir.path().into()));
    let path = dir.path().join("headless-analytics.sqlite");
    let rows: Vec<Value> = case["snapshots"]
        .as_object()
        .unwrap()
        .values()
        .map(|r| {
            let mut r = r.clone();
            r["cwd"] = dir.path().to_string_lossy().to_string().into();
            if r["session_id"] == "claude-session" {
                r["transcript_path"] = main.to_string_lossy().to_string().into();
            }
            r
        })
        .collect();
    let db = Analytics::open(path.clone(), pricing.clone()).unwrap();
    let now = 1_790_553_600_000;
    let first = db
        .observe_and_read("analytics.summary", &json!({}), &rows, &[root.clone()], now)
        .unwrap();
    assert_eq!(first["totals"]["sessions"], 3);
    assert_eq!(first["totals"]["unrecordedSessions"], 1);
    assert_eq!(first["totals"]["inputTokens"], 3500);
    assert_eq!(first["totals"]["outputTokens"], 350);
    assert_eq!(first["byModel"].as_array().unwrap().len(), 4);
    let filtered = db
        .read("analytics.summary", &json!({"provider":"claude"}))
        .unwrap();
    assert_eq!(filtered["totals"]["inputTokens"], 1500);
    assert_eq!(filtered["byProvider"].as_array().unwrap().len(), 2);
    let recent = db
        .read("analytics.recent", &json!({"provider":"codex","limit":1}))
        .unwrap();
    assert_eq!(recent[0]["sessionId"], "codex-session");
    assert!(
        db.read("analytics.summary", &json!({"since":"not a date"}))
            .is_err()
    );
    drop(db);
    std::fs::remove_file(&main).unwrap();
    let db = Analytics::open(path, pricing).unwrap();
    assert_eq!(
        db.observe_and_read("analytics.summary", &json!({}), &rows, &[root], now)
            .unwrap()["totals"],
        first["totals"]
    );
    assert_eq!(
        db.observe_and_read("analytics.summary", &json!({}), &[], &[], now)
            .unwrap()["totals"],
        first["totals"]
    );
}
#[test]
fn malformed_database_and_outside_transcript_are_errors_not_measured_empty_history() {
    let dir = tempfile::tempdir().unwrap();
    let pricing = Arc::new(Pricing::new(dir.path().into()));
    let path = dir.path().join("broken.sqlite");
    std::fs::write(&path, "not sqlite").unwrap();
    assert!(Analytics::open(path, pricing.clone()).is_err());
    let db = Analytics::open(dir.path().join("history.sqlite"), pricing).unwrap();
    let outside = dir.path().join("outside.jsonl");
    std::fs::write(&outside, "{}").unwrap();
    assert!(
        db.observe_and_read(
            "analytics.summary",
            &json!({}),
            &[json!({"session_id":"one","transcript_path":outside})],
            &[dir.path().join("allowed")],
            0
        )
        .is_err()
    );
    assert_eq!(
        db.read("analytics.summary", &json!({})).unwrap()["totals"]["sessions"],
        0
    );
}
#[test]
fn pricing_roundtrip_and_shared_cost_contract() {
    let dir = tempfile::tempdir().unwrap();
    let pricing = Pricing::new(dir.path().into());
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/model-pricing-cases.json")).unwrap();
    for c in fixture["cases"].as_array().unwrap() {
        let model = c["model"].as_str().unwrap();
        assert_eq!(
            pricing.turn_cost(Some(model), &json!({"input_tokens":1000000})),
            c["input"].as_f64().unwrap()
        );
        assert_eq!(
            pricing.turn_cost(Some(model), &json!({"output_tokens":1000000})),
            c["output"].as_f64().unwrap()
        );
    }
    for c in fixture["cacheMultiplierCases"].as_array().unwrap() {
        let mut usage = json!({"input_tokens":c["inputTokens"],"output_tokens":c["outputTokens"],"cache_creation_input_tokens":c["cacheWriteTokens"],"cache_read_input_tokens":c["cacheReadTokens"]});
        for (from, to) in [
            ("ephemeral5m", "ephemeral_5m_input_tokens"),
            ("ephemeral1h", "ephemeral_1h_input_tokens"),
        ] {
            if let Some(value) = c.get(from) {
                usage["cache_creation"][to] = value.clone();
            }
        }
        let actual = pricing.turn_cost(c["model"].as_str(), &usage);
        assert!(
            (actual - c["expectedUSD"].as_f64().unwrap()).abs() < 1e-10,
            "{c}"
        );
    }
    pricing.save(&json!({"fixture-model":{"input":1,"output":2,"cached_input":0.3,"context_limit":1234}})).unwrap();
    assert_eq!(pricing.get()["overrides"]["fixture-model"]["output"], 2);
    assert!(
        pricing
            .save(&json!({"bad":{"input":-1,"output":2}}))
            .is_err()
    );
    assert!(pricing.save(&json!([])).is_err());
    assert_eq!(
        pricing.turn_cost(Some("fixture-model"), &json!({"input_tokens":1000000})),
        1.
    );
    pricing.save(&json!({})).unwrap();
    assert!(!pricing.path.exists());
}

#[tokio::test]
async fn owned_watcher_records_terminal_hooks_without_an_analytics_query_and_joins() {
    use claudemon::daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    };
    use workspacer_hub::{
        Hub, Options,
        services::{analytics::Watcher, profiles::Profiles},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("engine.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    let ready = engine.ready().await.unwrap();
    let service = Arc::new(
        Analytics::open(
            dir.path().join("history.sqlite"),
            Arc::new(Pricing::new(dir.path().into())),
        )
        .unwrap(),
    );
    let watcher = Watcher::new(
        service.clone(),
        Some(engine.client()),
        Arc::new(Profiles::new(dir.path().join("config"))),
        dir.path().into(),
    );
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let running = watcher.clone();
    let handle = hub.handle();
    let task = tokio::spawn(async move { running.run(handle).await });
    let http = reqwest::Client::new();
    for event in ["SessionStart", "SessionEnd"] {
        http.post(format!("http://{}/hook", ready.hook_addr))
            .json(&json!({"event":event,"session_id":"analytics-owned","cwd":dir.path()}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let observed = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let rows = service.read("analytics.recent", &json!({})).unwrap();
            if rows
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["sessionId"] == "analytics-owned" && r["status"] == "ended")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    watcher.close();
    task.await.unwrap().unwrap();
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
    engine.shutdown().await.unwrap();
    observed.expect("owned observer failed to persist terminal evidence");
    assert_eq!(
        service.read("analytics.summary", &json!({})).unwrap()["totals"]["unrecordedSessions"],
        1
    );
    let unavailable = Watcher::new(
        service.clone(),
        None,
        Arc::new(Profiles::new(dir.path().join("config"))),
        dir.path().into(),
    );
    assert!(
        unavailable
            .query("analytics.summary", json!({}))
            .await
            .is_err()
    );
    assert_eq!(
        service.read("analytics.summary", &json!({})).unwrap()["totals"]["sessions"],
        1
    );
}

#[test]
fn recomputation_replaces_old_model_slices_and_sidechains_do_not_replace_main_model() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("projects");
    let project = root.join("one");
    std::fs::create_dir_all(&project).unwrap();
    let main = project.join("session.jsonl");
    let sub = project.join("session/subagents");
    std::fs::create_dir_all(&sub).unwrap();
    let a = json!({"type":"assistant","uuid":"fallback-id","message":{"model":"claude-sonnet","usage":{"input_tokens":100,"output_tokens":10}}});
    let b = json!({"type":"assistant","message":{"id":"side","model":"claude-haiku","usage":{"input_tokens":2000,"output_tokens":20}}});
    std::fs::write(&main, a.to_string()).unwrap();
    std::fs::write(sub.join("agent.jsonl"), format!("{a}\n{b}")).unwrap();
    let service = Analytics::open(
        dir.path().join("history.sqlite"),
        Arc::new(Pricing::new(dir.path().into())),
    )
    .unwrap();
    let rows = vec![
        json!({"session_id":"session","provider":"claude","mode":"stopped","transcript_path":main}),
    ];
    let recent = service
        .observe_and_read("analytics.recent", &json!({}), &rows, &[root.clone()], 1000)
        .unwrap();
    assert_eq!(recent[0]["model"], "claude-sonnet");
    assert_eq!(recent[0]["peakContext"], 100);
    assert_eq!(recent[0]["inputTokens"], 2100);
    std::fs::remove_file(sub.join("agent.jsonl")).unwrap();
    std::fs::write(&main,json!({"type":"assistant","message":{"id":"new","model":"claude-opus","usage":{"input_tokens":50,"output_tokens":5}}}).to_string()).unwrap();
    let summary = service
        .observe_and_read("analytics.summary", &json!({}), &rows, &[root], 2000)
        .unwrap();
    assert_eq!(summary["byModel"].as_array().unwrap().len(), 1);
    assert_eq!(summary["byModel"][0]["key"], "claude-opus");
    assert_eq!(summary["totals"]["inputTokens"], 50);
}
