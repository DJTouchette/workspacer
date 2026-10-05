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

/// The editor, explorer and per-agent terminal against the real embedded
/// backend: a hub-owned shell in the agent's folder, re-attached with its
/// output, kept out of the session list; compare-and-write saves; listings
/// with and without git-ignored entries. No model provider is launched.
#[cfg(unix)]
#[tokio::test]
async fn editor_explorer_and_agent_terminal_round_trip_through_the_owned_hub() {
    use std::sync::Arc;
    use wks_native::{
        controller::{Command, View},
        features::Request,
        host::{Mode, NativeHost, RustOptions},
        terminal::{self, Status},
    };
    let root = std::env::temp_dir().join(format!(
        "wks-native-editor-{}-{}",
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
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/lib.rs"), "fn one() {}\n").unwrap();
    std::fs::write(project.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(project.join("target")).unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&project)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q"]);
    let project_path = std::fs::canonicalize(&project).unwrap();
    let project_str = project_path.to_string_lossy().into_owned();

    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    async fn wait(
        views: &mut tokio::sync::watch::Receiver<Arc<View>>,
        what: &str,
        mut done: impl FnMut(&View) -> bool,
    ) -> Arc<View> {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let view = views.borrow_and_update().clone();
                if done(&view) {
                    return view;
                }
                views.changed().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }
    wait(&mut views, "connection", |v| v.connected).await;

    // Explorer listings: ignored entries only on request; `.git` never.
    let list = |include_ignored| {
        controller
            .command(Command::Request(Request::ListDir {
                path: project_str.clone(),
                include_ignored,
            }))
            .unwrap()
    };
    let names = |view: &View| -> Vec<String> {
        view.requests["file-tree"].value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_owned())
            .collect()
    };
    list(false);
    let view = wait(&mut views, "listing", |v| {
        v.requests.get("file-tree").is_some_and(|s| !s.loading)
    })
    .await;
    assert_eq!(view.requests["file-tree"].error, None);
    // Folders first, then files; `target` is git-ignored and `.git` omitted.
    assert_eq!(names(&view), ["src", ".gitignore"]);
    let first = view.requests["file-tree"].number;
    list(true);
    let view = wait(&mut views, "listing with ignored", |v| {
        v.requests
            .get("file-tree")
            .is_some_and(|s| !s.loading && s.number > first)
    })
    .await;
    let all = names(&view);
    assert!(all.contains(&"target".to_owned()) && !all.contains(&".git".to_owned()));
    assert_eq!(view.requests["file-tree"].value["includeIgnored"], true);

    // Saves compare against what the editor loaded, then verify the write.
    let file = project_path
        .join("src/lib.rs")
        .to_string_lossy()
        .into_owned();
    let save = |contents: &str, base: &str, force: bool| {
        controller
            .command(Command::Request(Request::SaveFile {
                session: String::new(),
                path: file.clone(),
                contents: contents.into(),
                base: base.into(),
                force,
            }))
            .unwrap()
    };
    let mut last = 0;
    let mut saved = async |views: &mut tokio::sync::watch::Receiver<Arc<View>>| {
        let view = wait(views, "save", |v| {
            v.requests
                .get("file-save")
                .is_some_and(|s| !s.loading && s.number > last)
        })
        .await;
        last = view.requests["file-save"].number;
        let state = view.requests["file-save"].clone();
        assert_eq!(state.error, None);
        (*state.value).clone()
    };
    save("fn two() {}\n", "fn one() {}\n", false);
    let result = saved(&mut views).await;
    assert_eq!(result["saved"], true);
    assert_eq!(
        std::fs::read_to_string(project.join("src/lib.rs")).unwrap(),
        "fn two() {}\n"
    );
    // Someone else wrote meanwhile: nothing is written, the conflict says why.
    std::fs::write(project.join("src/lib.rs"), "fn theirs() {}\n").unwrap();
    save("fn mine() {}\n", "fn two() {}\n", false);
    let result = saved(&mut views).await;
    assert_eq!(result["conflict"], "changed");
    assert_eq!(result["current"], "fn theirs() {}\n");
    assert_eq!(
        std::fs::read_to_string(project.join("src/lib.rs")).unwrap(),
        "fn theirs() {}\n"
    );
    save("fn mine() {}\n", "fn two() {}\n", true);
    assert_eq!(saved(&mut views).await["saved"], true);
    assert_eq!(
        std::fs::read_to_string(project.join("src/lib.rs")).unwrap(),
        "fn mine() {}\n"
    );

    // The agent's terminal: a real shell in its folder.
    let agent = "agent-under-test".to_owned();
    controller
        .command(Command::Terminal(terminal::Command::Open {
            agent: agent.clone(),
            cwd: project_str.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let view = wait(&mut views, "live terminal", |v| {
        v.terminals
            .get(&agent)
            .is_some_and(|t| t.status == Status::Live)
            || v.terminals
                .get(&agent)
                .is_some_and(|t| t.status == Status::Failed)
    })
    .await;
    let state = view.terminals[&agent].clone();
    assert_eq!(state.status, Status::Live, "{:?}", state.error);
    let shell = state.shell.clone().unwrap();
    let feed = view.terminal_feed.clone();
    let mut screen = terminal::Emulator::new(30, 100);
    let read_until = async |screen: &mut terminal::Emulator,
                            views: &mut tokio::sync::watch::Receiver<Arc<View>>,
                            needle: &str| {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(chunk) = feed.take(&shell) {
                    if chunk.reset {
                        *screen = terminal::Emulator::new(30, 100);
                    }
                    screen.process(&chunk.bytes);
                }
                if screen.text().contains(needle) {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_millis(200), views.changed()).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("terminal never showed {needle:?}:\n{}", screen.text()))
    };
    let input = |text: &str| {
        controller
            .command(Command::Terminal(terminal::Command::Input {
                agent: agent.clone(),
                bytes: text.as_bytes().to_vec(),
            }))
            .unwrap()
    };
    input("echo wks-$((40+2)); pwd\r");
    read_until(&mut screen, &mut views, "wks-42").await;
    read_until(&mut screen, &mut views, &project_str).await;

    // The shell is not an agent: it never appears in the session list.
    controller.command(Command::Refresh).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let view = views.borrow().clone();
    assert!(!view.sessions.iter().any(|s| s.id == shell));

    // Hidden and reopened: the same shell, its earlier output replayed.
    controller
        .command(Command::Terminal(terminal::Command::Hide {
            agent: agent.clone(),
        }))
        .unwrap();
    wait(&mut views, "detached", |v| {
        v.terminals[&agent].status == Status::Detached
    })
    .await;
    controller
        .command(Command::Terminal(terminal::Command::Open {
            agent: agent.clone(),
            cwd: project_str.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let view = wait(&mut views, "re-attached", |v| {
        v.terminals[&agent].status == Status::Live && v.terminals[&agent].attach > state.attach
    })
    .await;
    assert_eq!(view.terminals[&agent].shell.as_ref(), Some(&shell));
    let mut replay = terminal::Emulator::new(30, 100);
    read_until(&mut replay, &mut views, "wks-42").await;

    // A second agent owns an independent persistent shell in its own cwd.
    let second_cwd = root.join("second-project");
    std::fs::create_dir(&second_cwd).unwrap();
    let second_cwd = std::fs::canonicalize(second_cwd)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    controller
        .command(Command::Terminal(terminal::Command::Open {
            agent: "second-agent".into(),
            cwd: second_cwd.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let second_view = wait(&mut views, "second shell", |v| {
        v.terminals
            .get("second-agent")
            .is_some_and(|t| t.status == Status::Live)
    })
    .await;
    let second_shell = second_view.terminals["second-agent"].shell.clone().unwrap();
    assert_ne!(second_shell, shell);
    controller
        .command(Command::Terminal(terminal::Command::Input {
            agent: "second-agent".into(),
            bytes: b"echo second-$((21*2)); pwd\r".to_vec(),
        }))
        .unwrap();
    let mut second_screen = terminal::Emulator::new(30, 100);
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(chunk) = feed.take(&second_shell) {
                second_screen.process(&chunk.bytes);
            }
            if second_screen.text().contains("second-42")
                && second_screen.text().contains(&second_cwd)
            {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_millis(100), views.changed()).await;
        }
    })
    .await
    .unwrap();
    assert!(!second_screen.text().contains("wks-42"));
    assert_eq!(
        views.borrow().terminals[&agent].shell.as_ref(),
        Some(&shell)
    );

    // Restart ends that shell and starts another in the same folder.
    controller
        .command(Command::Terminal(terminal::Command::Restart {
            agent: agent.clone(),
            cwd: project_str.clone(),
            cols: 100,
            rows: 30,
        }))
        .unwrap();
    let view = wait(&mut views, "restarted", |v| {
        v.terminals[&agent].status == Status::Live
            && v.terminals[&agent]
                .shell
                .as_ref()
                .is_some_and(|s| s != &shell)
    })
    .await;
    assert_ne!(view.terminals[&agent].shell.as_ref(), Some(&shell));

    drop(views);
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
