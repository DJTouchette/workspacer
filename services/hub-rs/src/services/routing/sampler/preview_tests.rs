use super::*;
use crate::{Hub, Options, auth, client::Client};
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn saved_owner_preferences_reach_select_preview_and_reopened_service() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let viewer = auth::mint(&tokens, auth::Scope::View, "preferences viewer").unwrap();
    let operator = auth::mint(&tokens, auth::Scope::Operator, "scoped operator").unwrap();
    let sink = Hub::start(Options::default()).unwrap();
    sink.ready().await.unwrap();
    let routing = Arc::new(RoutingService::open(root.path().into()).unwrap());
    let mut options = Options::default();
    options.control_plane_only = true;
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(install(options, routing, None, sink.handle())).unwrap();
    hub.ready().await.unwrap();
    let owner = Client::connect(&hub.handle()).await.unwrap();
    let view = owner
        .call("routing.preferences.get", json!({}))
        .await
        .unwrap();
    assert_eq!(view["configurable"], true);
    let request = json!({"baseRevision":view["revision"],"patch":{"activeProfile":"codex_only","roles":{"scout":"cheap"}}});
    for (token, can_read) in [(viewer.token, false), (operator.token, true)] {
        let unprivileged = Client::from_connection(
            hub.handle()
                .connect_authenticated(token, false)
                .await
                .unwrap(),
        );
        let view = unprivileged
            .call("routing.preferences.get", json!({}))
            .await;
        if can_read {
            assert_eq!(view.unwrap()["configurable"], false);
        } else {
            assert!(view.unwrap_err().to_string().contains("not authorized"));
            assert!(
                unprivileged
                    .call("routing.preview", json!({"role":"scout"}))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not authorized")
            );
        }
        for method in [
            "routing.preferences.validate",
            "routing.preferences.save",
            "routing.preferences.reset",
        ] {
            let params = if method.ends_with("reset") {
                json!({"baseRevision":request["baseRevision"]})
            } else {
                request.clone()
            };
            let error = unprivileged
                .call(method, params)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(if can_read {
                    "requires authenticated host authority"
                } else {
                    "not authorized"
                }),
                "{method}: {error}"
            );
        }
        unprivileged.close();
    }
    assert_eq!(
        owner
            .call("routing.preferences.save", request)
            .await
            .unwrap()["status"],
        "applied"
    );
    let selected = owner
        .call("routing.select", json!({"role":"scout"}))
        .await
        .unwrap();
    assert_eq!(selected["model"], "gpt-5.6-luna");
    assert_eq!(selected["capability"], "cheap");
    assert!(
        selected["reason"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("still owes routing.yaml a catalog check"))),
        "an unanswered catalog must remain visible in the actual decision"
    );
    let path = root.path().join("routing-decisions.jsonl");
    let before = std::fs::read(&path).unwrap();
    let preview = owner
        .call("routing.preview", json!({"role":"scout"}))
        .await
        .unwrap();
    for key in ["model", "effort", "capability"] {
        // The preview DTO always emits text; the private decision omits an
        // unspecified effort. Both represent the same empty effort selection.
        assert_eq!(word(&preview[key]), word(&selected[key]));
    }
    assert_eq!(std::fs::read(path).unwrap(), before);
    let reopened = RoutingService::open(root.path().into()).unwrap();
    let next = reopened
        .select(
            json!({"role":"scout"}),
            &json!({"providers":[]}),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(next["model"], selected["model"]);
    owner.close();
    hub.shutdown().unwrap();
    sink.shutdown().unwrap();
}

