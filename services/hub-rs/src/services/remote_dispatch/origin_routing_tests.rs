use super::*;
use std::sync::Mutex;
struct Echo(Mutex<Vec<Value>>);
impl Link for Echo {
    fn dispatch_enabled(&self, _: &str) -> bool {
        true
    }
    fn call<'a>(&'a self, _: &'a str, method: &'a str, params: Value) -> Operation<'a, Value> {
        Box::pin(async move {
            assert_eq!(method, "agents.spawn");
            self.0.lock().unwrap().push(params.clone());
            Ok(params)
        })
    }
}
struct NoDelivery;
impl Delivery for NoDelivery {
    fn recipient<'a>(&'a self, _: &'a str) -> Operation<'a, Option<String>> {
        Box::pin(async { panic!("not an owned fixture") })
    }
    fn deliver<'a>(&'a self, _: &'a str, _: &'a OriginRecord, _: &'a Update) -> Operation<'a, ()> {
        Box::pin(async { panic!("not an owned fixture") })
    }
}
struct LocalProject(String);
impl Booking for LocalProject {
    fn local_session_id(&self) -> &str {
        "paired:fixture"
    }
    fn source_root(&self) -> &str {
        &self.0
    }
    fn prepare<'a>(&'a self, _: &'a OriginRecord, _: &'a Value) -> Operation<'a, Value> {
        Box::pin(async { panic!("routing-only fixture must not book") })
    }
}
#[tokio::test]
async fn paired_gate_uses_host_owned_local_project_and_preserves_remote_execution_cwd() {
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("local-project");
    let remote = root.path().join("remote-execution");
    std::fs::create_dir(&local).unwrap();
    std::fs::create_dir(&remote).unwrap();
    let config = root.path().join("config");
    std::fs::create_dir(&config).unwrap();
    std::fs::write(config.join("routing.yaml"),format!("ceilings:\n  default: {{max_capability: frontier_plus}}\n  {}: {{max_capability: cheap}}\n",serde_json::to_string(&local.to_string_lossy()).unwrap())).unwrap();
    let routing = Arc::new(crate::services::routing::RoutingService::open(config.clone()).unwrap());
    let hub = crate::Hub::start(crate::Options::default()).unwrap();
    hub.ready().await.unwrap();
    let link = Arc::new(Echo(Mutex::new(vec![])));
    let origin = Origin::open_with_routing(
        root.path().join("journal"),
        hub.handle(),
        link.clone(),
        Arc::new(NoDelivery),
        Some(routing),
    )
    .unwrap();
    let caller = Caller {
        call_id: 1,
        activity_seq: 1,
        connection_id: 1,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: "caller-fingerprint".into(),
        federated: false,
    };
    let request = json!({"cwd":remote,"sourceRoot":remote,"provider":"claude","modelIdentity":"fable","contextWindow":1_000_000,"exactModel":true});
    // Floor: the remote cwd alone selects the permissive default. A fake wire
    // sourceRoot must not replace the private booking's already-validated project.
    assert_eq!(
        origin
            .forward_sanitized(&caller, "peer", request.clone())
            .await
            .unwrap()["modelIdentity"],
        "fable"
    );
    let booking: Arc<dyn Booking> = Arc::new(LocalProject(local.to_string_lossy().into_owned()));
    let error = origin
        .forward_booked(&caller, "peer", request.clone(), Some(booking.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no substitute was launched"), "{error}");
    assert_eq!(
        link.0.lock().unwrap().len(),
        1,
        "paired refusal reached the link"
    );
    let mut request = request;
    request["exactModel"] = json!(false);
    let sent = origin
        .forward_booked(&caller, "peer", request, Some(booking))
        .await
        .unwrap();
    assert_eq!(sent["cwd"], json!(remote));
    assert_eq!(sent["model"], "sonnet");
    assert!(sent.get("modelIdentity").is_none() && sent.get("contextWindow").is_none());
    assert_eq!(link.0.lock().unwrap().len(), 2);
    let rows: Vec<Value> = std::fs::read_to_string(config.join("routing-decisions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1]["spawn"]["outcome"], "refused");
    assert_eq!(rows[2]["spawn"]["outcome"], "clamped");
    assert_eq!(
        rows[2]["spawn"]["cwd"],
        json!(std::fs::canonicalize(local).unwrap())
    );
    hub.shutdown().unwrap();
}
