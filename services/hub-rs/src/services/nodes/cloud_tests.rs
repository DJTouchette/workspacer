use super::*;
use axum::{
    Router,
    extract::{Request, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::any,
};
use std::sync::{
    Mutex,
    atomic::{AtomicU16, Ordering},
};

const SECRET: &str = "cloud-fixture-secret";
struct Call {
    method: String,
    uri: String,
    headers: HeaderMap,
    body: Value,
}
struct Server {
    calls: Mutex<Vec<Call>>,
    status: AtomicU16,
    body: Mutex<String>,
}
struct Fixture {
    server: Arc<Server>,
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        async fn respond(
            AxumState(state): AxumState<Arc<Server>>,
            request: Request,
        ) -> axum::response::Response {
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, 8192).await.unwrap();
            state.calls.lock().unwrap().push(Call {
                method: parts.method.to_string(),
                uri: parts.uri.to_string(),
                headers: parts.headers,
                body: if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                },
            });
            (
                StatusCode::from_u16(state.status.load(Ordering::SeqCst)).unwrap(),
                [("Retry-After", "3")],
                state.body.lock().unwrap().clone(),
            )
                .into_response()
        }
        let server = Arc::new(Server {
            calls: Mutex::new(vec![]),
            status: AtomicU16::new(200),
            body: Mutex::new(r#"{"state":"stopped"}"#.into()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/prefix", listener.local_addr().unwrap());
        let app = Router::new()
            .fallback(any(respond))
            .with_state(server.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { server, base, task }
    }
    fn client(&self, id: &str) -> Arc<Http> {
        Http::new(
            &CloudConfig {
                app: "host/app".into(),
                machine_id: id.into(),
                base_url: self.base.clone(),
                ..Default::default()
            },
            SECRET.into(),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn wire_contract_preserves_wait_bounds_and_fractional_stop_grace() {
    let f = Fixture::new().await;
    let c = f.client("machine?id");
    assert_eq!(c.stop(Duration::ZERO).await, Err(Failure::Invalid));
    assert!(f.server.calls.lock().unwrap().is_empty());
    assert!(
        c.gates.lock().await.is_empty(),
        "invalid stop consumed a rate slot"
    );
    c.start().await.unwrap();
    c.stop(Duration::from_millis(1500)).await.unwrap();
    assert_eq!(c.state().await.unwrap(), State::Stopped);
    for timeout in [
        Duration::ZERO,
        Duration::from_secs(45),
        Duration::from_secs(600),
    ] {
        c.wait(State::Started, timeout).await.unwrap();
    }
    let calls = f.server.calls.lock().unwrap();
    assert_eq!(calls.len(), 6);
    for call in calls.iter() {
        assert!(
            call.uri
                .starts_with("/prefix/v1/apps/host%2Fapp/machines/machine%3Fid")
        );
        assert_eq!(call.headers["authorization"], format!("Bearer {SECRET}"));
        assert_eq!(call.headers["accept"], "application/json");
        assert!(!call.uri.contains(SECRET));
        assert!(!call.body.to_string().contains(SECRET));
    }
    assert_eq!(calls[0].method, "POST");
    assert!(calls[0].uri.ends_with("/start"));
    assert_eq!(calls[1].method, "POST");
    assert!(calls[1].uri.ends_with("/stop"));
    assert_eq!(calls[1].headers["content-type"], "application/json");
    assert_eq!(calls[1].body, json!({"signal":"SIGTERM","timeout":"1.5s"}));
    for (call, seconds) in calls[3..].iter().zip(["60", "45", "60"]) {
        assert_eq!(call.method, "GET");
        assert!(call.uri.contains("/wait?state=started"));
        assert!(call.uri.ends_with(&format!("timeout={seconds}")));
    }
}

#[tokio::test]
async fn response_categories_never_copy_credentials_or_attacker_body_text() {
    let f = Fixture::new().await;
    *f.server.body.lock().unwrap() = format!("echo Bearer {SECRET}");
    for (status, expected) in [
        (404, Failure::Status(404)),
        (429, Failure::RateLimited),
        (502, Failure::Status(502)),
    ] {
        f.server.status.store(status, Ordering::SeqCst);
        let c = f.client(&format!("id-{status}"));
        for error in [
            c.start().await.unwrap_err(),
            c.stop(Duration::from_secs(45)).await.unwrap_err(),
            c.state().await.unwrap_err(),
            c.wait(State::Started, Duration::from_secs(1))
                .await
                .unwrap_err(),
        ] {
            assert_eq!(error, expected);
            assert!(!error.to_string().contains(SECRET));
            assert!(!format!("{error:?}").contains(SECRET));
        }
    }
    f.server.status.store(200, Ordering::SeqCst);
    let c = f.client("unknown");
    *f.server.body.lock().unwrap() = format!(r#"{{"state":"unrecognized-{SECRET}"}}"#);
    assert_eq!(c.state().await.unwrap(), State::Unknown);
    *f.server.body.lock().unwrap() = "{".into();
    assert_eq!(c.state().await.unwrap_err(), Failure::Invalid);
    *f.server.body.lock().unwrap() = " ".repeat(65537);
    assert_eq!(c.state().await.unwrap_err(), Failure::Invalid);
}

#[tokio::test]
async fn transport_failure_is_a_typed_error_without_credential_text() {
    // Port zero cannot name a listening endpoint; no provider or account is contacted.
    let client = Http::new(
        &CloudConfig {
            app: "app".into(),
            machine_id: "id".into(),
            base_url: "http://127.0.0.1:0".into(),
            ..Default::default()
        },
        SECRET.into(),
    )
    .unwrap();
    let error = client.start().await.unwrap_err();
    assert_eq!(error, Failure::Unavailable);
    assert!(!error.to_string().contains(SECRET));
}

#[tokio::test]
async fn action_reservations_are_per_machine_and_cancelled_waits_send_nothing() {
    let f = Fixture::new().await;
    let c = f.client("first");
    c.start().await.unwrap();
    let first = c.gates.lock().await["start"];
    assert!(
        tokio::time::timeout(Duration::from_millis(30), c.start())
            .await
            .is_err()
    );
    assert!(c.gates.lock().await["start"] >= first + Duration::from_secs(1));
    // A different machine and a different action have independent lanes.
    tokio::time::timeout(Duration::from_millis(500), f.client("second").start())
        .await
        .unwrap()
        .unwrap();
    c.stop(Duration::from_secs(1)).await.unwrap();
    let stop = c.gates.lock().await["stop"];
    c.stop(Duration::from_secs(1)).await.unwrap();
    assert!(c.gates.lock().await["stop"] >= stop + Duration::from_secs(1));
    let calls = f.server.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        4,
        "cancelled future must not issue its deferred start"
    );
    assert_eq!(
        calls.iter().filter(|c| c.uri.ends_with("/start")).count(),
        2
    );
}

#[test]
fn identifiers_and_explicit_grace_keep_their_boundary_values() {
    for (app, id, token) in [
        (" ", "id", SECRET),
        ("app", "\t", SECRET),
        ("app", "id", " "),
        ("..", "id", SECRET),
    ] {
        assert!(
            Http::new(
                &CloudConfig {
                    app: app.into(),
                    machine_id: id.into(),
                    ..Default::default()
                },
                token.into()
            )
            .is_err()
        );
    }
    assert_eq!(drain_timeout(Duration::from_secs(45)), "45s");
    assert_eq!(drain_timeout(Duration::from_millis(500)), "0.5s");
    assert_eq!(drain_timeout(Duration::from_nanos(1)), "0.000000001s");
}

#[test]
fn cloud_interface_stays_reversible_and_deployment_restart_spelling_stays_distinct() {
    let source = include_str!("cloud.rs");
    let interface = source
        .split_once("pub trait Cloud:")
        .unwrap()
        .1
        .split_once("pub struct Http")
        .unwrap()
        .0;
    let verbs: std::collections::BTreeSet<_> = interface
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn "))
        .map(|line| line.split('(').next().unwrap())
        .collect();
    assert_eq!(
        verbs,
        ["start", "state", "stop", "wait"].into_iter().collect()
    );
    let public_inherent: Vec<_> = source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("pub fn ")
                .or_else(|| line.trim().strip_prefix("pub async fn "))
        })
        .map(|line| line.split('(').next().unwrap())
        .collect();
    assert_eq!(
        public_inherent,
        ["new"],
        "cloud HTTP must not expose extra mutation entrypoints"
    );

    // These are different provider surfaces; this client never sets either policy.
    let toml = include_str!("../../../../../deploy/fly/node/fly.toml");
    assert!(toml.contains("policy  = \"on-failure\"") || toml.contains("policy = \"on-failure\""));
    assert!(
        toml.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .any(|word| word == "on-fail")
    );
}
