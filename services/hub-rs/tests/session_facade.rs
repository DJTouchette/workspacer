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
