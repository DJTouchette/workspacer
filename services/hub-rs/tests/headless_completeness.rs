//! Shipping literal client calls must resolve in the actual owned headless hub.
use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{Options, backend::Backend, client::Client};

#[tokio::test]
async fn every_shipped_client_literal_has_a_registered_headless_provider() {
    let calls = regex::Regex::new(
        r"(?s)(?:\bclient\s*\.\s*call|\bbusCall|\bcall)(?:<.*?>)?\(\s*'([a-z][\w.]*\.[\w.]+)'",
    )
    .unwrap();
    let clients: Vec<_> = [
        (
            "web",
            include_str!("../../../apps/desktop/src/renderer/src/backend/webBackend.ts"),
            71,
        ),
        ("mobile", include_str!("../assets/web/mobile.html"), 19),
        ("remote", include_str!("../assets/web/remote.html"), 8),
    ]
    .into_iter()
    .map(|(client, source, minimum)| {
        let names: BTreeSet<_> = calls
            .captures_iter(source)
            .map(|hit| hit[1].to_owned())
            .collect();
        assert!(
            names.len() >= minimum,
            "{client} parser or shipping surface lost its reviewed floor: {} < {minimum}",
            names.len()
        );
        (client, names)
    })
    .collect();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let mut options = Options::default();
    options.home_dir = Some(root.path().join("home"));
    options.config_dir = Some(config.clone());
    // These are the full standalone composition inputs set by cli/serve.rs,
    // not additional method registrations invented by this fixture.
    options.data_dir = Some(config.clone());
    options.jobs_file = Some(config.join("jobs.json"));
    options.scoped_tokens = Some(config.join("tokens.json"));
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "headless-inventory-fixture".into();
    let owner = Backend::start(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("daemon.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (health, provided) = loop {
        let health = owner.handle().health().await.unwrap();
        let provided: BTreeSet<_> = health["methodNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect();
        // Node/provider actors register asynchronously after the bus is ready.
        if clients.iter().all(|(_, names)| names.is_subset(&provided))
            || tokio::time::Instant::now() >= deadline
        {
            break (health, provided);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let client = Client::connect(&owner.handle()).await.unwrap();
    let listed = client.call("nodes.list", json!({})).await;
    let wake = client
        .call("nodes.wake", json!({"id":"unconfigured"}))
        .await;
    let sleep = client
        .call("nodes.sleep", json!({"id":"unconfigured"}))
        .await;
    client.close();
    owner.shutdown().await.unwrap();
    assert_eq!(listed.unwrap(), json!([]));
    for result in [wake, sleep] {
        assert!(result.unwrap_err().to_string().contains("unknown node"));
    }
    assert_eq!(health["launchReady"], true);
    assert!(
        provided.len() >= 100,
        "headless registration unexpectedly sparse"
    );
    assert!(!provided.contains("fixture.unprovided"));
    for (client, names) in clients {
        let missing: Vec<_> = names.difference(&provided).collect();
        assert!(
            missing.is_empty(),
            "{client} calls unprovided capabilities: {missing:?}"
        );
    }
}
