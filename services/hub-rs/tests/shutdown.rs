//! Separate integration-test process: intentionally poisons the restart lease.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use workspacer_hub::{Hub, Options, protocol::Frame};

#[tokio::test]
async fn unconfirmed_blocking_shutdown_is_reported_and_prevents_replacement() {
    let (started, mut began) = tokio::sync::mpsc::channel(1);
    let finished = Arc::new(AtomicBool::new(false));
    let completion = finished.clone();
    let options = Options::default().handler("fixture.block", move |_, _| {
        let started = started.clone();
        let finished = completion.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                started.blocking_send(()).unwrap();
                std::thread::sleep(Duration::from_secs(3));
                finished.store(true, Ordering::Release);
                serde_json::json!(true)
            })
            .await
            .map_err(Into::into)
        }
    });
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let connection = hub.handle().connect().await.unwrap();
    connection
        .send(Frame {
            id: "one".into(),
            method: "fixture.block".into(),
            ..Frame::op("call")
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), began.recv())
        .await
        .unwrap();
    assert!(
        hub.shutdown()
            .unwrap_err()
            .to_string()
            .contains("cleanup limit")
    );
    assert!(Hub::start(Options::default()).is_err());
    tokio::time::timeout(Duration::from_secs(3), async {
        while !finished.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
