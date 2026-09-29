use clap::Parser;
use serde_json::{Value, json};
use workspacer_hub::{
    Hub, Options, Status,
    auth::{self, Scope},
    cli::{Command, CommandLine, execute, load_or_create_host_token, plan_serve},
};
#[test]
fn cli_preserves_flag_order_and_requires_explicit_database_for_alternate_ports() {
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "serve",
        "--claudemon-api-port",
        "9001",
        "--config-dir",
        "/tmp/fixture",
    ])
    .unwrap();
    let Command::Serve(serve) = &args.command else {
        panic!()
    };
    assert!(plan_serve(&args, serve).is_err());
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "serve",
        "--claudemon-api-port",
        "9001",
        "--claudemon-db-path",
        "/tmp/fixture.sqlite",
        "--config-dir",
        "/tmp/fixture",
    ])
    .unwrap();
    let Command::Serve(serve) = &args.command else {
        panic!()
    };
    assert_eq!(plan_serve(&args, serve).unwrap().api_port, 9001);
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "jobs",
        "enable",
        "job-prefix",
        "--hub-port",
        "9010",
    ])
    .unwrap();
    assert_eq!(args.hub_port, 9010);
    assert!(
        CommandLine::try_parse_from([
            "workspacer-rust",
            "token",
            "facade-authority",
            "--label",
            "service"
        ])
        .is_err()
    );
}
#[test]
fn host_identity_first_run_empty_directories_state_loss_and_concurrent_creation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("plugins")).unwrap();
    let token = load_or_create_host_token(root.path(), false).unwrap();
    assert_eq!(
        token,
        load_or_create_host_token(root.path(), false).unwrap()
    );
    std::fs::write(root.path().join("config.yaml"), "{}").unwrap();
    std::fs::remove_file(root.path().join("remote-token")).unwrap();
    assert!(
        load_or_create_host_token(root.path(), false)
            .unwrap_err()
            .to_string()
            .contains("STATE LOSS")
    );
    assert_ne!(token, load_or_create_host_token(root.path(), true).unwrap());
    let root = tempfile::tempdir().unwrap();
    let results = std::thread::scope(|scope| {
        let mut workers = vec![];
        for _ in 0..6 {
            workers.push(scope.spawn(|| load_or_create_host_token(root.path(), false).unwrap()));
        }
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert_eq!(results.len(), 1);
}
#[tokio::test]
async fn token_commands_redact_lists_and_reject_session_facade_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tokens.json");
    let path = path.to_str().unwrap();
    let mut out = vec![];
    let mut err = vec![];
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "token",
        "create",
        "--scope",
        "operator",
        "--label",
        "service",
        "--tokens-file",
        path,
    ])
    .unwrap();
    execute(&args, &mut out, &mut err).await.unwrap();
    let token = String::from_utf8(out.clone()).unwrap().trim().to_owned();
    assert!(!token.is_empty());
    out.clear();
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "token",
        "list",
        "--tokens-file",
        path,
        "--json",
    ])
    .unwrap();
    execute(&args, &mut out, &mut err).await.unwrap();
    assert!(!String::from_utf8(out.clone()).unwrap().contains(&token));
    out.clear();
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "token",
        "facade-authority",
        "--label",
        "service",
        "--enabled=true",
        "--tokens-file",
        path,
    ])
    .unwrap();
    execute(&args, &mut out, &mut err).await.unwrap();
    assert!(auth::load(std::path::Path::new(path)).unwrap()[0].facade_authority);
    auth::mint(
        std::path::Path::new(path),
        Scope::Operator,
        "session:worker",
    )
    .unwrap();
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "token",
        "facade-authority",
        "--label",
        "session:worker",
        "--enabled=true",
        "--tokens-file",
        path,
    ])
    .unwrap();
    assert!(execute(&args, &mut out, &mut err).await.is_err());
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "token",
        "revoke",
        &token,
        "--tokens-file",
        path,
    ])
    .unwrap();
    execute(&args, &mut out, &mut err).await.unwrap();
    assert!(
        auth::load(std::path::Path::new(path))
            .unwrap()
            .iter()
            .all(|record| record.token != token)
    );
}
#[tokio::test]
async fn jobs_approval_uses_host_authority_and_does_not_enable_proposals_implicitly() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let capture = seen.clone();
    let mut options=Options::default().handler("jobs.list",|_,_|async{Ok(json!({"jobs":[{"id":"job-long-id","name":"fixture","enabled":false,"proposedBy":"session:worker","trigger":{"kind":"manual"},"action":{"kind":"shell","command":"true"},"nextRunAt":12,"running":false}]}))}).handler("jobs.upsert",move|caller,params|{let capture=capture.clone();async move{capture.lock().unwrap().push((caller.authenticated_host,params.clone()));Ok(params)}});
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "fixture-host".into();
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let port = match *hub.handle().status().borrow() {
        Status::Ready {
            address: Some(address),
            ..
        } => address.port().to_string(),
        _ => panic!(),
    };
    let root = tempfile::tempdir().unwrap();
    let config = root.path().to_str().unwrap();
    let mut out = vec![];
    let mut err = vec![];
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "jobs",
        "enable",
        "job",
        "--hub-port",
        &port,
        "--token",
        "fixture-host",
        "--config-dir",
        config,
    ])
    .unwrap();
    assert!(execute(&args, &mut out, &mut err).await.is_err());
    assert!(seen.lock().unwrap().is_empty());
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "jobs",
        "approve",
        "job",
        "--hub-port",
        &port,
        "--token",
        "fixture-host",
        "--config-dir",
        config,
    ])
    .unwrap();
    execute(&args, &mut out, &mut err).await.unwrap();
    let saved = seen.lock().unwrap();
    assert!(saved[0].0);
    assert_eq!(saved[0].1["enabled"], true);
    assert_eq!(saved[0].1["proposedBy"], "");
    assert!(saved[0].1.get("nextRunAt").is_none());
    drop(saved);
    hub.shutdown().unwrap();
}
fn standalone_smoke(parent_pipe: bool) {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let xdg = root.path().join("xdg");
    let config = if parent_pipe {
        xdg.join("workspacer")
    } else {
        root.path().join("config")
    };
    let historical = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/workspacer-hub")
    } else {
        xdg.join("workspacer-hub")
    };
    std::fs::create_dir_all(&historical).unwrap();
    use base64::Engine as _;
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
    let key = web_push_native::jwt_simple::algorithms::ES256KeyPair::generate();
    let secret = web_push_native::p256::SecretKey::from_slice(&key.to_bytes()).unwrap();
    let vapid=serde_json::to_vec(&json!({"privateKey":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.to_bytes()),"publicKey":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret.public_key().to_encoded_point(false).as_bytes())})).unwrap();
    std::fs::write(historical.join("vapid.json"), &vapid).unwrap();
    std::fs::write(
        historical.join("layout.json"),
        json!({"version":7,"data":{"migrationFixture":true}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        historical.join("usage-pacing.json"),
        json!({"schedule":"five_day"}).to_string(),
    )
    .unwrap();
    std::fs::write(historical.join("jobs.json"),json!({"jobs":[{"id":"retained-job","name":"Retained job","enabled":false,"trigger":{"kind":"manual"},"action":{"kind":"call","call":{"method":"config.get"}}}]}).to_string()).unwrap();
    let db = root.path().join("engine.sqlite");
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let hook = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_port = api.local_addr().unwrap().port();
    let hook_port = hook.local_addr().unwrap().port();
    drop(api);
    drop(hook);
    let mut command = Command::new(env!("CARGO_BIN_EXE_workspacer-rust"));
    command.arg("serve");
    if parent_pipe {
        command.arg("--no-claudemon-init");
    }
    let mut child = Child(
        command
            .args([
                "--json",
                "--hub-port",
                "0",
                "--mcp-port",
                "0",
                "--claudemon-api-port",
                &api_port.to_string(),
                "--claudemon-hook-port",
                &hook_port.to_string(),
            ])
            .arg("--config-dir")
            .arg(&config)
            .arg("--home-dir")
            .arg(&home)
            .arg("--claudemon-db-path")
            .arg(&db)
            .env(
                "WORKSPACER_PARENT_PID",
                if parent_pipe {
                    std::process::id().to_string()
                } else {
                    String::new()
                },
            )
            .stdin(Stdio::piped())
            .env_remove("HUB_TOKEN")
            .env_remove("WORKSPACER_ALLOW_NEW_TOKEN")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &xdg)
            .env("XDG_CONFIG_HOME", &xdg)
            .env("WORKSPACER_USAGE_POLL_ON_BOOT", "0")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let output = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(output).read_line(&mut line).unwrap();
        tx.send(line).unwrap();
    });
    let banner = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("Rust launcher did not become ready");
    let banner: Value =
        serde_json::from_str(&banner).expect("launcher stdout should contain only JSON banner");
    assert_eq!(banner["service"], "workspacer-rust");
    assert_eq!(home.join(".claude/settings.json").exists(), !parent_pipe);
    assert!(banner["busUrl"].as_str().unwrap().starts_with("ws://"));
    assert_eq!(
        std::fs::read_to_string(config.join("remote-token"))
            .unwrap()
            .trim(),
        banner["token"].as_str().unwrap()
    );
    assert_eq!(
        std::fs::read(historical.join("vapid.json")).unwrap(),
        vapid,
        "historical VAPID identity must not rotate"
    );
    assert_eq!(
        config.join("vapid.json").exists(),
        !parent_pipe,
        "standard CLI uses historical hub state; custom config remains isolated"
    );
    if !parent_pipe {
        assert_ne!(std::fs::read(config.join("vapid.json")).unwrap(), vapid);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let bus = workspacer_hub::client::Client::connect_remote(
                banner["busUrl"].as_str().unwrap(),
                banner["token"].as_str().unwrap(),
            )
            .await
            .unwrap();
            let layout = bus.call("layout.get", json!({})).await.unwrap();
            let pacing = bus.call("usage.pacingSchedule", json!({})).await.unwrap();
            let jobs = bus.call("jobs.list", json!({})).await.unwrap();
            if parent_pipe {
                assert_eq!(layout["version"], 7);
                assert_eq!(layout["data"]["migrationFixture"], true);
                assert_eq!(pacing["schedule"], "five_day");
                assert_eq!(jobs["jobs"][0]["id"], "retained-job");
            } else {
                assert_eq!(layout["version"], 0);
                assert_eq!(pacing["schedule"], "");
                assert!(jobs["jobs"].as_array().unwrap().is_empty());
            }
            bus.close();
        });
    if parent_pipe {
        drop(child.0.stdin.take());
    } else {
        #[cfg(unix)]
        unsafe {
            libc::kill(child.0.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        panic!("SIGTERM fixture is Unix-only");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "launcher failed to reap backend on SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::net::TcpListener::bind(("127.0.0.1", api_port)).unwrap();
    std::net::TcpListener::bind(("127.0.0.1", hook_port)).unwrap();
}

#[cfg(unix)]
#[test]
fn standalone_rust_serve_reports_ready_and_handles_sigterm_without_other_binaries() {
    standalone_smoke(false);
}
#[test]
fn standalone_rust_serve_handles_parent_pipe_eof_even_while_parent_pid_lives() {
    standalone_smoke(true);
}

#[tokio::test]
async fn fleet_exit_codes_distinguish_busy_rest_and_unreachable() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let quiet = Arc::new(AtomicBool::new(false));
    let state = quiet.clone();
    let mut options = Options::default().handler("fleet.quiescence", move |_, _| {
        let state = state.clone();
        async move { Ok(json!({"quiescent":state.load(Ordering::Acquire),"blockers":[]})) }
    });
    options.token = "host-fixture".into();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let port = match *hub.handle().status().borrow() {
        Status::Ready {
            address: Some(address),
            ..
        } => address.port().to_string(),
        _ => panic!(),
    };
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "fleet",
        "quiescence",
        "--quiet",
        "--hub-port",
        &port,
        "--token",
        "host-fixture",
    ])
    .unwrap();
    let mut out = vec![];
    let mut err = vec![];
    assert_eq!(execute(&args, &mut out, &mut err).await.unwrap(), 1);
    quiet.store(true, Ordering::Release);
    assert_eq!(execute(&args, &mut out, &mut err).await.unwrap(), 0);
    hub.shutdown().unwrap();
    assert_eq!(execute(&args, &mut out, &mut err).await.unwrap(), 2);
    assert!(out.is_empty());
    assert!(!err.is_empty());
}

