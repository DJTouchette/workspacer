#![cfg(feature = "rust-hub")]
//! Archive against a real, isolated Rust hub: the native client's archive is
//! the hub's shared document (what the web reads), it survives a restart of
//! the owning host, and restore removes it. No provider is started.
use std::{sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::{Request, archive_contains},
    host::{Mode, NativeHost, RustOptions},
};

async fn until(controller: &Controller, what: &str, check: impl Fn(&View) -> bool) -> Arc<View> {
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let view: Arc<View> = views.borrow_and_update().clone();
            if check(&view) {
                return view;
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} never happened"))
}

async fn start(root: &std::path::Path) -> (NativeHost, Controller) {
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir_all(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    until(&controller, "the shared archive read", |v| {
        v.connected && v.session_archive.is_some()
    })
    .await;
    (host, controller)
}

#[tokio::test]
async fn native_archive_is_the_hubs_shared_document_and_survives_restart() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-archive-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (host, controller) = start(&root).await;
    controller
        .command(Command::Request(Request::SetArchive {
            session: "native-archived".into(),
            archived: true,
        }))
        .unwrap();
    let view = until(&controller, "the archive receipt", |v| {
        !v.archive_receipts.is_empty()
    })
    .await;
    assert!(
        view.archive_receipts[0].error.is_none(),
        "{:?}",
        view.archive_receipts[0].error
    );
    assert!(archive_contains(
        view.session_archive.as_ref().unwrap(),
        "native-archived"
    ));
    // Persisted where every client of this hub (the web included) reads it.
    let disk = std::fs::read_to_string(root.join("state/hub/session-archive.json")).unwrap();
    assert!(disk.contains("native-archived"), "{disk}");
    drop(controller);
    host.shutdown().await.unwrap();

    // A restarted host — a reconnecting client — still hides it.
    let (host, controller) = start(&root).await;
    let view = controller.views.borrow().clone();
    assert!(archive_contains(
        view.session_archive.as_ref().unwrap(),
        "native-archived"
    ));
    controller
        .command(Command::Request(Request::SetArchive {
            session: "native-archived".into(),
            archived: false,
        }))
        .unwrap();
    let view = until(&controller, "the restore", |v| {
        v.session_archive
            .as_ref()
            .is_some_and(|d| !archive_contains(d, "native-archived"))
    })
    .await;
    assert!(view.archive_receipts.iter().all(|r| r.error.is_none()));
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
