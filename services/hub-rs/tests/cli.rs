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
#[cfg(unix)]
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
        unsafe {
            libc::kill(child.0.id() as libc::pid_t, libc::SIGTERM);
        }
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
#[cfg(unix)]
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

#[cfg(unix)]
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
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let db = root.path().join("must-not-create.sqlite");
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_workspacer-rust"))
            .args([
                "serve",
                "--hub-only",
                "--quiet",
                "--json",
                "--no-mcp",
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
            .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", root.path().join("xdg"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
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