#[test]
fn desktop_hub_only_flags_and_worker_roles_are_explicit() {
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "serve",
        "--hub-only",
        "--quiet",
        "--sidecar-node",
        "/desktop/electron",
        "--external-claudemon",
        "http://127.0.0.1:7891",
        "--plugins-dir",
        "/plugins",
        "--examples-dir",
        "/examples",
        "--claudemon-api-port",
        "9009",
        "--config-dir",
        "/fixture/config",
    ])
    .unwrap();
    let Command::Serve(serve) = &args.command else {
        panic!()
    };
    assert!(serve.quiet && serve.hub_only);
    assert_eq!(
        serve.external_claudemon.as_deref(),
        Some("http://127.0.0.1:7891")
    );
    assert_eq!(serve.sidecar_node.as_deref(), Some("/desktop/electron"));
    assert!(plan_serve(&args, serve).is_ok());
    for flags in [
        vec!["--external-claudemon", "http://127.0.0.1:7891"],
        vec!["--hub-only", "--upstream", "ws://localhost:7899/bus"],
        vec!["--upstream", "ws://localhost:7899/bus", "--no-mcp"],
    ] {
        let mut command = vec!["workspacer-rust", "serve"];
        command.extend(flags);
        let Ok(args) = CommandLine::try_parse_from(command) else {
            continue;
        };
        let Command::Serve(serve) = &args.command else {
            panic!()
        };
        assert!(plan_serve(&args, serve).is_err());
    }
}

