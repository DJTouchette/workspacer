#![cfg(all(unix, feature = "rust-hub"))]
//! Settings' default terminal shell against the real owning hub: the choice
//! is saved in the hub's shared config (`terminal.shell`), offered only from
//! the shells the hub's host has, refused when it is not a login shell, and
//! is what the next agent terminal actually runs.
use std::{sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::Request,
    host::{Mode, NativeHost, RustOptions},
    terminal::{self, Status},
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
    .unwrap_or_else(|_| panic!("default shell fixture timed out: {phase}"))
}

/// Issue a request and wait for its own settled state.
async fn settle(controller: &Controller, request: Request) -> Arc<View> {
    let before = controller
        .views
        .borrow()
        .requests
        .get("terminal-shell")
        .map_or(0, |s| s.number);
    controller.command(Command::Request(request)).unwrap();
    until(controller, "terminal-shell request", |v| {
        v.requests
            .get("terminal-shell")
            .is_some_and(|s| s.number > before && !s.loading)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_shell_is_saved_on_the_hub_and_starts_the_next_terminal() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-default-shell-{}-{}",
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
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let project = std::fs::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let host = NativeHost::start(Mode::Rust(options.clone())).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    until(&controller, "connected", |v| v.connected).await;

    // Read: nothing configured yet, and the host's own shells are offered.
    let read = settle(&controller, Request::TerminalShell { set: None }).await;
    let state = &read.requests["terminal-shell"];
    assert!(state.error.is_none(), "{:?}", state.error);
    assert_eq!(state.value["shell"], "");
    assert!(state.value["listError"].is_null(), "{}", state.value);
    let choices = terminal::shell_choices(&state.value);
    assert_eq!(choices[0].path, "", "the hub default comes first");
    for choice in &choices[1..] {
        assert!(
            std::path::Path::new(&choice.path).is_file(),
            "offered a shell the host does not have: {choice:?}"
        );
    }

    // Save /bin/sh (always an allowed login shell), verified on readback.
    let saved = settle(
        &controller,
        Request::TerminalShell {
            set: Some("/bin/sh".into()),
        },
    )
    .await;
    let state = &saved.requests["terminal-shell"];
    assert!(state.error.is_none(), "{:?}", state.error);
    assert_eq!(state.value["shell"], "/bin/sh");

    // The next agent terminal runs it: `$0` is the argv[0] the hub spawned.
    let agent = "default-shell-agent".to_owned();
    controller
        .command(Command::Terminal(terminal::Command::Open {
            agent: agent.clone(),
            cwd: project.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let view = until(&controller, "live terminal", |v| {
        v.terminals
            .get(&agent)
            .is_some_and(|t| matches!(t.status, Status::Live | Status::Failed))
    })
    .await;
    let state = view.terminals[&agent].clone();
    assert_eq!(state.status, Status::Live, "{:?}", state.error);
    let shell = state.shell.clone().unwrap();
    let feed = view.terminal_feed.clone();
    controller
        .command(Command::Terminal(terminal::Command::Input {
            agent: agent.clone(),
            bytes: b"echo \"shell=[$0]\"\r".to_vec(),
        }))
        .unwrap();
    let mut screen = terminal::Emulator::new(30, 100);
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(chunk) = feed.take(&shell) {
                if chunk.reset {
                    screen = terminal::Emulator::new(30, 100);
                }
                screen.process(&chunk.bytes);
            }
            if screen.text().contains("shell=[/bin/sh]") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the terminal did not run /bin/sh:\n{}", screen.text()));

    // A program that is not one of the host's login shells is refused by the
    // hub when a terminal starts with it, never run.
    let sneaky = root.join("not-a-shell");
    std::fs::write(&sneaky, "#!/bin/sh\necho ran\n").unwrap();
    let saved = settle(
        &controller,
        Request::TerminalShell {
            set: Some(sneaky.to_string_lossy().into_owned()),
        },
    )
    .await;
    assert!(saved.requests["terminal-shell"].error.is_none());
    controller
        .command(Command::Terminal(terminal::Command::Restart {
            agent: agent.clone(),
            cwd: project.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let refused = until(&controller, "refused restart", |v| {
        v.terminals[&agent].status == Status::Failed
    })
    .await;
    let error = refused.terminals[&agent].error.clone().unwrap_or_default();
    assert!(error.contains("login shells"), "{error}");

    // The choice lives in the hub's shared config, so it outlasts the hub.
    let kept = settle(
        &controller,
        Request::TerminalShell {
            set: Some("/bin/sh".into()),
        },
    )
    .await;
    assert_eq!(kept.requests["terminal-shell"].value["shell"], "/bin/sh");
    host.shutdown().await.unwrap();
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    until(&controller, "reconnected", |v| v.connected).await;
    let reread = settle(&controller, Request::TerminalShell { set: None }).await;
    assert_eq!(reread.requests["terminal-shell"].value["shell"], "/bin/sh");
    host.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&root);
}