#[test]
fn wire_preview_rejects_duplicates_and_null_before_value_decoding() {
    for method in ["routing.preview", "hub:fixture/routing.preview"] {
        for params in [
            r#"{"role":"scout","role":"implementer"}"#,
            r#"{"role":"scout","cwd":null}"#,
            "null",
        ] {
            let wire =
                format!(r#"{{"op":"call","id":"fixture","method":"{method}","params":{params}}}"#);
            assert!(
                crate::protocol::Frame::decode(wire.as_bytes()).is_err(),
                "{wire}"
            );
        }
        let wire = format!(
            r#"{{"op":"call","id":"fixture","method":"{method}","params":{{"role":"scout"}}}}"#
        );
        assert!(crate::protocol::Frame::decode(wire.as_bytes()).is_ok());
    }
}

#[tokio::test]
async fn operator_preview_reads_fresh_usage_redacts_private_policy_and_never_probes_catalog() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("PRIVATE_ROUTING_CEILING");
    std::fs::create_dir(&project).unwrap();
    let mut patch = json!({"ceilings":{}});
    patch["ceilings"][project.to_str().unwrap()] = json!({"max_capability":"cheap"});
    std::fs::write(
        root.path().join("routing.yaml"),
        serde_yaml::to_string(&patch).unwrap(),
    )
    .unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let paths = Arc::new(Mutex::new(Vec::<String>::new()));
    let fail = Arc::new(AtomicBool::new(false));
    let (count, requests, failing) = (reads.clone(), paths.clone(), fail.clone());
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let (count, requests, failing) = (count.clone(), requests.clone(), failing.clone());
        async move {
            requests.lock().unwrap().push(request.uri().path().into());
            count.fetch_add(1, Ordering::SeqCst);
            if failing.load(Ordering::SeqCst) {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(json!({})),
                )
            } else {
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"providers":[]})),
                )
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let external = crate::services::external_claudemon::ExternalDaemon::new(&format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let catalogs = Arc::new(AtomicUsize::new(0));
    let probes = catalogs.clone();
    let sink = Hub::start(
        Options::default().handler("claude.listModels", move |_, _| {
            let probes = probes.clone();
            async move {
                probes.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"aliases":[{"value":"sonnet"}]}))
            }
        }),
    )
    .unwrap();
    sink.ready().await.unwrap();
    let sink_client = Client::connect(&sink.handle()).await.unwrap();
    sink_client
        .call("claude.listModels", json!({}))
        .await
        .unwrap();
    assert_eq!(
        catalogs.swap(0, Ordering::SeqCst),
        1,
        "catalog trap execution floor"
    );
    let routing = Arc::new(RoutingService::open(root.path().into()).unwrap());
    routing.log_decision(&json!({"decisionId":"fixture-floor"}));
    let log = root.path().join("routing-decisions.jsonl");
    let baseline = std::fs::read(&log).unwrap();
    let tokens = root.path().join("tokens.json");
    let operator = auth::mint(&tokens, auth::Scope::Operator, "preview fixture").unwrap();
    let mut options = Options::default();
    options.control_plane_only = true;
    options.scoped_tokens = Some(tokens);
    options.external_claudemon = Some(external.clone());
    let mut options = install(options, routing.clone(), None, sink.handle());
    options.external_claudemon = None;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::from_connection(
        hub.handle()
            .connect_authenticated(operator.token, false)
            .await
            .unwrap(),
    );
    let params = json!({"role":"implementer","cwd":project,"account":"PRIVATE_ACCOUNT","ticketId":"PRIVATE_TICKET"});
    for expected in 1..=2 {
        let before = chrono::Utc::now().timestamp_millis();
        let preview = client
            .call("routing.preview", params.clone())
            .await
            .unwrap();
        assert_eq!(
            reads.load(Ordering::SeqCst),
            expected,
            "preview reused completed usage cache"
        );
        assert_eq!(preview["usageState"], "live-at-time");
        assert_eq!(preview["capped"], true);
        assert_eq!(preview["capability"], "cheap");
        assert!(
            (before..=chrono::Utc::now().timestamp_millis())
                .contains(&preview["observedAt"].as_i64().unwrap())
        );
        assert!(!preview.to_string().contains("PRIVATE_"), "{preview}");
        for field in [
            "decisionId",
            "decidedAt",
            "ceiling",
            "matrix",
            "capacity",
            "demand",
            "ticketId",
        ] {
            assert!(
                preview.get(field).is_none(),
                "private field {field}: {preview}"
            );
        }
    }
    for invalid in [
        json!({}),
        json!({"role":"scout","surprise":true}),
        json!({"role":"scout","cwd":null}),
        json!({"role":"scout","expectedWork":[{"count":1.5}]}),
        json!({"role":"scout","requireIndependentFamily":"yes"}),
    ] {
        assert!(client.call("routing.preview", invalid).await.is_err());
    }
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "invalid preview must fail before upstream I/O"
    );
    fail.store(true, Ordering::SeqCst);
    let unknown = client.call("routing.preview", params).await.unwrap();
    assert_eq!(unknown["usageState"], "unknown");
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    assert_eq!(catalogs.load(Ordering::SeqCst), 0);
    assert!(
        paths
            .lock()
            .unwrap()
            .iter()
            .all(|path| path == "/usage/report")
    );
    assert_eq!(std::fs::read(log).unwrap(), baseline);
    assert_eq!(routing.catalog(), json!({}));
    client.close();
    sink_client.close();
    hub.shutdown().unwrap();
    sink.shutdown().unwrap();
    external.close();
    stop.send(()).unwrap();
    server.await.unwrap();
}