#[test]
fn quiet_control_plane_starts_without_engine_or_hook_side_effects() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for (origin_override, expected_origin) in [
        (None, "https://environment.fixture:8443"),
        (Some(""), ""),
        (
            Some("https://explicit.fixture:8443/path"),
            "https://explicit.fixture:8443",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let db = root.path().join("must-not-create.sqlite");
        let mut command = Command::new(env!("CARGO_BIN_EXE_workspacer-rust"));
        command
            .args([
                "serve",
                "--hub-only",
                "--quiet",
                "--json",
                "--no-mcp",
                "--trusted-host",
                "client.fixture",
                "--hub-port",
                &port.to_string(),
            ])
            .arg("--config-dir")
            .arg(&config)
            .arg("--home-dir")
            .arg(&home)
            .arg("--claudemon-db-path")
            .arg(&db)
            .env("HUB_TOKEN", "SUPERVISED_HOST_SECRET")
            .env(
                "WORKSPACER_PLUGIN_ORIGIN",
                "https://environment.fixture:8443/path",
            )
            .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", root.path().join("xdg"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(origin) = origin_override {
            command.arg("--plugin-origin").arg(origin);
        }
        let mut child = Child(command.spawn().unwrap());
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if client
                .get(format!("http://127.0.0.1:{port}/health"))
                .send()
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "control plane exited before readiness"
            );
            assert!(Instant::now() < deadline, "control plane readiness timeout");
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            client
                .get(format!("http://127.0.0.1:{port}/health"))
                .header("host", "client.fixture")
                .send()
                .unwrap()
                .status()
                .is_success()
        );
        assert_eq!(
            client
                .get(format!("http://127.0.0.1:{port}/health"))
                .header("host", "untrusted.fixture")
                .send()
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        let origin: Value = client
            .get(format!("http://127.0.0.1:{port}/plugins/origin"))
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(origin["origin"], expected_origin);
        assert!(config.join("plugins").is_dir());
        assert!(!db.exists());
        assert!(!home.join(".claude/settings.json").exists());
        drop(child.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(25));
        }
        use std::io::Read;
        let mut output = String::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(output.is_empty(), "quiet mode must emit no startup banner");
        child
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(!output.contains("SUPERVISED_HOST_SECRET"));
    }
}

