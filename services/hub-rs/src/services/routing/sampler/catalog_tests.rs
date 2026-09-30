use super::*;
use std::sync::atomic::AtomicUsize;

#[test]
fn fallover_skips_models_and_efforts_refuted_by_an_answered_catalog_only() {
    let mut matrix = defaults();
    matrix["active_profile"] = json!("fixture");
    matrix["roles"]["scout"] = json!("cheap");
    matrix["profiles"]["fixture"] = json!({"cheap":{"provider":"codex","model":"missing","alternatives":[
        {"provider":"codex","model":"valid","effort":"impossible"},
        {"provider":"codex","model":"valid","effort":"high"}
    ]}});
    matrix["_catalog"]["codex"] =
        json!({"state":"available","models":[{"id":"valid","effortLevels":["high"]}]});
    let selected = select(
        &matrix,
        &json!({"role":"scout"}),
        &json!({"providers":[]}),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert_eq!(selected["model"], "valid");
    assert_eq!(selected["effort"], "high");
    assert_eq!(selected["fellOverFrom"]["model"], "missing");
    matrix["_catalog"]["codex"] = json!({"state":"unknown"});
    let unknown = select(
        &matrix,
        &json!({"role":"scout"}),
        &json!({"providers":[]}),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert_eq!(
        unknown["model"], "missing",
        "an unanswered catalog must not invent model unavailability"
    );
    matrix["_catalog"]["codex"] = json!({"state":"unavailable","models":[]});
    let empty = select(
        &matrix,
        &json!({"role":"scout"}),
        &json!({"providers":[]}),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert!(
        empty["reason"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("reported no launchable model"))),
        "measured-empty provider must explain the skipped candidates: {empty}"
    );
}

#[tokio::test]
async fn actual_catalog_producer_distinguishes_empty_malformed_and_positive_ttl() {
    let response = Arc::new(Mutex::new((
        axum::http::StatusCode::OK,
        json!({"models":[]}),
    )));
    let reads = Arc::new(AtomicUsize::new(0));
    let (reply, count) = (response.clone(), reads.clone());
    let app = axum::Router::new().route(
        "/providers/codex/models",
        axum::routing::get(move || {
            let (reply, count) = (reply.clone(), count.clone());
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                let (status, body) = reply.lock().unwrap().clone();
                (status, axum::Json(body))
            }
        }),
    );
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
    let root = tempfile::tempdir().unwrap();
    let service = Arc::new(RoutingService::open(root.path().into()).unwrap());
    service.state.lock().unwrap().matrix["profiles"] =
        json!({"fixture":{"cheap":{"provider":"codex","model":"fixture"}}});
    let mut sampler = UsageSampler::new(None);
    sampler.external = Some(external.clone());
    let sampler = Arc::new(sampler);
    assert!(service.catalog_pending());
    async fn refresh(sampler: &Arc<UsageSampler>, service: &Arc<RoutingService>) {
        *sampler.catalog_last.lock().unwrap() = None;
        sampler.refresh_catalog(service.clone());
        tokio::time::timeout(Duration::from_secs(3), async {
            while sampler.catalog_flight.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("catalog producer did not complete");
    }
    refresh(&sampler, &service).await;
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(service.catalog()["codex"]["state"], "unavailable");
    assert!(service.catalog_pending());
    *response.lock().unwrap() = (
        axum::http::StatusCode::OK,
        json!({"models":[{"id":"fixture","effortLevels":["high"],"secret":"discard"}]}),
    );
    refresh(&sampler, &service).await;
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(service.catalog()["codex"]["state"], "available");
    assert_eq!(
        service.catalog()["codex"]["models"],
        json!([{"id":"fixture","effortLevels":["high"]}])
    );
    assert!(!service.catalog_pending());
    refresh(&sampler, &service).await;
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "positive catalog must retain its ten-minute TTL"
    );
    service.state.lock().unwrap().catalog["codex"]["observedAt"] =
        json!(chrono::Utc::now().timestamp_millis() - 600_001);
    *response.lock().unwrap() = (
        axum::http::StatusCode::OK,
        json!({"models":[{"id":"valid"},{"id":42}]}),
    );
    refresh(&sampler, &service).await;
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    assert_eq!(service.catalog()["codex"]["state"], "unknown");
    assert!(service.catalog()["codex"].get("models").is_none());
    assert!(service.catalog_pending());
    *response.lock().unwrap() = (axum::http::StatusCode::BAD_GATEWAY, json!({"models":[]}));
    refresh(&sampler, &service).await;
    assert_eq!(reads.load(Ordering::SeqCst), 4);
    assert_eq!(
        service.catalog()["codex"]["state"],
        "unknown",
        "transport failure is not evidence of an installed CLI having zero models"
    );
    external.close();
    stop.send(()).unwrap();
    server.await.unwrap();
}
