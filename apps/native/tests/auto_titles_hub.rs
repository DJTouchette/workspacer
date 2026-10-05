#![cfg(all(unix, feature = "rust-hub"))]
//! Native launch → owning Rust hub → automatic title → native view, with the
//! real embedded engine and deterministic fake provider CLIs (a Claude stream
//! session and a `codex exec` titler). No model is called.
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, NewSession, View},
    features::Request,
    host::{Mode, NativeHost, RustOptions},
};

async fn until(
    controller: &Controller,
    phase: &str,
    mut done: impl FnMut(&View) -> bool,
) -> Arc<View> {
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view: Arc<View> = views.borrow_and_update().clone();
            if done(&view) {
                return view;
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("native auto-title fixture timed out: {phase}"))
}

fn fake(bin: &Path, name: &str, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    let python = std::process::Command::new("python3")
        .args(["-I", "-S", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = String::from_utf8(python.stdout).unwrap();
    let path = bin.join(name);
    std::fs::write(&path, format!("#!{}\n{source}", python.trim())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unnamed_native_launch_is_titled_by_the_hub_with_the_configured_model() {
    let dir = std::env::temp_dir().join(format!(
        "wks-native-auto-title-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.as_path();
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    fake(
        &bin,
        "claude",
        include_str!("../../../services/hub-rs/tests/fixtures/fake_claude_title_session.py"),
    );
    fake(
        &bin,
        "codex",
        include_str!("../../../services/hub-rs/tests/fixtures/fake_codex_titler.py"),
    );
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    // First start initializes the isolated hub state (and its token); the
    // fixture config is written while it is stopped. (JSON is valid YAML.)
    let config_file = options.config_dir.join("config.yaml");
    let first = NativeHost::start(Mode::Rust(options.clone())).unwrap();
    tokio::time::timeout(Duration::from_secs(30), first.ready())
        .await
        .unwrap()
        .unwrap();
    first.shutdown().await.unwrap();
    // Agent on Claude; every title pinned to Codex with an exact model.
    std::fs::write(
        &config_file,
        serde_json::to_string(&json!({
            "agents":{"binaries":{"claude":bin.join("claude"),"codex":bin.join("codex")},
                "autoTitle":{"provider":"codex","models":{"codex":"gpt-5.4-mini-fixture"}}},
            "claude":{"transport":"stream"}}))
        .unwrap(),
    )
    .unwrap();
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    until(&controller, "connected", |v| v.connected).await;

    controller
        .command(Command::Create(NewSession {
            provider: "claude".into(),
            cwd: project.display().to_string(),
            message: "please fix the login redirect loop on mobile".into(),
            ..Default::default()
        }))
        .unwrap();
    let created = until(&controller, "spawn receipt", |v| v.spawn_receipt.is_some()).await;
    let receipt = created.spawn_receipt.clone().unwrap();
    assert!(receipt.error.is_none(), "{:?}", receipt.error);
    let id = receipt.session.unwrap();
    let titled = until(&controller, "title in the native session list", |v| {
        v.sessions
            .iter()
            .any(|s| s.id == id && s.label == "Fix the login redirect loop")
    })
    .await;
    assert_eq!(titled.selected.as_deref(), Some(id.as_str()));
    let calls: Vec<Value> = std::fs::read_to_string(root.join("titler-calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|w| w == [json!("--model"), json!("gpt-5.4-mini-fixture")])
    );

    // History (the hub's sessions.recent) names it the same way.
    controller
        .command(Command::Request(Request::Recent))
        .unwrap();
    until(&controller, "history row", |v| {
        v.requests.get("recent").is_some_and(|s| {
            !s.loading
                && s.value.as_array().is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["sessionId"] == id.as_str()
                            && row["name"] == "Fix the login redirect loop"
                    })
                })
        })
    })
    .await;
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
