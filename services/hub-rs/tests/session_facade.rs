use serde_json::json;
use std::sync::Arc;
use workspacer_hub::{
    auth,
    services::{
        agent_lifecycle::LaunchPreparation,
        session_facade::{SessionFacade, compose_instructions},
        spawn_plan,
    },
};

#[tokio::test]
async fn verified_facade_injects_generation_bound_credentials_and_revokes_only_its_generation() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route("/health", axum::routing::get(move |headers: axum::http::HeaderMap| async move {
            assert!(headers.get("authorization").is_none());
        axum::Json(json!({"status":"ok","service":"workspacer-mcp-facade","hubConnected":true,"pluginCatalogReady":true,"listenAddr":address.to_string(),"hubUrl":"in-process"}))
    }));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let facade = SessionFacade {
        endpoint: Some(format!("http://{address}/mcp").parse().unwrap()),
        expected_hub: "in-process".into(),
        readiness: workspacer_hub::services::session_facade::Readiness::Legacy,
        tokens: dir.path().join("tokens.json"),
        directory: dir.path().join("mcp"),
        home: dir.path().join("unused-home"),
        instructions: "Read the installed collaboration skill.".into(),
    };
    let mut plan = spawn_plan::resolve(
        &json!({"cwd":dir.path(),"provider":"codex"}),
        &json!({}),
        None,
        dir.path(),
        "agent",
        false,
    )
    .unwrap();
    facade.prepare(&mut plan, "one").await.unwrap();
    let record = auth::load(&facade.tokens).unwrap().remove(0);
    assert_eq!(record.scope, "operator");
    assert_eq!(record.label, "session:agent");
    assert_eq!(record.metadata["plugins"], json!(["*"]));
    assert!(
        plan.request["mcp"]
            .as_str()
            .unwrap()
            .contains(&record.token)
    );
    facade.prepare(&mut plan, "two").await.unwrap();
    facade.revoke("agent", "one").await.unwrap();
    assert_eq!(auth::load(&facade.tokens).unwrap().len(), 1);
    facade.revoke("agent", "two").await.unwrap();
    assert!(auth::load(&facade.tokens).unwrap().is_empty());
    let mut claude = spawn_plan::resolve(
        &json!({"cwd":dir.path()}),
        &json!({}),
        None,
        dir.path(),
        "claude",
        false,
    )
    .unwrap();
    workspacer_hub::services::library::Library::new(dir.path().to_owned()).save(&json!({"scope":"global","id":"custom","kind":"mcp","mcp":{"type":"stdio","command":"fixture","env":{"SECRET":"library-secret"}}})).unwrap();
    claude.mcp_item_ids = vec!["custom".into()];
    facade.prepare(&mut claude, "three").await.unwrap();
    let content: serde_json::Value =
        serde_json::from_slice(&std::fs::read(facade.directory.join("claude.json")).unwrap())
            .unwrap();
    let claude_token = auth::load(&facade.tokens)
        .unwrap()
        .into_iter()
        .find(|row| row.label == "session:claude")
        .unwrap();
    assert!(
        !claude.request["argv"]
            .to_string()
            .contains(&claude_token.token)
    );
    assert_eq!(
        content["mcpServers"]["workspacer"]["headers"]["Authorization"],
        format!("Bearer {}", claude_token.token)
    );
    assert!(
        content["mcpServers"]["workspacer"]["headers"]["Authorization"]
            .as_str()
            .unwrap()
            .starts_with("Bearer ")
    );
    assert_eq!(
        content["mcpServers"]["custom"]["env"]["SECRET"],
        "library-secret"
    );
    assert!(
        !serde_json::to_string(&content)
            .unwrap()
            .contains("private-owner-health")
    );
    assert!(
        claude.request["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("--strict-mcp-config"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(facade.directory.join("claude.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    auth::mint(&facade.tokens, auth::Scope::Operator, "owner-pairing").unwrap();
    auth::mint(&facade.tokens, auth::Scope::Operator, "session:legacy-live").unwrap();
    auth::mint(&facade.tokens, auth::Scope::Operator, "session:legacy-dead").unwrap();
    facade
        .sweep(&["legacy-live".to_string()].into_iter().collect())
        .await
        .unwrap();
    let remaining = auth::load(&facade.tokens).unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|r| r.label == "owner-pairing"));
    let mut manager = spawn_plan::resolve(
        &json!({"cwd":dir.path(),"provider":"codex","manager":true,"parentSessionId":"accidental-parent"}),
        &json!({"agents":{"fleetFullAccess":true}}), None, dir.path(), "manager", false,
    ).unwrap();
    facade
        .prepare(&mut manager, "manager-generation")
        .await
        .unwrap();
    let record = auth::load(&facade.tokens)
        .unwrap()
        .into_iter()
        .find(|row| row.label == "session:manager")
        .unwrap();
    assert_eq!(record.scope, "operator");
    assert_eq!(record.metadata["role"], "manager");
    assert_eq!(record.metadata["plugins"], json!(["*"]));
    assert!(!record.metadata.contains_key("yoloAllowed"));
    assert!(!record.metadata.contains_key("profilesAllowed"));
    assert_eq!(manager.request["yolo"], true);
    assert!(
        !manager.request["instructions"]
            .as_str()
            .unwrap()
            .contains("STRUCTURED WORKER ESCALATION CONTRACT")
    );
    task.abort();
}
#[tokio::test]
async fn mismatched_listener_cannot_mint_a_credential() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener,axum::Router::new().route("/health",axum::routing::get(|headers:axum::http::HeaderMap| async move { assert!(headers.get("authorization").is_none()); axum::Json(json!({"status":"ok","service":"workspacer-rust-mcp-migration","hubConnected":true,"pluginCatalogReady":false})) }))).await.unwrap();
    });
    let facade = SessionFacade {
        endpoint: Some(format!("http://{address}/mcp").parse().unwrap()),
        expected_hub: "in-process".into(),
        readiness: workspacer_hub::services::session_facade::Readiness::Legacy,
        tokens: dir.path().join("tokens.json"),
        directory: dir.path().join("mcp"),
        home: dir.path().join("unused-home"),
        instructions: "ready".into(),
    };
    let mut plan = spawn_plan::resolve(
        &json!({"cwd":dir.path()}),
        &json!({}),
        None,
        dir.path(),
        "agent",
        false,
    )
    .unwrap();
    assert!(facade.prepare(&mut plan, "one").await.is_err());
    assert!(!facade.tokens.exists());
    task.abort();
}
#[test]
fn prompt_fragments_are_additive() {
    let mut args = vec![
        json!("claude"),
        json!("--append-system-prompt=profile"),
        json!("--model"),
        json!("opus"),
        json!("--append-system-prompt"),
        json!("facade"),
    ];
    compose_instructions(&mut args).unwrap();
    assert_eq!(
        args,
        vec![
            json!("claude"),
            json!("--model"),
            json!("opus"),
            json!("--append-system-prompt"),
            json!("profile\n\nfacade")
        ]
    );
}

