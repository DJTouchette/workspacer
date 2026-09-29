use super::*;
#[test]
fn registry_is_explicit_and_corruption_never_silently_disables_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nodes.json");
    assert!(load(&path).unwrap().is_empty());
    for body in [
        "broken",
        r#"[{"id":"bad/id"}]"#,
        r#"[{"id":"node","fly":{"app":"half"}}]"#,
        r#"[{"id":"same"},{"id":"same"}]"#,
    ] {
        std::fs::write(&path, body).unwrap();
        assert!(load(&path).is_err());
    }
    std::fs::write(
        &path,
        r#"[{"id":" node ","label":" name ","fly":{"token":"private"}},{"id":"plain"}]"#,
    )
    .unwrap();
    let rows = load(&path).unwrap();
    assert_eq!(rows[0].id, "node");
    assert_eq!(rows[0].label, "name");
    assert!(!rows[0].coordinates());
}

use cloud::{Cloud, Failure, Outcome};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct FakeCloud {
    starts: AtomicUsize,
    stops: AtomicUsize,
    state: Mutex<Option<cloud::State>>,
    start_errors: Mutex<std::collections::VecDeque<Failure>>,
    stop_error: Mutex<Option<Failure>>,
    stop_wait_error: Mutex<Option<Failure>>,
    hold_start: std::sync::atomic::AtomicBool,
    release_start: tokio::sync::Notify,
    signals: Mutex<Vec<Duration>>,
}
impl Cloud for FakeCloud {
    fn start(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if self.hold_start.load(Ordering::SeqCst) {
                self.release_start.notified().await;
            }
            if let Some(error) = self.start_errors.lock().unwrap().pop_front() {
                return Err(error);
            }
            *self.state.lock().unwrap() = Some(cloud::State::Started);
            Ok(())
        })
    }
    fn stop(
        &self,
        grace: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async move {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.signals.lock().unwrap().push(grace);
            if let Some(error) = *self.stop_error.lock().unwrap() {
                return Err(error);
            }
            *self.state.lock().unwrap() = Some(cloud::State::Stopped);
            Ok(())
        })
    }
    fn state(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<cloud::State>> + Send + '_>>
    {
        Box::pin(async { Ok(self.state.lock().unwrap().unwrap_or(cloud::State::Stopped)) })
    }
    fn wait(
        &self,
        state: cloud::State,
        _: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async move {
            if state == cloud::State::Stopped {
                if let Some(error) = *self.stop_wait_error.lock().unwrap() {
                    return Err(error);
                }
            }
            Ok(())
        })
    }
}
struct FakeProbe {
    reading: Mutex<Reading>,
    evictions: Mutex<Vec<u64>>,
}
impl FakeProbe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            reading: Mutex::new(Reading::Absent),
            evictions: Mutex::new(vec![]),
        })
    }
}
impl Probe for FakeProbe {
    fn read(&self, _: Duration) -> Operation<'_, Reading> {
        Box::pin(async { Ok(self.reading.lock().unwrap().clone()) })
    }
    fn evict(&self, connection: u64) -> Operation<'_, bool> {
        Box::pin(async move {
            let mut current = self.reading.lock().unwrap();
            let matches = matches!(*current,Reading::Silent(id) if id==connection);
            if matches {
                self.evictions.lock().unwrap().push(connection);
                *current = Reading::Absent
            }
            Ok(matches)
        })
    }
}
struct Fixture {
    service: Arc<Supervisor>,
    cloud: Arc<FakeCloud>,
    probe: Arc<FakeProbe>,
    changes: Arc<Mutex<Vec<Change>>>,
}
impl Fixture {
    fn new(count: usize, keep: bool) -> Self {
        Self::timed(count, keep, Duration::from_secs(10))
    }
    fn fast(count: usize, keep: bool) -> Self {
        Self::timed(count, keep, Duration::from_millis(50))
    }
    fn timed(count: usize, keep: bool, register: Duration) -> Self {
        let cloud = Arc::new(FakeCloud::default());
        let probe = FakeProbe::new();
        let changes = Arc::new(Mutex::new(vec![]));
        let publish = changes.clone();
        let nodes = (0..count)
            .map(|index| Node {
                id: format!("node{index}"),
                label: format!("Worker {index}"),
                fly: Some(CloudConfig {
                    app: "private-app".into(),
                    machine_id: "private-machine".into(),
                    token: "PRIVATE_CLOUD_CREDENTIAL".into(),
                    token_file: "/private/secret".into(),
                    base_url: "https://private-cloud-endpoint".into(),
                }),
            })
            .collect::<Vec<_>>();
        let clients = nodes
            .iter()
            .map(|node| (node.id.clone(), cloud.clone() as Arc<dyn Cloud>))
            .collect();
        let timings = Timings {
            poll: Duration::from_secs(3600),
            probe: Duration::from_millis(10),
            register,
            register_poll: Duration::from_millis(2),
            retry_delay: Duration::from_millis(1),
            stop_timeout: Duration::from_millis(20),
            keep_failed_wakes_running: keep,
            ..Timings::default()
        };
        let service = Supervisor::new(
            nodes,
            clients,
            probe.clone(),
            timings,
            Arc::new(move |change| publish.lock().unwrap().push(change)),
        )
        .unwrap();
        Self {
            service,
            cloud,
            probe,
            changes,
        }
    }
    fn start(&self) -> tokio::task::JoinHandle<Result<()>> {
        let service = self.service.clone();
        tokio::spawn(service.run())
    }
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn reconciliation_is_three_valued_and_views_never_disclose_cloud_configuration() {
    let f = Fixture::new(1, false);
    assert_eq!(f.service.list()[0].state, State::Unreachable);
    f.service.reconcile().await;
    assert_eq!(f.service.list()[0].state, State::Stopped);
    *f.cloud.state.lock().unwrap() = Some(cloud::State::Started);
    f.service.reconcile().await;
    assert_eq!(f.service.list()[0].state, State::Unreachable);
    assert!(f.service.list()[0].may_be_running);
    *f.cloud.state.lock().unwrap() = Some(cloud::State::Unknown);
    f.service.reconcile().await;
    assert!(f.service.list()[0].may_be_running);
    let prior = f.service.list();
    *f.probe.reading.lock().unwrap() = Reading::Unavailable;
    f.service.reconcile().await;
    assert_eq!(prior, f.service.list());
    let wire = serde_json::to_string(&f.service.list()).unwrap();
    for secret in [
        "PRIVATE_CLOUD_CREDENTIAL",
        "private-app",
        "private-machine",
        "/private/secret",
        "private-cloud-endpoint",
    ] {
        assert!(!wire.contains(secret));
    }
}
#[tokio::test]
async fn silent_strikes_are_bound_to_the_registered_connection_and_absence_never_evicts() {
    let f = Fixture::new(1, false);
    f.service.reconcile().await;
    assert!(f.probe.evictions.lock().unwrap().is_empty());
    *f.probe.reading.lock().unwrap() = Reading::Silent(1);
    f.service.reconcile().await;
    *f.probe.reading.lock().unwrap() = Reading::Unavailable;
    f.service.reconcile().await;
    assert!(f.probe.evictions.lock().unwrap().is_empty());
    *f.probe.reading.lock().unwrap() = Reading::Silent(2);
    f.service.reconcile().await;
    assert!(f.probe.evictions.lock().unwrap().is_empty());
    f.service.reconcile().await;
    assert_eq!(*f.probe.evictions.lock().unwrap(), vec![2]);
}
#[tokio::test]
async fn anonymous_provider_is_ambiguous_for_multiple_nodes_but_named_provider_is_available() {
    let f = Fixture::new(2, false);
    *f.probe.reading.lock().unwrap() = Reading::Answered {
        connection: 1,
        node: String::new(),
        last_exit: None,
    };
    f.service.reconcile().await;
    assert!(
        f.service
            .list()
            .iter()
            .all(|view| view.state != State::Available)
    );
    *f.probe.reading.lock().unwrap() = Reading::Answered {
        connection: 1,
        node: "node1".into(),
        last_exit: Some(ExitRecord {
            reason: "boot-failure".into(),
            exit_code: Some(1),
            at: "yesterday".into(),
        }),
    };
    f.service.reconcile().await;
    let view = f.service.view("node1").unwrap();
    assert_eq!(view.state, State::Available);
    assert!(view.detail.contains("DID NOT END CLEANLY"));
    assert_eq!(f.service.view("node0").unwrap().state, State::Stopped);
}
#[tokio::test]
async fn concurrent_wake_and_sleep_are_single_actions_and_preserve_drain_policy() {
    let f = Fixture::new(1, false);
    let running = f.start();
    let (a, b, c) = tokio::join!(
        f.service.wake("node0"),
        f.service.wake("node0"),
        f.service.wake("node0")
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert_eq!(f.cloud.starts.load(Ordering::SeqCst), 1);
    let (a, b) = tokio::join!(f.service.sleep("node0"), f.service.sleep("node0"));
    assert!(a.is_ok() && b.is_ok());
    until(|| f.service.view("node0").unwrap().state == State::Stopped).await;
    assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        *f.cloud.signals.lock().unwrap(),
        vec![Duration::from_secs(45)]
    );
    assert!(f.service.view("node0").unwrap().slept_by_hub);
    f.service.close();
    running.await.unwrap().unwrap();
}
#[tokio::test]
async fn failed_wake_stops_only_its_owned_attempt_and_never_a_ready_provider() {
    let f = Fixture::fast(1, false);
    let running = f.start();
    f.service.wake("node0").await.unwrap();
    until(|| f.cloud.stops.load(Ordering::SeqCst) == 1).await;
    until(|| !f.service.view("node0").unwrap().may_be_running).await;
    let view = f.service.view("node0").unwrap();
    assert_eq!(view.state, State::Unreachable);
    assert_eq!(view.wake_failures, 1);
    assert!(!view.slept_by_hub);
    f.service.reconcile().await;
    assert_eq!(f.service.view("node0").unwrap().state, State::Unreachable);
    f.service.close();
    running.await.unwrap().unwrap();
    let f = Fixture::new(1, false);
    *f.probe.reading.lock().unwrap() = Reading::Answered {
        connection: 1,
        node: "node0".into(),
        last_exit: None,
    };
    let running = f.start();
    f.service.wake("node0").await.unwrap();
    assert_eq!(f.service.view("node0").unwrap().state, State::Available);
    assert_eq!(f.cloud.starts.load(Ordering::SeqCst), 0);
    assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 0);
    f.service.close();
    running.await.unwrap().unwrap();
}
#[tokio::test]
async fn keep_failed_wake_and_failed_stop_remain_explicitly_running_or_unknown() {
    let f = Fixture::fast(1, true);
    let running = f.start();
    f.service.wake("node0").await.unwrap();
    until(|| f.service.view("node0").unwrap().wake_failures == 1).await;
    assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 0);
    assert!(f.service.view("node0").unwrap().may_be_running);
    *f.cloud.stop_wait_error.lock().unwrap() = Some(Failure::Timeout);
    f.service.sleep("node0").await.unwrap();
    until(|| f.service.view("node0").unwrap().state == State::Unreachable).await;
    assert!(f.service.view("node0").unwrap().may_be_running);
    assert!(!f.service.view("node0").unwrap().slept_by_hub);
    f.service.close();
    running.await.unwrap().unwrap();
}
#[tokio::test]
async fn retry_is_bounded_and_sleep_supersedes_a_start_without_repeating_the_stop() {
    let f = Fixture::new(1, false);
    f.cloud
        .start_errors
        .lock()
        .unwrap()
        .push_back(Failure::Status(500));
    let running = f.start();
    f.service.wake("node0").await.unwrap();
    assert_eq!(f.cloud.starts.load(Ordering::SeqCst), 2);
    f.service.sleep("node0").await.unwrap();
    until(|| f.service.view("node0").unwrap().state == State::Stopped).await;
    f.service.close();
    running.await.unwrap().unwrap();
    for error in [Failure::Status(404), Failure::RateLimited] {
        let f = Fixture::new(1, false);
        f.cloud.start_errors.lock().unwrap().push_back(error);
        let running = f.start();
        assert!(f.service.wake("node0").await.is_err());
        assert_eq!(f.cloud.starts.load(Ordering::SeqCst), 1);
        assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 0);
        f.service.close();
        running.await.unwrap().unwrap();
    }
}
#[tokio::test]
async fn cancelled_caller_does_not_abandon_an_accepted_start_or_race_a_later_stop() {
    let f = Fixture::new(1, false);
    f.cloud.hold_start.store(true, Ordering::SeqCst);
    let running = f.start();
    let service = f.service.clone();
    let wake = tokio::spawn(async move { service.wake("node0").await });
    until(|| f.cloud.starts.load(Ordering::SeqCst) == 1).await;
    wake.abort();
    let _ = wake.await;
    let service = f.service.clone();
    let sleep = tokio::spawn(async move { service.sleep("node0").await });
    until(|| f.service.view("node0").unwrap().state == State::Stopping).await;
    assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 0);
    f.cloud.release_start.notify_one();
    sleep.await.unwrap().unwrap();
    until(|| f.service.view("node0").unwrap().state == State::Stopped).await;
    assert_eq!(f.cloud.starts.load(Ordering::SeqCst), 1);
    assert_eq!(f.cloud.stops.load(Ordering::SeqCst), 1);
    f.service.close();
    running.await.unwrap().unwrap();
    assert!(
        f.changes
            .lock()
            .unwrap()
            .iter()
            .any(|change| change.node.state == State::Stopping)
    );
}
#[tokio::test]
async fn shutdown_owns_and_cancels_background_cloud_waits() {
    let f = Fixture::new(1, false);
    f.cloud.hold_start.store(true, Ordering::SeqCst);
    let running = f.start();
    let service = f.service.clone();
    let call = tokio::spawn(async move { service.wake("node0").await });
    until(|| f.cloud.starts.load(Ordering::SeqCst) == 1).await;
    f.service.close();
    tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(call.await.unwrap().is_err());
    assert!(f.service.wake("node0").await.is_err());
}

