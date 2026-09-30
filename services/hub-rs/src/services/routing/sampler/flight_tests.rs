use super::*;
use std::sync::atomic::AtomicUsize;

struct Fixture {
    sampler: Arc<UsageSampler>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
    reads: Arc<AtomicUsize>,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
async fn fixture() -> Fixture {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let (notice, gate, count) = (entered.clone(), release.clone(), reads.clone());
    let app = axum::Router::new().route(
        "/usage/report",
        axum::routing::get(move || {
            let (notice, gate, count) = (notice.clone(), gate.clone(), count.clone());
            async move {
                let sample = count.fetch_add(1, Ordering::SeqCst) + 1;
                notice.notify_one();
                gate.acquire().await.unwrap().forget();
                axum::Json(json!({"providers":[],"fixtureSample":sample}))
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
    let mut sampler = UsageSampler::new(None);
    sampler.external = Some(external);
    Fixture {
        sampler: Arc::new(sampler),
        entered,
        release,
        reads,
        stop,
        server,
    }
}
async fn cache_published(sampler: &UsageSampler) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if sampler.cache.lock().unwrap().report.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned fetch did not publish after all waiters left");
}
async fn finish(f: Fixture) {
    f.sampler.external.as_ref().unwrap().close();
    f.stop.send(()).unwrap();
    f.server.await.unwrap();
}
#[tokio::test]
async fn cancelled_last_waiter_does_not_cancel_fetch_or_lose_late_cache_publication() {
    let f = fixture().await;
    let sampler = f.sampler.clone();
    let waiter = tokio::spawn(async move { sampler.report(Duration::ZERO).await });
    tokio::time::timeout(Duration::from_secs(3), f.entered.notified())
        .await
        .expect("upstream request was not entered");
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    f.release.add_permits(1);
    cache_published(&f.sampler).await;
    assert_eq!(
        f.sampler.report(Duration::from_secs(60)).await.unwrap()["fixtureSample"],
        1
    );
    assert_eq!(
        f.reads.load(Ordering::SeqCst),
        1,
        "late observation must not need a second fetch"
    );
    finish(f).await;
}
#[tokio::test]
async fn caller_three_second_budget_does_not_end_the_ten_second_shared_fetch() {
    assert_eq!(CALLER_WAIT, Duration::from_secs(3));
    assert_eq!(FETCH_TIMEOUT, Duration::from_secs(10));
    let f = fixture().await;
    let sampler = f.sampler.clone();
    let waiter = tokio::spawn(async move { sampler.report(Duration::ZERO).await });
    tokio::time::timeout(Duration::from_secs(3), f.entered.notified())
        .await
        .expect("upstream request was not entered");
    let error = waiter.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("caller budget"), "{error}");
    assert!(
        f.sampler.cache.lock().unwrap().flight.is_some(),
        "caller timeout killed the shared flight"
    );
    assert!(f.sampler.cache.lock().unwrap().report.is_none());
    f.release.add_permits(1);
    cache_published(&f.sampler).await;
    assert_eq!(
        f.sampler.report(Duration::from_secs(60)).await.unwrap()["fixtureSample"],
        1
    );
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    finish(f).await;
}