#[test]
fn malformed_profile_prompt_pins_cannot_consume_host_flags() {
    let mut args = vec![
        json!("claude"),
        json!("--append-system-prompt"),
        json!("--model"),
        json!("opus"),
        json!("--append-system-prompt="),
        json!("--append-system-prompt=profile"),
        json!("--append-system-prompt"),
        json!("host contract"),
        json!("--session-id"),
        json!("owned-id"),
        json!("--append-system-prompt"),
    ];
    compose_instructions(&mut args).unwrap();
    assert_eq!(
        args,
        vec![
            json!("claude"),
            json!("--model"),
            json!("opus"),
            json!("--session-id"),
            json!("owned-id"),
            json!("--append-system-prompt"),
            json!("profile\n\nhost contract")
        ]
    );
    // A split-form empty value is a supplied fragment in the legacy helper;
    // only an empty inline spelling is omitted.
    let mut args = vec![
        json!("claude"),
        json!("--append-system-prompt"),
        json!(""),
        json!("--append-system-prompt=host"),
    ];
    compose_instructions(&mut args).unwrap();
    assert_eq!(
        args,
        vec![
            json!("claude"),
            json!("--append-system-prompt"),
            json!("\n\nhost")
        ]
    );
    let mut invalid = vec![json!("claude"), json!("--append-system-prompt"), json!(42)];
    assert!(compose_instructions(&mut invalid).is_err());
}