#[tokio::test]
async fn explicit_worker_identity_migration_preserves_existing_key_and_never_prints_it() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.yaml"), "{}\n").unwrap();
    let parse = |allow: bool| {
        let mut args = vec![
            "workspacer-rust",
            "--config-dir",
            root.path().to_str().unwrap(),
            "token",
            "init-host",
        ];
        if allow {
            args.push("--allow-new-token");
        }
        CommandLine::try_parse_from(args).unwrap()
    };
    let (mut out, mut err) = (vec![], vec![]);
    assert!(execute(&parse(false), &mut out, &mut err).await.is_err());
    execute(&parse(true), &mut out, &mut err).await.unwrap();
    let token = std::fs::read_to_string(root.path().join("remote-token")).unwrap();
    assert!(!String::from_utf8_lossy(&out).contains(token.trim()));
    out.clear();
    execute(&parse(false), &mut out, &mut err).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("remote-token")).unwrap(),
        token
    );
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["prefix"].as_str().unwrap().len(), 8);
}

#[test]
fn facade_flags_remain_separate_from_the_hub_owner_credential() {
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "serve",
        "--no-jobs",
        "--uploads-to-worker",
        "--untokened",
        "view",
        "--mcp-token",
        "separate-facade",
        "--token",
        "hub-owner",
    ])
    .unwrap();
    assert_eq!(args.token.as_deref(), Some("hub-owner"));
    let Command::Serve(serve) = args.command else {
        panic!()
    };
    assert_eq!(serve.mcp_token.as_deref(), Some("separate-facade"));
    assert!(serve.untokened.is_some() && serve.no_jobs && serve.uploads_to_worker);
    assert!(
        CommandLine::try_parse_from(["workspacer-rust", "serve", "--untokened", "allow"]).is_err()
    );
}

