#![cfg(unix)]
//! A cooperative TERM handler must not be killed merely because its log pipe is full.
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use workspacer_hub::plugins::supervisor::{Factory, NativeFactory, Spec};
const BYTES: usize = 2 * 1024 * 1024;
#[test]
fn graceful_stop_drains_shutdown_output() {
    const FIXTURE: &str = "WKS_SUPERVISOR_SHUTDOWN_FIXTURE";
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = std::path::PathBuf::from(root);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .unwrap();
                std::fs::write(root.join("ready"), "ready").unwrap();
                term.recv().await;
                std::io::stdout().write_all(&vec![b'x'; BYTES]).unwrap();
                std::io::stdout().flush().unwrap();
                std::fs::write(root.join("completed"), "graceful").unwrap();
            });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let bytes = Arc::new(AtomicUsize::new(0));
    let sink = bytes.clone();
    let mut child = NativeFactory
        .spawn(&Spec {
            command: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec![
                "--exact".into(),
                "graceful_stop_drains_shutdown_output".into(),
                "--nocapture".into(),
            ],
            directory: root.path().into(),
            env: [(FIXTURE.into(), root.path().to_string_lossy().into_owned())].into(),
            health_url: None,
            log: Some(Arc::new(move |stream, line| {
                if stream == "stdout" {
                    sink.fetch_add(
                        line.bytes().filter(|byte| *byte == b'x').count(),
                        Ordering::SeqCst,
                    );
                }
            })),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("ready").exists() {
        assert!(!child.exited().unwrap());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    child.stop().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("completed"))
            .ok()
            .as_deref(),
        Some("graceful"),
        "full log pipe prevented cooperative shutdown"
    );
    assert!(
        bytes.load(Ordering::SeqCst) >= BYTES,
        "shutdown output did not drain"
    );
}
