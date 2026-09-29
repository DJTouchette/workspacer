use super::*;
#[derive(Clone, Copy, PartialEq)]
enum Failure {
    None,
    Start,
    Call,
    Message,
    MissingSession,
}
struct Fixture {
    calls: Mutex<Vec<(String, Value)>>,
    output: String,
    exit_ok: bool,
    call_output: Value,
    failure: Failure,
}
impl Fixture {
    fn new(output: &str) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            output: output.into(),
            exit_ok: true,
            call_output: json!({"ok":true}),
            failure: Failure::None,
        }
    }
}
impl Runner for Fixture {
    fn invoke<'a>(&'a self, method: &'a str, params: Value) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((method.into(), params));
            if self.failure == Failure::Call
                || (method == "agents.sendMessage" && self.failure == Failure::Message)
            {
                bail!("fixture call failed");
            }
            Ok(if method == "agents.spawn" {
                if self.failure == Failure::MissingSession {
                    json!({})
                } else {
                    json!({"sessionId":"child"})
                }
            } else {
                self.call_output.clone()
            })
        })
    }
    fn shell<'a>(&'a self, action: &'a Value) -> BoxFuture<'a, Result<(String, bool)>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(("shell".into(), action.clone()));
            if self.failure == Failure::Start {
                bail!("fixture command could not start");
            }
            Ok((self.output.clone(), self.exit_ok))
        })
    }
}
fn spawn(step: Value) -> Job {
    serde_json::from_value(json!({"name":"fixture","trigger":{"kind":"manual"},"action":{"kind":"spawn","spawn":{"cwd":"/fixture","prompt":"Task: {{output}}","context":[step]}}})).unwrap()
}
fn owner() -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        connection_id: 1,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: String::new(),
        federated: false,
    }
}
#[tokio::test]
async fn empty_context_and_failed_context_never_reach_agent_admission() {
    for empty in [Value::Null, json!({}), json!([]), json!("")] {
        let mut runner = Fixture::new("");
        runner.call_output = empty;
        let job = spawn(json!({"kind":"call","call":{"method":"context.read"},"skipIfEmpty":true}));
        assert!(
            perform(&job, &runner)
                .await
                .unwrap_err()
                .downcast_ref::<Skip>()
                .is_some()
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
    for (failure, ignore, success) in [
        (Failure::None, true, true),
        (Failure::None, false, false),
        (Failure::Start, true, false),
        (Failure::Call, true, false),
    ] {
        let mut runner = Fixture::new("exit status is data");
        runner.failure = failure;
        runner.exit_ok = false;
        let step = if failure == Failure::Call {
            json!({"kind":"call","call":{"method":"context.read"},"ignoreExitCode":ignore})
        } else {
            json!({"kind":"shell","shell":{"command":"fixture"},"ignoreExitCode":ignore})
        };
        let result = perform(&spawn(step), &runner).await;
        assert_eq!(result.is_ok(), success);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), if success { 3 } else { 1 });
        if !success {
            assert!(result.unwrap_err().downcast_ref::<Skip>().is_none());
        }
        if calls[0].0 == "shell" {
            assert!(
                calls[0].1.get("cwd").is_none(),
                "context inherited spawn cwd"
            );
        }
    }
}
#[tokio::test]
async fn full_output_guards_precede_middle_elision_and_spawn_is_never_replayed() {
    let output = format!("{}MIDDLE{}", "A".repeat(6000), "Z".repeat(12000));
    let runner = Fixture::new(&output);
    let job =
        spawn(json!({"kind":"shell","shell":{"command":"fixture"},"skipUnlessMatch":"MIDDLE"}));
    perform(&job, &runner).await.unwrap();
    {
        let calls = runner.calls.lock().unwrap();
        let prompt = calls.last().unwrap().1["text"].as_str().unwrap();
        assert!(prompt.starts_with("Task: AAAA") && prompt.ends_with("ZZZZ"));
        assert!(prompt.contains("characters elided"));
        assert!(!prompt.contains("MIDDLE"));
        assert!(prompt.len() < 12120);
    }
    for failure in [Failure::MissingSession, Failure::Message] {
        let mut runner = Fixture::new("MIDDLE");
        runner.failure = failure;
        let error = perform(&job, &runner).await.unwrap_err();
        assert!(error.to_string().contains(if failure == Failure::Message {
            "spawned child but prompt failed"
        } else {
            "no sessionId"
        }));
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|(method, _)| method == "agents.spawn")
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|(method, _)| method == "agents.sendMessage")
                .count(),
            usize::from(failure == Failure::Message)
        );
    }
}
#[tokio::test]
async fn skipped_runs_are_quiet_failed_runs_notify_and_history_survives_its_cap() {
    use crate::protocol::Frame;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("jobs.json");
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut subscriber = hub.handle().connect().await.unwrap();
    subscriber.recv().await.unwrap();
    subscriber
        .send(Frame {
            topics: vec!["notify.post".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    subscriber.recv().await.unwrap();
    let (service, _) = Service::open(path.clone(), hub.handle());
    let job = spawn(json!({"kind":"shell","shell":{"command":"fixture"},"skipIfEmpty":true}));
    let saved = service.call(&owner(), "jobs.upsert", json!(job)).unwrap();
    let job: Job = serde_json::from_value(saved).unwrap();
    service
        .clone()
        .execute_with_runner(job.clone(), &Fixture::new(""))
        .await;
    hub.handle().health().await.unwrap();
    assert_eq!(
        service.state.lock().unwrap().history[&job.id][0].status,
        "skipped"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), subscriber.recv())
            .await
            .is_err(),
        "skip raised a notification"
    );
    let mut runner = Fixture::new("");
    runner.failure = Failure::Start;
    service
        .clone()
        .execute_with_runner(job.clone(), &runner)
        .await;
    let notification = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    let body = notification.data.unwrap();
    assert_eq!(body["level"], "error");
    assert_eq!(body["key"], format!("job-{}", job.id));
    assert_eq!(
        service.state.lock().unwrap().history[&job.id][0].status,
        "error"
    );
    {
        let mut state = service.state.lock().unwrap();
        for stamp in 0..40 {
            service.record(
                &mut state,
                Run {
                    job_id: job.id.clone(),
                    started_at: stamp,
                    finished_at: stamp,
                    status: "ok".into(),
                    detail: "fixture".into(),
                },
            );
        }
    }
    let (reopened, _) = Service::open(path, hub.handle());
    let history = reopened
        .call(&owner(), "jobs.history", json!({"id":job.id}))
        .unwrap();
    assert_eq!(history["runs"].as_array().unwrap().len(), 30);
    assert_eq!(history["runs"][0]["startedAt"], 39);
    assert_eq!(history["runs"][29]["startedAt"], 10);
    reopened
        .call(&owner(), "jobs.remove", json!({"id":job.id}))
        .unwrap();
    assert!(
        reopened
            .call(&owner(), "jobs.history", json!({"id":job.id}))
            .unwrap()["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    hub.shutdown().unwrap();
}
