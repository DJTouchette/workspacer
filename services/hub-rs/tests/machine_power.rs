use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use workspacer_hub::{
    Caller,
    services::{
        machine_power::{Controller, PowerProvider, Timing, idle_inputs},
        quiescence::{
            Evidence, JobInfo, Tunables, evaluate, power::PowerConfig, source::EvidenceSource,
        },
    },
};
fn caller() -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        federated: false,
        connection_id: 1,
        authenticated_host: false,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: "scoped-operator".into(),
    }
}
#[derive(Default)]
struct Source {
    busy: AtomicBool,
    unknown: AtomicBool,
    job: AtomicBool,
}
impl EvidenceSource for Source {
    fn read(&self) -> BoxFuture<'_, Result<Evidence>> {
        Box::pin(async move {
            let now = chrono::Utc::now().timestamp_millis();
            if self.unknown.load(Ordering::SeqCst) {
                return Ok(Evidence::unknown(now));
            }
            Ok(Evidence {
                now_ms: now,
                sessions: Ok(if self.busy.load(Ordering::SeqCst) {
                    json!([{"sessionId":"busy","mode":"responding"}])
                } else {
                    json!([])
                }),
                clients: Ok(vec![]),
                jobs: Ok(if self.job.load(Ordering::SeqCst) {
                    vec![JobInfo {
                        id: "later".into(),
                        name: String::new(),
                        action_kind: "shell".into(),
                        next_run_ms: Some(now + 86_400_000),
                        running: false,
                    }]
                } else {
                    vec![]
                }),
                peers: Ok(vec![]),
                operations: vec![],
            })
        })
    }
}
struct Provider {
    source: Arc<Source>,
    checks: AtomicUsize,
    stops: AtomicUsize,
    fail_check: AtomicBool,
    fail_stop: AtomicBool,
    work_after_check: AtomicBool,
    order: Arc<Mutex<Vec<&'static str>>>,
}
impl PowerProvider for Provider {
    fn check(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async {
            self.checks.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push("check");
            if self.fail_check.load(Ordering::SeqCst) {
                bail!("fixture-secret-provider-error");
            }
            if self.work_after_check.load(Ordering::SeqCst) {
                self.source.busy.store(true, Ordering::SeqCst);
            }
            Ok(())
        })
    }
    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push("stop");
            if self.fail_stop.load(Ordering::SeqCst) {
                bail!("fixture-secret-uncertain");
            }
            Ok(())
        })
    }
}
fn setup(
    automatic: bool,
) -> (
    Arc<Source>,
    Arc<Provider>,
    Arc<Controller>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let source = Arc::new(Source::default());
    let order = Arc::new(Mutex::new(vec![]));
    let provider = Arc::new(Provider {
        source: source.clone(),
        checks: AtomicUsize::new(0),
        stops: AtomicUsize::new(0),
        fail_check: AtomicBool::new(false),
        fail_stop: AtomicBool::new(false),
        work_after_check: AtomicBool::new(false),
        order: order.clone(),
    });
    let controller = Controller::new(
        PowerConfig {
            idle_timeout_ms: if automatic { 30 } else { 0 },
            requested_stop: automatic,
            ..Default::default()
        },
        Some(provider.clone()),
        source.clone(),
        Arc::new(move || {
            let order = order.clone();
            Box::pin(async move {
                order.lock().unwrap().push("disconnect");
                Ok(())
            })
        }),
        Timing {
            sample: Duration::from_millis(5),
            receipt_delay: Duration::from_millis(40),
            check_timeout: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(1),
        },
    );
    let running = controller.clone();
    let task = tokio::spawn(async move { running.run().await });
    (source, provider, controller, task)
}
async fn until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn manual_authority_preflight_idempotence_and_disconnect_order() {
    let (_, provider, controller, task) = setup(false);
    for (scope, trusted) in [
        ("operator", false),
        ("provider", true),
        ("view", true),
        ("triage", true),
    ] {
        let mut c = caller();
        c.scope = scope.into();
        c.trusted = trusted;
        assert!(controller.manual_stop(&c).await.is_err());
        assert_eq!(controller.info(&c, 0)["canStop"], false);
    }
    provider.fail_check.store(true, Ordering::SeqCst);
    let error = controller
        .manual_stop(&caller())
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("fixture-secret"));
    assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
    assert!(!provider.order.lock().unwrap().contains(&"disconnect"));
    provider.fail_check.store(false, Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(
            controller.manual_stop(&caller()).await.unwrap(),
            json!({"accepted":true})
        );
    }
    until(|| provider.stops.load(Ordering::SeqCst) == 1).await;
    let order = provider.order.lock().unwrap().clone();
    assert_eq!(&order[order.len() - 2..], ["disconnect", "stop"]);
    assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    controller.close();
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn automatic_stop_requires_continuous_quiet_and_all_schedules_block() {
    let (source, provider, controller, task) = setup(true);
    source.busy.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(65)).await;
    assert_eq!(provider.checks.load(Ordering::SeqCst), 0);
    source.busy.store(false, Ordering::SeqCst);
    source.job.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(65)).await;
    assert_eq!(provider.checks.load(Ordering::SeqCst), 0);
    source.job.store(false, Ordering::SeqCst);
    source.unknown.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(65)).await;
    assert_eq!(provider.checks.load(Ordering::SeqCst), 0);
    source.unknown.store(false, Ordering::SeqCst);
    until(|| provider.stops.load(Ordering::SeqCst) == 1).await;
    controller.close();
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn automatic_recheck_cancels_new_work_but_manual_request_can_override() {
    let (source, provider, controller, task) = setup(true);
    provider.work_after_check.store(true, Ordering::SeqCst);
    until(|| provider.checks.load(Ordering::SeqCst) > 0).await;
    tokio::time::sleep(Duration::from_millis(90)).await;
    assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
    assert_eq!(
        controller.info(&caller(), chrono::Utc::now().timestamp_millis())["stopping"],
        false
    );
    source.busy.store(false, Ordering::SeqCst);
    until(|| controller.info(&caller(), chrono::Utc::now().timestamp_millis())["stopping"] == true)
        .await;
    controller.manual_stop(&caller()).await.unwrap();
    until(|| provider.stops.load(Ordering::SeqCst) == 1).await;
    controller.close();
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn uncertain_stop_is_visible_and_never_automatically_replayed() {
    let (_, provider, controller, task) = setup(true);
    provider.fail_stop.store(true, Ordering::SeqCst);
    until(|| {
        !controller.info(&caller(), chrono::Utc::now().timestamp_millis())["error"]
            .as_str()
            .unwrap()
            .is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    assert!(
        !controller
            .info(&caller(), 0)
            .to_string()
            .contains("fixture-secret")
    );
    provider.fail_stop.store(false, Ordering::SeqCst);
    controller.manual_stop(&caller()).await.unwrap();
    until(|| provider.stops.load(Ordering::SeqCst) == 2).await;
    controller.close();
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn owner_close_cancels_accepted_delay_without_host_effect() {
    let (_, provider, controller, task) = setup(false);
    controller.manual_stop(&caller()).await.unwrap();
    controller.close();
    task.await.unwrap().unwrap();
    assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
    assert!(!provider.order.lock().unwrap().contains(&"disconnect"));
}
#[test]
fn idle_power_promotes_distant_shell_schedule_to_running_blocker() {
    let evidence = Evidence {
        now_ms: 1000,
        sessions: Ok(json!([])),
        clients: Ok(vec![]),
        jobs: Ok(vec![JobInfo {
            id: "future".into(),
            name: String::new(),
            action_kind: "shell".into(),
            next_run_ms: Some(86_400_000),
            running: false,
        }]),
        peers: Ok(vec![]),
        operations: vec![],
    };
    assert_eq!(
        evaluate(
            &idle_inputs(evidence),
            Tunables::default(),
            &Default::default()
        )[0]
        .kind,
        "job-running"
    );
}

#[tokio::test]
async fn observation_mode_reports_quiet_without_preflight_or_action() {
    let (source, provider, old, old_task) = setup(false);
    old.close();
    old_task.await.unwrap().unwrap();
    let controller = Controller::new(
        PowerConfig {
            idle_timeout_ms: 30,
            requested_stop: false,
            ..Default::default()
        },
        Some(provider.clone()),
        source,
        Arc::new(|| Box::pin(async { panic!("observe mode disconnected clients") })),
        Timing {
            sample: Duration::from_millis(5),
            receipt_delay: Duration::from_millis(5),
            ..Default::default()
        },
    );
    let running = controller.clone();
    let task = tokio::spawn(async move { running.run().await });
    until(|| {
        controller.info(&caller(), chrono::Utc::now().timestamp_millis())["idle"]["quiescent"]
            == true
    })
    .await;
    let info = controller.info(&caller(), chrono::Utc::now().timestamp_millis());
    assert_eq!(info["idleMode"], "observe");
    assert_eq!(info["canStop"], true);
    assert_eq!(provider.checks.load(Ordering::SeqCst), 0);
    assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
    controller.close();
    task.await.unwrap().unwrap();
}