#[tokio::test]
async fn cloud_transport_uses_only_host_coordinates_and_does_not_render_error_bodies() {
    use axum::{
        Router,
        extract::{Request, State as AxumState},
        http::StatusCode,
        response::IntoResponse,
        routing::any,
    };
    #[derive(Default)]
    struct Server {
        calls: Mutex<Vec<(String, String, serde_json::Value)>>,
        status: std::sync::atomic::AtomicU16,
    }
    async fn request(
        AxumState(server): AxumState<Arc<Server>>,
        request: Request,
    ) -> axum::response::Response {
        let path = request.uri().to_string();
        let credential = request
            .headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let method = request.method().clone();
        let bytes = axum::body::to_bytes(request.into_body(), 8192)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        server.calls.lock().unwrap().push((path, credential, body));
        let status = server.status.load(Ordering::SeqCst);
        if status > 0 {
            return (
                StatusCode::from_u16(status).unwrap(),
                "PRIVATE_CLOUD_CREDENTIAL should never escape",
            )
                .into_response();
        }
        if method == "GET" {
            axum::Json(json!({"state":"stopped"})).into_response()
        } else {
            axum::Json(json!({})).into_response()
        }
    }
    let state = Arc::new(Server::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(any(request))
        .with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cloud = cloud::Http::new(
        &CloudConfig {
            app: "host/app".into(),
            machine_id: "machine?id".into(),
            token: String::new(),
            token_file: String::new(),
            base_url: format!("http://{address}/prefix"),
        },
        "PRIVATE_CLOUD_CREDENTIAL".into(),
    )
    .unwrap();
    cloud.start().await.unwrap();
    cloud.stop(Duration::from_secs(45)).await.unwrap();
    assert_eq!(cloud.state().await.unwrap(), cloud::State::Stopped);
    cloud
        .wait(cloud::State::Stopped, Duration::from_secs(120))
        .await
        .unwrap();
    let calls = state.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls.iter().all(|(path, token, _)| {
        path.starts_with("/prefix/v1/apps/host%2Fapp/machines/machine%3Fid")
            && token == "Bearer PRIVATE_CLOUD_CREDENTIAL"
    }));
    assert_eq!(calls[1].2, json!({"signal":"SIGTERM","timeout":"45s"}));
    assert!(calls[3].0.contains("timeout=60"));
    drop(calls);
    state.status.store(403, Ordering::SeqCst);
    let error = cloud.state().await.unwrap_err();
    assert_eq!(error, Failure::Status(403));
    assert!(!error.to_string().contains("PRIVATE_CLOUD_CREDENTIAL"));
    server.abort();
    let _ = server.await;
}
#[tokio::test]
async fn real_broker_node_routes_use_scope_and_named_external_provider_liveness() {
    use crate::protocol::Frame;
    use axum::{
        Router,
        extract::{Request, State as AxumState},
        response::IntoResponse,
        routing::any,
    };
    #[derive(Default)]
    struct Server {
        started: std::sync::atomic::AtomicBool,
        starts: AtomicUsize,
        stops: AtomicUsize,
        stop_body: Mutex<serde_json::Value>,
    }
    async fn cloud(
        AxumState(state): AxumState<Arc<Server>>,
        request: Request,
    ) -> axum::response::Response {
        let path = request.uri().path().to_owned();
        assert!(path.starts_with("/v1/apps/app/machines/machine"));
        assert_eq!(
            request.headers()["authorization"],
            "Bearer PRIVATE_CLOUD_CREDENTIAL"
        );
        if path.ends_with("/start") {
            state.starts.fetch_add(1, Ordering::SeqCst);
            state.started.store(true, Ordering::SeqCst);
        }
        if path.ends_with("/stop") {
            state.stops.fetch_add(1, Ordering::SeqCst);
            state.started.store(false, Ordering::SeqCst);
            let bytes = axum::body::to_bytes(request.into_body(), 8192)
                .await
                .unwrap();
            *state.stop_body.lock().unwrap() = serde_json::from_slice(&bytes).unwrap();
        }
        axum::Json(
            json!({"state":if state.started.load(Ordering::SeqCst){"started"}else{"stopped"}}),
        )
        .into_response()
    }
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(Server::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(any(cloud)).with_state(server_state),
        )
        .await
        .unwrap()
    });
    let registry = root.path().join("nodes.json");
    std::fs::write(&registry,serde_json::to_vec(&json!([{"id":"node0","label":"remote worker","fly":{"app":"app","machineId":"machine","token":"PRIVATE_CLOUD_CREDENTIAL","baseUrl":format!("http://{address}")}}])).unwrap()).unwrap();
    let tokens = root.path().join("tokens.json");
    let view = crate::auth::mint(&tokens, crate::auth::Scope::View, "view").unwrap();
    let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "operator").unwrap();
    let provider = crate::auth::mint(&tokens, crate::auth::Scope::Provider, "node").unwrap();
    let mut options = Options::default();
    options.token = "host-fixture".into();
    options.scoped_tokens = Some(tokens);
    options.nodes_file = Some(registry);
    let hub = crate::Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let viewer = Client::from_connection(
        hub.handle()
            .connect_authenticated(view.token, false)
            .await
            .unwrap(),
    );
    let owner = Client::from_connection(
        hub.handle()
            .connect_authenticated(operator.token, false)
            .await
            .unwrap(),
    );
    assert!(
        viewer
            .call("nodes.wake", json!({"id":"node0"}))
            .await
            .is_err()
    );
    let listing = viewer.call("nodes.list", json!({})).await.unwrap();
    let rendered = listing.to_string();
    for secret in [
        "PRIVATE_CLOUD_CREDENTIAL",
        "machineId",
        "baseUrl",
        "tokenFile",
    ] {
        assert!(!rendered.contains(secret));
    }
    let wake = owner
        .call(
            "nodes.wake",
            json!({"id":"node0","app":"foreign","machineId":"other","token":"forged"}),
        )
        .await
        .unwrap();
    assert_eq!(wake["state"], "waking");
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    let mut connection = hub
        .handle()
        .connect_authenticated(provider.token, false)
        .await
        .unwrap();
    connection.recv().await.unwrap();
    connection
        .send(Frame {
            methods: vec!["brain.info".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(connection.recv().await.unwrap().methods, vec!["brain.info"]);
    let provider_task = tokio::spawn(async move {
        while let Some(frame) = connection.recv().await {
            if frame.op == "call" {
                connection
                    .send(Frame {
                        id: frame.id,
                        result: Some(json!({"scope":"full","provider":"rust","node":"node0"})),
                        ..Frame::op("result")
                    })
                    .unwrap();
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if viewer.call("nodes.list", json!({})).await.unwrap()[0]["state"] == "available" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let sleep=owner.call("nodes.sleep",json!({"id":"node0","signal":"SIGKILL","timeout":0,"app":"foreign","machineId":"other","baseUrl":"https://foreign"})).await.unwrap();
    assert_eq!(sleep["state"], "stopping");
    assert_eq!(
        *state.stop_body.lock().unwrap(),
        json!({"signal":"SIGTERM","timeout":"45s"})
    );
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    hub.shutdown().unwrap();
    provider_task.await.unwrap();
    server.abort();
    let _ = server.await;
}