#[tokio::test]
async fn deliberately_disabled_facade_keeps_selected_servers_and_contracts_without_credentials() {
    use workspacer_hub::services::{library::Library, session_facade::Readiness};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut facade = SessionFacade {
        endpoint: None,
        expected_hub: String::new(),
        readiness: Readiness::Disabled,
        tokens: root.path().join("tokens.json"),
        directory: root.path().join("session-mcp"),
        home: root.path().join("home"),
        instructions: "Owner-supplied instruction.".into(),
    };
    Library::new(root.path().into()).save(&json!({"scope":"global","id":"custom","kind":"mcp","mcp":{"type":"stdio","command":"fixture-mcp","env":{"LIBRARY_ONLY":"preserved"}}})).unwrap();
    let mut plan=spawn_plan::resolve(&json!({"cwd":project,"provider":"claude","transport":"stream","mcpItemIds":["custom"],"resultSchema":{"type":"object"}}),&json!({}),None,root.path(),"disabled-agent",false).unwrap();
    plan.request["extra_args"] = json!(["--append-system-prompt", "Profile instruction."]);
    facade.prepare(&mut plan, "generation").await.unwrap();
    let servers: serde_json::Value = serde_json::from_slice(
        &std::fs::read(facade.directory.join("disabled-agent.json")).unwrap(),
    )
    .unwrap();
    assert!(servers["mcpServers"].get("workspacer").is_none());
    assert_eq!(
        servers["mcpServers"]["custom"]["env"]["LIBRARY_ONLY"],
        "preserved"
    );
    let args = plan.request["extra_args"].as_array().unwrap();
    assert!(args.contains(&json!("mcp__custom")));
    assert!(args.contains(&json!("--strict-mcp-config")));
    assert!(args.iter().any(|arg| {
        arg.as_str().is_some_and(|text| {
            text.contains("Profile instruction.") && text.contains("Owner-supplied instruction.")
        })
    }));
    assert!(
        plan.request["instructions"]
            .as_str()
            .unwrap()
            .contains("STRUCTURED RESULT CONTRACT")
    );
    assert!(
        !plan
            .request
            .to_string()
            .contains("with access to the local workspacer MCP facade")
    );
    assert!(!project.join(".workspacer/skills").exists());
    facade.sweep(&Default::default()).await.unwrap();
    facade.revoke("disabled-agent", "generation").await.unwrap();
    assert!(!facade.tokens.exists());
    let mut codex = spawn_plan::resolve(
        &json!({"cwd":project,"provider":"codex"}),
        &json!({}),
        None,
        root.path(),
        "codex-disabled",
        false,
    )
    .unwrap();
    facade.prepare(&mut codex, "two").await.unwrap();
    assert!(codex.request.get("mcp").is_none());
    facade.readiness = Readiness::Legacy;
    assert!(
        facade
            .prepare(&mut codex, "missing-configured-endpoint")
            .await
            .is_err()
    );
    assert!(!facade.tokens.exists());
    assert!(
        spawn_plan::resolve(
            &json!({"cwd":project,"provider":"pi"}),
            &json!({}),
            None,
            root.path(),
            "pi-disabled",
            false
        )
        .is_err()
    );
}
#[test]
fn concurrent_token_mutations_preserve_other_pairings() {
    let dir = tempfile::tempdir().unwrap();
    let path = Arc::new(dir.path().join("tokens.json"));
    let threads: Vec<_> = (0..12)
        .map(|i| {
            let path = path.clone();
            std::thread::spawn(move || {
                auth::mint(&path, auth::Scope::Operator, &format!("pair-{i}")).unwrap()
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(auth::load(&path).unwrap().len(), 12);
    assert!(
        auth::update_records(&path, |records| -> anyhow::Result<()> {
            records.clear();
            anyhow::bail!("rollback")
        })
        .is_err()
    );
    assert_eq!(auth::load(&path).unwrap().len(), 12);
}

#[test]
fn credential_lock_child() {
    let Some(path) = std::env::var_os("WKS_TEST_CREDENTIAL_LOCK_PATH") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    auth::update_records(&path, |_| -> anyhow::Result<()> {
        std::fs::write(path.with_extension("ready"), "locked")?;
        std::thread::sleep(std::time::Duration::from_secs(300));
        Ok(())
    })
    .unwrap();
}
#[test]
fn killed_credential_writer_releases_advisory_lock_without_age_stealing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    auth::mint(&path, auth::Scope::Operator, "before").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "credential_lock_child", "--nocapture"])
        .env("WKS_TEST_CREDENTIAL_LOCK_PATH", &path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.with_extension("ready").exists() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("credential helper failed to acquire lock");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    auth::mint(&path, auth::Scope::Operator, "after").unwrap();
    assert_eq!(auth::load(&path).unwrap().len(), 2);
    assert!(dir.path().join("tokens.json.lock").exists());
}

#[tokio::test]
async fn exact_health_status_and_identity_are_rechecked_before_mint_and_recover_without_cache() {
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode, Uri},
        routing::get,
    };
    use serde_json::Value;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let good = json!({"status":"ok","service":"workspacer-mcp-facade","hubConnected":true,"pluginCatalogReady":true,"listenAddr":address.to_string(),"hubUrl":"expected-hub"});
    let state = Arc::new(Mutex::new((StatusCode::OK, good.clone())));
    let calls = Arc::new(AtomicUsize::new(0));
    let served = state.clone();
    let counted = calls.clone();
    let app = Router::new().route(
        "/health",
        get(move |headers: HeaderMap, uri: Uri| {
            let (status, body) = served.lock().unwrap().clone();
            counted.fetch_add(1, Ordering::SeqCst);
            async move {
                assert!(headers.get("authorization").is_none());
                assert!(uri.query().is_none());
                (status, Json(body))
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let facade = SessionFacade {
        endpoint: Some(
            format!("http://{address}/mcp?keep=1&t=old-credential#fragment")
                .parse()
                .unwrap(),
        ),
        expected_hub: "expected-hub".into(),
        readiness: workspacer_hub::services::session_facade::Readiness::Legacy,
        tokens: root.path().join("tokens.json"),
        directory: root.path().join("mcp"),
        home: root.path().join("unused-home"),
        instructions: String::new(),
    };
    let plan = |id: &str| {
        spawn_plan::resolve(
            &json!({"cwd":root.path(),"provider":"codex"}),
            &json!({}),
            None,
            root.path(),
            id,
            false,
        )
        .unwrap()
    };
    for status in [
        StatusCode::CREATED,
        StatusCode::ACCEPTED,
        StatusCode::FOUND,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        *state.lock().unwrap() = (status, good.clone());
        let mut candidate = plan("bad-status");
        assert!(
            facade.prepare(&mut candidate, "bad-status").await.is_err(),
            "{status}"
        );
        assert!(!facade.tokens.exists());
        assert!(candidate.request.get("mcp").is_none());
    }
    for (key, bad) in [
        ("status", json!("starting")),
        ("service", json!("different-service")),
        ("hubConnected", json!(false)),
        ("pluginCatalogReady", json!(false)),
        ("listenAddr", json!("127.0.0.1:1")),
        ("hubUrl", json!("other-hub")),
    ] {
        let mut body = good.clone();
        body[key] = bad;
        *state.lock().unwrap() = (StatusCode::OK, body);
        assert!(
            facade
                .prepare(&mut plan("bad-identity"), "bad-identity")
                .await
                .is_err(),
            "{key}"
        );
        assert!(!facade.tokens.exists());
    }
    *state.lock().unwrap() = (StatusCode::OK, good.clone());
    let mut first = plan("first");
    facade.prepare(&mut first, "first").await.unwrap();
    let records = auth::load(&facade.tokens).unwrap();
    assert_eq!(records.len(), 1);
    let endpoint: url::Url = first.request["mcp"].as_str().unwrap().parse().unwrap();
    assert_eq!(endpoint.query_pairs().filter(|(k, _)| k == "t").count(), 1);
    assert!(
        endpoint
            .query_pairs()
            .any(|(k, v)| k == "t" && v == records[0].token)
    );
    assert!(endpoint.query_pairs().any(|(k, v)| k == "keep" && v == "1"));
    let before = std::fs::read(&facade.tokens).unwrap();
    let mut down = good.clone();
    down["pluginCatalogReady"] = Value::Bool(false);
    *state.lock().unwrap() = (StatusCode::OK, down);
    assert!(facade.prepare(&mut plan("down"), "down").await.is_err());
    assert_eq!(std::fs::read(&facade.tokens).unwrap(), before);
    *state.lock().unwrap() = (StatusCode::OK, good);
    facade
        .prepare(&mut plan("recovered"), "recovered")
        .await
        .unwrap();
    assert_eq!(auth::load(&facade.tokens).unwrap().len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 13);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn failed_launch_and_retrying_end_cleanup_use_real_generation_credentials() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use workspacer_hub::services::agent_lifecycle::{LaunchEngine, Lifecycle, Operation};
    struct Engine {
        fail: AtomicBool,
        tokens: std::path::PathBuf,
    }
    impl LaunchEngine for Engine {
        fn sessions(&self) -> Operation<'_, serde_json::Value> {
            Box::pin(async { Ok(json!([])) })
        }
        fn spawn<'a>(&'a self, plan: &'a spawn_plan::Plan) -> Operation<'a, serde_json::Value> {
            Box::pin(async move {
                let records = auth::load(&self.tokens)?;
                assert!(
                    records
                        .iter()
                        .any(|r| r.label == format!("session:{}", plan.session_id))
                );
                if self.fail.load(Ordering::SeqCst) {
                    anyhow::bail!("injected engine failure after credential mint");
                }
                Ok(json!({"session_id":plan.session_id}))
            })
        }
        fn stop<'a>(&'a self, _: &'a str) -> Operation<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().route("/health", axum::routing::get(move || async move {
            axum::Json(json!({"status":"ok","service":"workspacer-mcp-facade","hubConnected":true,"pluginCatalogReady":true,"listenAddr":address.to_string(),"hubUrl":"in-process"}))
        }))).await.unwrap();
    });
    for provider in ["claude", "codex"] {
        let dir = tempfile::tempdir().unwrap();
        let tokens = dir.path().join("tokens.json");
        let facade = Arc::new(SessionFacade {
            endpoint: Some(format!("http://{address}/mcp").parse().unwrap()),
            expected_hub: "in-process".into(),
            readiness: workspacer_hub::services::session_facade::Readiness::Legacy,
            tokens: tokens.clone(),
            directory: dir.path().join("mcp"),
            home: dir.path().join("unused-home"),
            instructions: String::new(),
        });
        let engine = Arc::new(Engine {
            fail: AtomicBool::new(true),
            tokens: tokens.clone(),
        });
        let lifecycle =
            Lifecycle::open(dir.path().join("journal.json"), engine.clone(), facade).unwrap();
        let plan = || {
            spawn_plan::resolve(
                &json!({"cwd":dir.path(),"provider":provider}),
                &json!({}),
                None,
                dir.path(),
                "agent",
                false,
            )
            .unwrap()
        };
        assert!(
            lifecycle
                .launch(plan())
                .await
                .unwrap_err()
                .to_string()
                .contains("engine failure")
        );
        assert!(auth::load(&tokens).unwrap().is_empty());
        engine.fail.store(false, Ordering::SeqCst);
        lifecycle.launch(plan()).await.unwrap();
        let generation = lifecycle.records()["agent"].generation.clone();
        let bytes = std::fs::read(&tokens).unwrap();
        assert_eq!(auth::load(&tokens).unwrap().len(), 1);
        let backup = dir.path().join("tokens-backup.json");
        std::fs::rename(&tokens, &backup).unwrap();
        std::fs::create_dir(&tokens).unwrap();
        assert!(lifecycle.stopped("agent", &generation).await.is_err());
        assert!(lifecycle.records()["agent"].revocation_pending);
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        std::fs::remove_dir(&tokens).unwrap();
        std::fs::rename(&backup, &tokens).unwrap();
        assert!(lifecycle.stopped("agent", &generation).await.unwrap());
        assert!(!lifecycle.records()["agent"].revocation_pending);
        assert!(auth::load(&tokens).unwrap().is_empty());
        // A duplicate stop after durable success must not revisit the store.
        std::fs::remove_file(&tokens).unwrap();
        std::fs::create_dir(&tokens).unwrap();
        assert!(lifecycle.stopped("agent", &generation).await.unwrap());
    }
    server.abort();
}

