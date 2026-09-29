#![cfg(feature = "rust-hub")]
use serde_json::json;
use std::time::Duration;
use wks_native::{backend::Backend, bus::Event};

#[tokio::test]
async fn owned_rust_host_survives_viewer_drop_and_joins_every_reported_listener() {
    use wks_native::host::{
        Mode, NativeHost, RustOptions, Status, verify_owned_listeners_released,
    };
    let root = std::env::temp_dir().join(format!(
        "wks-native-owned-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.clone()).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    let status = host.status();
    let ready = tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.bus_url, "in-process");
    assert_eq!(ready.owned_listeners.len(), 4);
    assert!(ready.engine_api.is_some() && ready.engine_hook.is_some());
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    drop(views);
    drop(controller);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(
        matches!(*status.borrow(), Status::Ready(_)),
        "dropping a viewer must not stop its owner"
    );
    host.shutdown().await.unwrap();
    assert!(matches!(*status.borrow(), Status::Stopped));
    verify_owned_listeners_released(&ready.owned_listeners)
        .await
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_backend_uses_in_memory_calls_and_events() {
    let options = workspacer_hub::Options::default()
        .handler("sessions.snapshots", |_, _| async { Ok(json!([])) });
    let hub = workspacer_hub::Hub::start(options).unwrap();
    assert!(hub.ready().await.unwrap().is_none());
    let (backend, events) = Backend::in_process(&hub.handle()).await.unwrap();
    assert!(matches!(events.recv().await.unwrap(), Event::Connected));
    assert_eq!(backend.snapshots().await.unwrap(), json!([]));
    backend
        .topics(["agent.snapshot".into()].into())
        .await
        .unwrap();
    backend.snapshots().await.unwrap(); // actor barrier after subscription
    hub.handle()
        .publish(workspacer_hub::protocol::Event::new(
            "agent.snapshot",
            "fixture",
            json!({"sessionId":"one"}),
        ))
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event,Event::Data {topic,data,..} if topic=="agent.snapshot" && data["sessionId"]=="one")
    );
    hub.shutdown().unwrap();
    assert!(backend.snapshots().await.is_err());
}

#[tokio::test]
async fn embedded_power_pause_keeps_owner_alive_without_reconnecting() {
    use futures_util::future::BoxFuture;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct FakePower(Arc<AtomicUsize>);
    impl workspacer_hub::services::machine_power::PowerProvider for FakePower {
        fn check(&self) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn stop(&self) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }
    let stopped = Arc::new(AtomicUsize::new(0));
    let mut options = workspacer_hub::Options::default();
    options.machine_power_provider = Some(Arc::new(FakePower(stopped.clone())));
    let hub = workspacer_hub::Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let (backend, events) = Backend::in_process(&hub.handle()).await.unwrap();
    assert!(matches!(events.recv().await.unwrap(), Event::Connected));
    assert_eq!(
        backend.call("machine.stop", json!({})).await.unwrap()["accepted"],
        true
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(4), events.recv())
            .await
            .unwrap()
            .unwrap(),
        Event::PowerPaused
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        while stopped.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!backend.can_resume_power_pause());
    assert!(backend.resume_power_pause().is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "paused view channel closed and implicitly stopped its owner"
    );
    assert!(matches!(
        *hub.handle().status().borrow(),
        workspacer_hub::Status::Ready { .. }
    ));
    drop(backend);
    drop(events);
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}