#[tokio::test]
async fn status_preserves_string_details_counts_and_rejects_a_different_http_service() {
    use axum::{Json, Router, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let daemon_address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/health",
                    get(|| async { Json(json!({"service":"fixture-daemon"})) }),
                )
                .route(
                    "/sessions",
                    get(|| async { Json(json!([{"id":"one"},{"id":"two"}])) }),
                ),
        )
        .await
        .unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "status-fixture-owner".into();
    options = options.handler("brain.info", |_, _| async {
        Ok(json!({"scope":"full","implementation":"rust"}))
    });
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let base = vec![
        "workspacer-rust".to_owned(),
        "status".into(),
        "--hub-port".into(),
        address.port().to_string(),
        "--claudemon-api-port".into(),
        daemon_address.port().to_string(),
        "--config-dir".into(),
        root.path().to_string_lossy().into_owned(),
        "--token".into(),
        "status-fixture-owner".into(),
    ];
    let mut args = base.clone();
    args.push("--json".into());
    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        execute(
            &CommandLine::try_parse_from(args).unwrap(),
            &mut out,
            &mut err
        )
        .await
        .unwrap(),
        0
    );
    let report: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(report["claudemon"]["detail"], "healthy, 2 session(s)");
    assert!(
        report["hub"]["detail"]
            .as_str()
            .unwrap()
            .contains("capability method(s)")
    );
    assert_eq!(
        report["brain"]["detail"],
        "registered (brain.info answered)"
    );
    out.clear();
    assert_eq!(
        execute(
            &CommandLine::try_parse_from(base).unwrap(),
            &mut out,
            &mut err
        )
        .await
        .unwrap(),
        0
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains(&format!("http://{address}")));
    assert!(text.contains(&format!("http://{daemon_address}")));
    let args = CommandLine::try_parse_from([
        "workspacer-rust",
        "status",
        "--hub-port",
        &daemon_address.port().to_string(),
        "--claudemon-api-port",
        &daemon_address.port().to_string(),
        "--config-dir",
        root.path().to_str().unwrap(),
        "--json",
    ])
    .unwrap();
    let mut out = Vec::new();
    assert_eq!(execute(&args, &mut out, &mut err).await.unwrap(), 1);
    let report: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(report["hub"]["detail"], "unexpected /health answer");
    assert_eq!(report["brain"]["detail"], "not checked (hub is down)");
    hub.shutdown().unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn invalid_plugin_dev_manifest_has_no_identity_or_hook_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("plugin");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("plugin.json"), "{}").unwrap();
    let config = root.path().join("config");
    let home = root.path().join("home");
    let args = CommandLine::try_parse_from([
        "workspacer",
        "plugin",
        "dev",
        source.to_str().unwrap(),
        "--config-dir",
        config.to_str().unwrap(),
        "--home-dir",
        home.to_str().unwrap(),
    ])
    .unwrap();
    assert!(
        execute(&args, &mut Vec::new(), &mut Vec::new())
            .await
            .is_err()
    );
    assert!(!config.exists());
    assert!(!home.exists());
}