#[tokio::test]
async fn worker_escalation_survives_facade_preparation_and_profile_prompt_forms() {
    let captured: serde_json::Value = serde_json::from_str(include_str!(
        "../../../apps/desktop/tests/fixtures/worker-escalation-prompt.json"
    ))
    .unwrap();
    let expected = captured["prompt"].as_str().unwrap();
    assert!(expected.len() > 900);
    assert!(captured["literalCount"].as_u64().unwrap() > 5);
    assert_eq!(
        workspacer_hub::services::worker_results::ESCALATION_CONTRACT,
        expected
    );
    let mut checked = 0;
    use workspacer_hub::services::{profiles::Profile, session_facade::Readiness};
    let dir = tempfile::tempdir().unwrap();
    let facade = SessionFacade {
        endpoint: None,
        expected_hub: String::new(),
        readiness: Readiness::Disabled,
        tokens: dir.path().join("tokens.json"),
        directory: dir.path().join("mcp"),
        home: dir.path().join("home"),
        instructions: "Additional host instruction".into(),
    };
    for (provider, transport) in [
        ("codex", "stream"),
        ("opencode", "stream"),
        ("claude", "pty"),
    ] {
        for extras in [
            vec!["--append-system-prompt", "PROFILE"],
            vec!["--append-system-prompt=PROFILE"],
        ] {
            let profile = Profile {
                extra_args: extras.into_iter().map(str::to_owned).collect(),
                ..Default::default()
            };
            let mut plan = spawn_plan::resolve(&json!({"cwd":dir.path(),"provider":provider,"transport":transport,"parentSessionId":"manager"}), &json!({}), Some(&profile), dir.path(), "worker", false).unwrap();
            facade.prepare(&mut plan, "generation").await.unwrap();
            let instructions = if provider == "claude" {
                let argv = plan.request["argv"].as_array().unwrap();
                assert_eq!(
                    argv.iter()
                        .filter(|arg| arg
                            .as_str()
                            .is_some_and(|s| s.starts_with("--append-system-prompt")))
                        .count(),
                    1
                );
                let position = argv
                    .iter()
                    .position(|arg| arg == "--append-system-prompt")
                    .unwrap();
                let prompt = argv[position + 1].as_str().unwrap();
                assert!(prompt.find("PROFILE").unwrap() < prompt.find("wks-escalation").unwrap());
                assert!(prompt.contains("Additional host instruction"));
                prompt
            } else {
                plan.request["instructions"].as_str().unwrap()
            };
            assert!(instructions.contains("wks-escalation"));
            assert!(instructions.contains("requiredAuthorityOrDecision"));
            assert!(
                instructions.contains(expected),
                "full captured prompt must survive facade preparation"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 6);
    assert!(!facade.tokens.exists());
}
