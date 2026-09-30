//! Explicit real-backend cutover probe; normal TUI unit tests need no hub binary.
use super::*;
use crate::bus::{BusClient, BusEvent, TOPIC_BUS_HELLO};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn reserve_ports() -> [u16; 4] {
    let listeners: Vec<_> = (0..4)
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    std::array::from_fn(|i| listeners[i].local_addr().unwrap().port())
}
async fn start(
    binary: &Path,
    root: &Path,
    ports: [u16; 4],
    generation: usize,
) -> (
    Daemons,
    std::sync::mpsc::Receiver<std::io::Result<std::process::ExitStatus>>,
) {
    let output = root.join(format!("stdout-{generation}"));
    let errors = root.join(format!("stderr-{generation}"));
    let child = Command::new(binary)
        .args([
            "serve",
            "--json",
            "--no-claudemon-init",
            "--host",
            "127.0.0.1",
        ])
        .args([
            "--hub-port",
            &ports[0].to_string(),
            "--mcp-port",
            &ports[1].to_string(),
            "--claudemon-api-port",
            &ports[2].to_string(),
            "--claudemon-hook-port",
            &ports[3].to_string(),
        ])
        .arg("--config-dir")
        .arg(root.join("config"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .arg("--home-dir")
        .arg(root.join("home"))
        .arg("--claudemon-db-path")
        .arg(root.join("sessions.db"))
        .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
        .env("WORKSPACER_USAGE_POLL_ON_BOOT", "0")
        .env("HUB_TOKEN", "tui-backend-fixture")
        .env_remove("WORKSPACER_ALLOW_NEW_TOKEN")
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg"))
        .env("APPDATA", root.join("xdg"))
        .stdin(Stdio::piped())
        .stdout(std::fs::File::create(&output).unwrap())
        .stderr(std::fs::File::create(&errors).unwrap())
        .spawn()
        .expect("start supplied Rust backend");
    let (tx, rx) = std::sync::mpsc::channel();
    let mut owner = Daemons::none();
    owner.rust_backend = Some(child);
    owner.exit_receipt = Some(tx);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let output = std::fs::read_to_string(&output).unwrap();
            if let Some(banner) = output
                .lines()
                .find_map(|line| serde_json::from_str::<Value>(line).ok())
            {
                assert_eq!(banner["service"], "workspacer-rust");
                return;
            }
            assert!(
                owner
                    .rust_backend
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_none(),
                "backend exited before readiness: {}",
                std::fs::read_to_string(&errors).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Rust backend readiness deadline");
    (owner, rx)
}
async fn call(bus: &BusClient, method: &str, params: Value) -> Value {
    tokio::time::timeout(Duration::from_secs(5), bus.call(method, params))
        .await
        .expect("TUI call deadline")
        .unwrap()
}
async fn event(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<BusEvent>,
    topic: &str,
) -> BusEvent {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let event = events.recv().await.expect("TUI event owner closed");
            if event.topic == topic {
                return event;
            }
        }
    })
    .await
    .expect("TUI event deadline")
}
async fn hook(port: u16, root: &Path, kind: &str) {
    let body =
        json!({"hook_event_name":kind,"session_id":"tui-fixture-session","cwd":root}).to_string();
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!("POST /hook HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response)
    );
}
async fn stop(
    owner: Daemons,
    receipt: std::sync::mpsc::Receiver<std::io::Result<std::process::ExitStatus>>,
    ports: [u16; 4],
) {
    tokio::task::spawn_blocking(move || drop(owner))
        .await
        .unwrap();
    let status = receipt
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(
        status.success(),
        "owned backend did not exit cleanly: {status}"
    );
    for port in ports {
        assert!(
            !port_open(port),
            "owned listener still open after wait: 127.0.0.1:{port}"
        );
    }
    eprintln!("TUI owned backend exit={status}; closed ports={ports:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit real Rust backend probe: make test-tui-rust-backend WKS_RUST_BACKEND_BIN=/absolute/binary"]
async fn real_rust_backend_calls_events_reconnect_and_owned_shutdown() {
    let binary = PathBuf::from(
        std::env::var_os("WKS_RUST_BACKEND_BIN")
            .expect("explicit probe requires WKS_RUST_BACKEND_BIN"),
    );
    assert!(
        binary.is_absolute() && binary.is_file(),
        "supply an existing absolute Rust backend binary"
    );
    let root = Scratch(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-config")
            .join(format!("rust-backend-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir_all(root.0.join("home")).unwrap();
    std::fs::create_dir_all(root.0.join("config")).unwrap();
    std::fs::write(
        root.0.join("config/config.yaml"),
        "agents:\n  checkProviderOnStartup: false\nusage:\n  pollOnBoot: false\n",
    )
    .unwrap();
    let ports = reserve_ports();
    let (owner, receipt) = start(&binary, &root.0, ports, 1).await;
    let (bus, mut events) = BusClient::connect(
        format!("ws://127.0.0.1:{}/bus", ports[0]),
        Some("tui-backend-fixture".into()),
    );
    assert_eq!(
        event(&mut events, TOPIC_BUS_HELLO).await.data["scope"],
        "operator"
    );
    bus.subscribe(vec!["agent.snapshot".into(), "layout.changed".into()])
        .unwrap();
    assert_eq!(call(&bus, "sessions.snapshots", json!({})).await, json!([]));
    // A second bootstrap owner sees the already-listening Rust bus and must
    // borrow it. Dropping that empty owner cannot stop the external service.
    let mut borrowed = Daemons::none();
    ensure_rust_backend(
        &mut borrowed,
        &format!("ws://127.0.0.1:{}/bus", ports[0]),
        &format!("http://127.0.0.1:{}", ports[2]),
    );
    assert!(borrowed.rust_backend.is_none() && borrowed.children.is_empty());
    drop(borrowed);
    assert_eq!(call(&bus, "sessions.snapshots", json!({})).await, json!([]));
    // Hook-only fixture creates daemon observations, never launches a model.
    hook(ports[3], &root.0, "SessionStart").await;
    hook(ports[3], &root.0, "Stop").await;
    let snapshot = event(&mut events, "agent.snapshot").await;
    assert_eq!(snapshot.data["sessionId"], "tui-fixture-session");
    assert!(snapshot.hub.is_none());
    let rows = call(&bus, "sessions.snapshots", json!({})).await;
    assert!(rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["sessionId"] == "tui-fixture-session"));
    assert!(call(
        &bus,
        "sessions.conversation",
        json!({"sessionId":"tui-fixture-session"})
    )
    .await
    .is_object());
    stop(owner, receipt, ports).await;
    let (owner, receipt) = start(&binary, &root.0, ports, 2).await;
    assert_eq!(
        event(&mut events, TOPIC_BUS_HELLO).await.data["scope"],
        "operator"
    );
    // Same BusClient; no second subscribe call. The unique marker distinguishes
    // replayed old events from the new broker generation's subscription.
    let marker = uuid::Uuid::new_v4().to_string();
    call(
        &bus,
        "layout.set",
        json!({"data":{"tuiReconnectMarker":marker}}),
    )
    .await;
    let delivered = event(&mut events, "layout.changed").await;
    assert_eq!(delivered.data["data"]["tuiReconnectMarker"], marker);
    assert!(call(&bus, "sessions.snapshots", json!({})).await.is_array());
    drop(bus);
    drop(events);
    stop(owner, receipt, ports).await;
}