#[test]
fn launcher_refuses_inferred_relative_database_and_explains_external_ownership() {
    let root = tempfile::tempdir().unwrap();
    for xdg in ["", "relative-data"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
            .args(["serve", "--no-claudemon-init", "--config-dir"])
            .arg(root.path().join("config"))
            .arg("--home-dir")
            .arg(root.path())
            .env("XDG_DATA_HOME", xdg)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(
            message.contains("XDG_DATA_HOME") && message.contains("--claudemon-db-path"),
            "{message}"
        );
        assert!(!root.path().join("config").exists());
    }
    let args =
        CommandLine::try_parse_from(["workspacer", "serve", "--external-claudemon"]).unwrap();
    let Command::Serve(serve) = &args.command else {
        panic!()
    };
    let error = plan_serve(&args, serve).unwrap_err().to_string();
    assert!(
        error.contains("--hub-only") && error.contains("borrowed"),
        "{error}"
    );
    let args = CommandLine::try_parse_from([
        "workspacer",
        "serve",
        "--hub-only",
        "--external-claudemon",
        "--claudemon-api-port",
        "19091",
    ])
    .unwrap();
    let Command::Serve(serve) = &args.command else {
        panic!()
    };
    assert_eq!(
        plan_serve(&args, serve)
            .unwrap()
            .external_claudemon
            .as_deref(),
        Some("http://127.0.0.1:19091")
    );
    assert!(
        CommandLine::try_parse_from(["workspacer", "plugin", "dev", ".", "--debounce", "400ms"])
            .is_ok()
    );
}

#[test]
fn disposable_declared_parent_fixture() {
    let Some(marker) = std::env::var_os("WKS_DISPOSABLE_PARENT_MARKER") else {
        return;
    };
    std::fs::write(marker, "ready").unwrap();
    use std::io::Read;
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte);
}

