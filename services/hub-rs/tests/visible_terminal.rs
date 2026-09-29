//! Bound real owner shutdown probes so a regression cannot hang the test runner.
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn visible_terminal_request_refuses_a_stopped_bus() {
    if std::env::var("WKS_VISIBLE_STOPPED_BUS_CHILD").as_deref() == Ok("1") {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(check_stopped_bus());
        drop(runtime);
        eprintln!("VISIBLE_STOPPED_BUS_COMPLETE");
        return;
    }
    // Immediate shutdown races observer startup; fresh processes keep each
    // attempt independent and exercise both favorable and late scheduling.
    for attempt in 0..8 {
        run_child(attempt);
    }
}

fn run_child(attempt: usize) {
    let scratch = tempfile::tempdir().unwrap();
    let log_path = scratch.path().join("child.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "visible_terminal_request_refuses_a_stopped_bus",
            "--nocapture",
        ])
        .env("WKS_VISIBLE_STOPPED_BUS_CHILD", "1")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let log = std::fs::read_to_string(&log_path).unwrap();
            assert!(
                status.success(),
                "stopped-bus child {attempt} failed: {log}"
            );
            assert!(
                log.contains("VISIBLE_STOPPED_BUS_COMPLETE"),
                "child {attempt} did not execute its fixture: {log}"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "stopped-bus child {attempt} exceeded deadline: {}",
                std::fs::read_to_string(&log_path).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

async fn check_stopped_bus() {
    use claudemon::daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    };
    use serde_json::json;
    use workspacer_hub::{Caller, Hub, Options, services::terminals::Terminals};
    let root = tempfile::tempdir().unwrap();
    eprintln!("stage: engine start");
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("state.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    eprintln!("stage: hub start");
    let mut options = Options::default();
    options.home_dir = Some(root.path().into());
    options.config_dir = Some(root.path().join("config"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let service = Terminals::new(engine.client(), hub.handle(), root.path().into());
    let caller = Caller {
        call_id: 1,
        activity_seq: 1,
        federated: false,
        connection_id: 1,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: String::new(),
    };
    assert_eq!(
        service
            .call(caller.clone(), "terminals.open", json!({}))
            .await
            .unwrap(),
        json!({"ok":true})
    );
    assert!(service.owned_shell_ids().is_empty());
    eprintln!("stage: hub shutdown");
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
    eprintln!("stage: stopped publication");
    let error = service
        .call(caller, "terminals.open", json!({}))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("hub stopped before publishing event"),
        "{error}"
    );
    eprintln!("stage: terminal service close");
    service.close().await;
    drop(service);
    eprintln!("stage: engine shutdown");
    engine.shutdown().await.unwrap();
}
