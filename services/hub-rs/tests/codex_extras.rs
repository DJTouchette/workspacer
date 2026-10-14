//! The Codex extras bus methods over the real bus and an embedded claudemon:
//! `agents.review` / `agents.setGoal` / `agents.clearGoal` /
//! `agents.windowsSandboxSetup` / `agents.rename` are registered, validate
//! their params before claudemon is asked, answer a non-Codex session with
//! claudemon's own reason, and are owner-only (no scoped tier holds them).
//! The Codex side itself is covered against a fake app-server in claudemon.
#![cfg(unix)]
use claudemon::daemon::{
    ServeConfig,
    embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::json;
use std::time::Duration;
use workspacer_hub::{
    Hub, Options,
    auth::{self, Scope},
    client::Client,
};

const SID: &str = "codex-extras-hub";

#[tokio::test]
async fn codex_extras_validate_refuse_non_codex_and_stay_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("stub-claude");
    std::fs::write(&stub, "#!/bin/sh\nwhile IFS= read -r line; do :; done\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("state.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    let engine_client = engine.client();
    let _updates = engine_client.subscribe().unwrap();
    let mut options = Options::default();
    options.engine = Some(engine_client.clone());
    options.data_dir = Some(dir.path().join("hub"));
    options.config_dir = Some(dir.path().join("config"));
    std::fs::create_dir(dir.path().join("config")).unwrap();
    options.scoped_tokens = Some(dir.path().join("tokens.json"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    engine_client
        .request(Command::Request {
            method: "POST".into(),
            path: "/sessions/spawn-managed".into(),
            payload: Some(json!({
                "provider": "claude", "transport": "stream", "cwd": dir.path(),
                "bin": stub, "session_id": SID,
            })),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while client
            .call("sessions.snapshot", json!({"sessionId":SID}))
            .await
            .is_err()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("session row");

    // Refused by the hub before claudemon is asked.
    for (method, params) in [
        ("agents.review", json!({"sessionId":SID})),
        (
            "agents.review",
            json!({"sessionId":SID,"target":{"type":"everything"}}),
        ),
        (
            "agents.review",
            json!({"sessionId":SID,"target":{"type":"uncommittedChanges"},"delivery":"later"}),
        ),
        ("agents.setGoal", json!({"sessionId":SID})),
        ("agents.setGoal", json!({"sessionId":SID,"tokenBudget":-5})),
        (
            "agents.windowsSandboxSetup",
            json!({"sessionId":SID,"mode":"root"}),
        ),
        ("agents.rename", json!({"sessionId":SID,"name":"  "})),
        (
            "agents.rename",
            json!({"sessionId":SID,"name":"two\nlines"}),
        ),
        ("agents.clearGoal", json!({"sessionId":"../x"})),
    ] {
        assert!(
            client.call(method, params.clone()).await.is_err(),
            "{method} {params}"
        );
    }
    // A Claude session has no Codex driver: claudemon's reason comes back.
    for (method, params) in [
        (
            "agents.review",
            json!({"sessionId":SID,"target":{"type":"baseBranch","branch":"main"}}),
        ),
        (
            "agents.setGoal",
            json!({"sessionId":SID,"objective":"ship"}),
        ),
        ("agents.clearGoal", json!({"sessionId":SID})),
        ("agents.windowsSandboxSetup", json!({"sessionId":SID})),
    ] {
        let error = client.call(method, params).await.unwrap_err().to_string();
        assert!(
            error.contains("needs a live Codex app-server session"),
            "{method}: {error}"
        );
    }
    // No launch record (spawned straight on the engine): nothing to name.
    let error = client
        .call("agents.rename", json!({"sessionId":SID,"name":"Mine"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no Workspacer launch record"), "{error}");

    // Owner-only: no scoped tier reaches any of them.
    let tokens = dir.path().join("tokens.json");
    for scope in [Scope::View, Scope::Triage, Scope::Provider] {
        let record = auth::mint(&tokens, scope, "codex-extras-fixture").unwrap();
        let scoped = Client::from_connection(
            hub.handle()
                .connect_authenticated(record.token, false)
                .await
                .unwrap(),
        );
        for method in [
            "agents.review",
            "agents.setGoal",
            "agents.clearGoal",
            "agents.windowsSandboxSetup",
            "agents.rename",
        ] {
            let error = scoped
                .call(method, json!({"sessionId":SID,"objective":"x","name":"x"}))
                .await
                .unwrap_err()
                .to_string();
            assert!(
                !error.contains("needs a live Codex"),
                "{} reached {method}: {error}",
                scope.name()
            );
        }
        scoped.close();
    }
    let _ = engine_client
        .request(Command::Request {
            method: "POST".into(),
            path: format!("/sessions/{SID}/signal"),
            payload: Some(json!({"signal":"SIGTERM"})),
        })
        .await;
}