#[test]
fn standalone_parent_death_stops_hub_even_while_its_stdin_writer_remains_open() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let marker = root.path().join("parent-ready");
    let mut parent = Child(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "disposable_declared_parent_fixture",
                "--nocapture",
            ])
            .env("WKS_DISPOSABLE_PARENT_MARKER", &marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !marker.exists() {
        assert!(Instant::now() < deadline);
        assert!(parent.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let mut launcher = Child(
        Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
            .args([
                "serve",
                "--hub-only",
                "--quiet",
                "--no-mcp",
                "--no-jobs",
                "--plugins-dir",
                "",
                "--hub-port",
                &port.to_string(),
                "--token",
                "parent-fixture",
                "--config-dir",
            ])
            .arg(root.path().join("config"))
            .arg("--home-dir")
            .arg(&home)
            .env("WORKSPACER_PARENT_PID", parent.0.id().to_string())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", root.path().join("appdata"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let held_writer = launcher.0.stdin.take().unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    while !client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .is_ok_and(|response| response.status().is_success())
    {
        assert!(launcher.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(parent.0.stdin.take());
    assert!(parent.0.wait().unwrap().success());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = launcher.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "parent PID death did not stop launcher while stdin stayed open"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(held_writer);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[tokio::test]
async fn bare_external_daemon_flag_attaches_read_only_and_never_owns_its_ports_or_store() {
    use axum::{Router, body::Body, http::Request, response::IntoResponse, routing::any};
    use std::{
        process::Stdio,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::io::AsyncBufReadExt;
    let writes = Arc::new(AtomicUsize::new(0));
    let observed = writes.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fake = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(any(move |request: Request<Body>| {
                let observed = observed.clone();
                async move {
                    if request.method() != axum::http::Method::GET {
                        observed.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.uri().path() == "/health" {
                        ([("X-Workspacer-Maintenance", "1")], "ok").into_response()
                    } else {
                        axum::Json(json!([])).into_response()
                    }
                }
            })),
        )
        .await
        .unwrap();
    });
    let hook = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_port = hook.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let database = root.path().join("must-not-open.sqlite");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
        .args([
            "serve",
            "--hub-only",
            "--external-claudemon",
            "--claudemon-api-port",
            &address.port().to_string(),
            "--claudemon-hook-port",
            &hook_port.to_string(),
            "--hub-port",
            "0",
            "--no-mcp",
            "--no-jobs",
            "--plugins-dir",
            "",
            "--json",
            "--token",
            "borrowed-fixture",
            "--config-dir",
        ])
        .arg(root.path().join("config"))
        .arg("--home-dir")
        .arg(&home)
        .arg("--claudemon-db-path")
        .arg(&database)
        .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", root.path().join("appdata"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
        .await
        .unwrap()
        .unwrap();
    let Some(line) = line else {
        let output = child.wait_with_output().await.unwrap();
        panic!(
            "launcher exited before readiness: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let banner: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(banner["mode"], "hub-only");
    assert_eq!(banner["claudemonUrl"], format!("http://{address}"));
    assert!(banner["database"].is_null());
    assert!(!database.exists());
    assert!(!home.join(".claude/settings.json").exists());
    drop(child.stdin.take());
    assert!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    let probe = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(probe.status(), reqwest::StatusCode::OK);
    for mcp_collision in [false, true] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"));
        let port = address.port().to_string();
        command
            .args([
                "serve",
                "--hub-only",
                "--external-claudemon",
                "--claudemon-api-port",
                &port,
                "--token",
                "borrowed-fixture",
                "--config-dir",
            ])
            .arg(root.path().join("collision-config"))
            .arg("--home-dir")
            .arg(&home)
            .env_remove("WORKSPACER_PARENT_PID")
            .kill_on_drop(true);
        if mcp_collision {
            command.args(["--hub-port", "0", "--mcp-port", &port]);
        } else {
            command.args(["--hub-port", &port, "--no-mcp"]);
        }
        let output = tokio::time::timeout(Duration::from_secs(10), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to kill"));
        assert!(!root.path().join("collision-config").exists());
    }
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    fake.abort();
    let _ = fake.await;
}

#[test]
fn launcher_environment_precedence_uses_explicit_token_clear_and_raw_config_poll_choice() {
    const KEY: &str = "WKS_CLI_SELECTION_FIXTURE";
    if let Some(root) = std::env::var_os(KEY) {
        let root = std::path::PathBuf::from(root);
        let args = CommandLine::try_parse_compatible_from([
            "workspacer",
            "-config-dir",
            root.to_str().unwrap(),
            "-token",
            "",
            "serve",
        ])
        .unwrap();
        assert_eq!(args.credential().unwrap(), "persisted-fixture");
        let Command::Serve(serve) = &args.command else {
            panic!()
        };
        assert_eq!(
            plan_serve(&args, serve).unwrap().usage_poll_on_boot,
            Some(false)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("remote-token"), "persisted-fixture").unwrap();
    std::fs::write(
        root.path().join("config.yaml"),
        "usage: {pollOnBoot: false}",
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "launcher_environment_precedence_uses_explicit_token_clear_and_raw_config_poll_choice",
            "--nocapture",
        ])
        .env(KEY, root.path())
        .env("HUB_TOKEN", "ambient-fixture")
        .env("WORKSPACER_USAGE_POLL_ON_BOOT", "1")
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .env("XDG_DATA_HOME", root.path().join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
        .args(["serve", "-help"])
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    let unsupported = std::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
        .args(["serve", "-brain-bin", "obsolete"])
        .output()
        .unwrap();
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("in process"));
    let lost = root.path().join("lost");
    std::fs::create_dir(&lost).unwrap();
    std::fs::write(lost.join("existing-state"), "retain").unwrap();
    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
        .args([
            "serve",
            "--hub-only",
            "--hub-port",
            "0",
            "--no-mcp",
            "--allow-new-token=false",
            "--config-dir",
        ])
        .arg(&lost)
        .env("WORKSPACER_ALLOW_NEW_TOKEN", "1")
        .env_remove("HUB_TOKEN")
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("STATE LOSS"));
    assert!(!lost.join("remote-token").exists());
}

#[test]
fn launcher_home_fallback_is_read_only_and_explicit_home_still_wins() {
    const KEY: &str = "WKS_CLI_HOME_FIXTURE";
    if let Some(root) = std::env::var_os(KEY) {
        let root = std::path::PathBuf::from(root);
        let mut args = CommandLine::try_parse_compatible_from([
            "workspacer",
            "serve",
            "--config-dir",
            root.to_str().unwrap(),
        ])
        .unwrap();
        let Command::Serve(serve) = &args.command else {
            panic!()
        };
        match directories::UserDirs::new() {
            Some(os) => {
                let plan = plan_serve(&args, serve).unwrap();
                assert_eq!(plan.home, os.home_dir());
                assert!(plan.home.is_absolute());
                assert_eq!(plan.database, plan.home.join(".claudemon/state.db"));
            }
            None => {
                // The reference accepts a named lookup error when the OS itself
                // cannot resolve a profile; it must never guess a relative home.
                let error = plan_serve(&args, serve).unwrap_err().to_string();
                assert!(error.contains("home directory unavailable"), "{error}");
            }
        }
        if let Command::Serve(serve) = &mut args.command {
            serve.home_dir = Some(root.join("selected-home"));
        }
        let Command::Serve(serve) = &args.command else {
            panic!()
        };
        let explicit = plan_serve(&args, serve).unwrap();
        assert_eq!(explicit.home, root.join("selected-home"));
        assert_eq!(explicit.database, explicit.home.join(".claudemon/state.db"));
        assert!(
            !root.join("selected-home").exists(),
            "planning must not create home data"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for empty in [false, true] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "launcher_home_fallback_is_read_only_and_explicit_home_still_wins",
                "--nocapture",
            ])
            .env(KEY, root.path())
            .env_remove("HOME")
            .env_remove("USERPROFILE")
            .env_remove("XDG_DATA_HOME");
        if empty {
            child.env("HOME", "").env("USERPROFILE", "");
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}\\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn isolated_plugin_build_fixture() {
    let Some(root) = std::env::var_os("WKS_CLI_PLUGIN_BUILD_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("builds.txt"))
        .unwrap();
    writeln!(file, "build").unwrap();
    assert!(
        !root.join("fail-build").exists(),
        "isolated fixture build failure"
    );
}

#[tokio::test]
async fn plugin_dev_isolates_source_reloads_after_build_and_preserves_live_plugin_on_failed_build()
{
    use std::{process::Stdio, time::Duration};
    use tokio::io::{AsyncBufReadExt, BufReader};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let config = root.path().join("config");
    let home = root.path().join("home");
    std::fs::create_dir_all(config.join("plugins/other")).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        config.join("plugins/other/plugin.json"),
        json!({"id":"other","name":"Other","apiVersion":"1"}).to_string(),
    )
    .unwrap();
    let install = json!([
        std::env::current_exe().unwrap(),
        "--exact",
        "isolated_plugin_build_fixture",
        "--nocapture"
    ]);
    let write = |name: &str| {
        std::fs::write(
            source.join("plugin.json"),
            json!({"id":"fixture.dev","name":name,"apiVersion":"1","install":install}).to_string(),
        )
        .unwrap()
    };
    write("Initial");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
        .args(["plugin", "dev"])
        .arg(&source)
        .args([
            "--hub-only",
            "--no-mcp",
            "--no-jobs",
            "--hub-port",
            "0",
            "--debounce",
            "20ms",
            "--json",
            "--token",
            "dev-fixture",
            "--config-dir",
        ])
        .arg(&config)
        .arg("--home-dir")
        .arg(&home)
        .env("WKS_CLI_PLUGIN_BUILD_FIXTURE", root.path())
        .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", root.path().join("appdata"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let banner = tokio::time::timeout(Duration::from_secs(30), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let banner: Value = serde_json::from_str(&banner).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(line) = stderr.next_line().await.unwrap() {
            if line.contains("[plugin-dev] watching ") {
                return;
            }
        }
        panic!("watcher never became ready")
    })
    .await
    .unwrap();
    let client = workspacer_hub::client::Client::connect_remote(
        banner["busUrl"].as_str().unwrap(),
        "dev-fixture",
    )
    .await
    .unwrap();
    let listed = client.call("plugins.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], "fixture.dev");
    write("Updated by successful build");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let list = client.call("plugins.list", json!({})).await.unwrap();
            if list[0]["name"] == "Updated by successful build" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("builds.txt")).unwrap(),
        "build\n"
    );
    std::fs::write(root.path().join("fail-build"), "fail").unwrap();
    write("Must not replace live plugin");
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(line) = stderr.next_line().await.unwrap() {
            if line.contains("plugin build failed; keeping current sidecar") {
                return;
            }
        }
        panic!("failed build was not reported")
    })
    .await
    .unwrap();
    let list = client.call("plugins.list", json!({})).await.unwrap();
    assert_eq!(list[0]["name"], "Updated by successful build");
    assert_eq!(
        std::fs::read_to_string(root.path().join("builds.txt")).unwrap(),
        "build\nbuild\n"
    );
    client.close();
    drop(child.stdin.take());
    assert!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(source.join("plugin.json").exists());
    assert!(config.join("plugins/other/plugin.json").exists());
}
