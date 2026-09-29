use super::*;
use crate::{Hub, Options, auth, client::Client, protocol::Event};
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn view_usage_report_joins_twelve_readers_preserves_uncertainty_and_has_no_decision_side_effects()
 {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let viewer = auth::mint(&tokens, auth::Scope::View, "overview fixture").unwrap();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let failing = Arc::new(AtomicBool::new(false));
    let observed = requests.clone();
    let failure = failing.clone();
    let report = json!({"generated_at":chrono::Utc::now().timestamp(),"providers":[{
        "provider":"claude","accounts":[{"account":"fixture","fresh":true,"windows":{
            "five_hour":{"used_percent":{"state":"unknown"},"resets_at":null,"window_minutes":300}
        }}]
    }]});
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let (observed, failing, report) = (observed.clone(), failure.clone(), report.clone());
        async move {
            observed.lock().unwrap().push(request.uri().path().into());
            if failing.load(Ordering::SeqCst) {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(json!({})),
                )
            } else {
                assert_eq!(request.uri().path(), "/usage/report");
                (axum::http::StatusCode::OK, axum::Json(report))
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
    let catalog_count = catalogs.clone();
    let sink = Hub::start(
        Options::default().handler("claude.listModels", move |_, _| {
            let count = catalog_count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"aliases":[],"seen":[]}))
            }
        }),
    )
    .unwrap();
    sink.ready().await.unwrap();
    let watcher = Client::connect(&sink.handle()).await.unwrap();
    watcher.call("claude.listModels", json!({})).await.unwrap();
    assert_eq!(
        catalogs.load(Ordering::SeqCst),
        1,
        "catalog observer must be live"
    );
    catalogs.store(0, Ordering::SeqCst);
    let mut events = watcher.events();
    watcher
        .topics(["routing.decision".into()].into())
        .await
        .unwrap();
    let routing = Arc::new(RoutingService::open(root.path().into()).unwrap());
    let matrix = routing.matrix();
    routing.log_decision(&json!({"decisionId":"fixture-log-floor"}));
    let log_path = root.path().join("routing-decisions.jsonl");
    let log_before = std::fs::read(&log_path).unwrap();
    let make_hub = || {
        let mut options = Options::default();
        options.control_plane_only = true;
        options.scoped_tokens = Some(tokens.clone());
        options.external_claudemon = Some(external.clone());
        let mut options = install(options, routing.clone(), None, sink.handle());
        // Test the real installed handlers and sampler in isolation from the
        // independent external-daemon SSE lifecycle adapter. The sampler has
        // captured this transport; production handler code is not duplicated.
        options.external_claudemon = None;
        Hub::start(options).unwrap()
    };
    let hub = make_hub();
    hub.ready().await.unwrap();
    let client = Client::from_connection(
        hub.handle()
            .connect_authenticated(viewer.token.clone(), false)
            .await
            .unwrap(),
    );
    let before = chrono::Utc::now().timestamp();
    let replies =
        futures_util::future::join_all((0..12).map(|_| client.call("usage.report", json!({}))))
            .await;
    let after = chrono::Utc::now().timestamp();
    for reply in replies {
        let value = reply.unwrap();
        assert!((before..=after).contains(&value["evaluated_at"].as_i64().unwrap()));
        assert_eq!(
            value["valid_until"].as_i64().unwrap() - value["evaluated_at"].as_i64().unwrap(),
            60
        );
        assert_eq!(
            value["providers"][0]["accounts"][0]["windows"]["five_hour"]["pace"]["known"],
            false
        );
        assert!(
            value["providers"][0]["accounts"][0]["windows"]["five_hour"]["resets_at"].is_null()
        );
        assert_eq!(
            value["providers"][0]["accounts"][0]["windows"]["five_hour"]["used_percent"]["state"],
            "unknown"
        );
        assert!(
            value["providers"][0]["accounts"][0]["windows"]["five_hour"]["used_percent"]
                .get("value")
                .is_none()
        );
    }
    assert_eq!(*requests.lock().unwrap(), vec!["/usage/report"]);
    for params in [
        json!({"url":"http://other"}),
        json!({"providers":[]}),
        json!([]),
        json!("invalid"),
    ] {
        assert!(
            client
                .call("usage.report", params)
                .await
                .unwrap_err()
                .to_string()
                .contains("no parameters accepted")
        );
    }
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(catalogs.load(Ordering::SeqCst), 0);
    assert_eq!(routing.matrix(), matrix);
    assert_eq!(std::fs::read(&log_path).unwrap(), log_before);
    sink.handle()
        .publish_wait(Event::new(
            "routing.decision",
            "fixture",
            json!({"sentinel":true}),
        ))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event.data,
        Some(json!({"sentinel":true})),
        "usage read published a routing decision"
    );

    failing.store(true, Ordering::SeqCst);
    // A fresh sampler must report transport failure, not an empty successful
    // projection. The first sampler may still use its within-policy cache.
    let failing_hub = make_hub();
    failing_hub.ready().await.unwrap();
    let failing_client = Client::from_connection(
        failing_hub
            .handle()
            .connect_authenticated(viewer.token, false)
            .await
            .unwrap(),
    );
    assert!(
        failing_client
            .call("usage.report", Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("usage.report unavailable")
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(client.call("usage.report", Value::Null).await.is_ok());
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "fresh cached observation needlessly refetched"
    );
    assert_eq!(catalogs.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&log_path).unwrap(), log_before);
    client.close();
    failing_client.close();
    watcher.close();
    failing_hub.shutdown().unwrap();
    hub.shutdown().unwrap();
    sink.shutdown().unwrap();
    external.close();
    stop.send(()).unwrap();
    server.await.unwrap();
}
