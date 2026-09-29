use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{
    Hub, Options,
    client::Client,
    federation::{Manager, Peer},
    protocol::Event,
};

#[tokio::test]
async fn router_checks_both_scopes_and_sanitizes_before_the_remote_hop() {
    use workspacer_hub::auth::{self, Scope};
    let dir = tempfile::tempdir().unwrap();
    let tokens = dir.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "viewer").unwrap();
    let mut remote_options = Options::default()
        .handler("config.get", |caller, _| async move {
            Ok(json!({"host":caller.authenticated_host}))
        })
        .handler(
            "agents.reportProgress",
            |_, params| async move { Ok(params) },
        );
    remote_options.listen = Some("127.0.0.1:0".parse().unwrap());
    remote_options.token = "link-fixture".into();
    let remote = Hub::start(remote_options).unwrap();
    let address = remote.ready().await.unwrap().unwrap();
    let mut options =
        Options::default().plugin_token("plugin-token", "fixture", vec!["fixture.*".into()]);
    options.scoped_tokens = Some(tokens);
    options.federation_peers = vec![Peer {
        name: "worker".into(),
        url: format!("ws://{address}/bus"),
        token: "link-fixture".into(),
        dispatch: false,
    }];
    let local = Hub::start(options).unwrap();
    local.ready().await.unwrap();
    let owner = Client::connect(&local.handle()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while owner.call("federation.peers", json!({})).await.unwrap()[0]["connected"] != true {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let viewer = Client::from_connection(
        local
            .handle()
            .connect_authenticated(view.token, false)
            .await
            .unwrap(),
    );
    assert_eq!(
        viewer
            .call("hub:worker/config.get", json!({}))
            .await
            .unwrap()["host"],
        false
    );
    assert!(
        viewer
            .call("hub:worker/config.save", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("not authorized")
    );
    let result = viewer
        .call(
            "hub:worker/agents.reportProgress",
            json!({"callerSessionId":"forged","message":"progress"}),
        )
        .await
        .unwrap();
    assert_eq!(result, json!({"message":"progress"}));
    let plugin = Client::from_connection(
        local
            .handle()
            .connect_authenticated("plugin-token".into(), false)
            .await
            .unwrap(),
    );
    assert!(
        plugin
            .call("hub:worker/config.get", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("plugins may not")
    );
    assert!(
        owner
            .call("hub:worker/agents.spawn", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("requires a configured execution service")
    );
    assert!(
        owner
            .call("hub:worker/hub:other/config.get", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid qualified")
    );
    local.shutdown().unwrap();
    remote.shutdown().unwrap();
}

async fn wait_state(manager: &Manager, connected: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while manager.peers()[0].connected != connected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn only_the_internal_federation_adapter_can_stamp_a_peer() {
    use workspacer_hub::protocol::Frame;
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut watcher = hub.handle().connect().await.unwrap();
    watcher.recv().await.unwrap();
    watcher
        .send(Frame {
            topics: vec!["agent.*".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    watcher.recv().await.unwrap();
    let mut publisher = hub.handle().connect().await.unwrap();
    publisher.recv().await.unwrap();
    let mut event = Event::new("agent.updated", "fixture", json!({}));
    event.hub = "forged-peer".into();
    publisher
        .send(Frame {
            event: Some(event),
            ..Frame::op("publish")
        })
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(frame.event.unwrap().hub.is_empty());
    let mut event = Event::new("agent.updated", "fixture", json!({}));
    event.hub = "verified-peer".into();
    hub.handle().publish_wait(event).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.event.unwrap().hub, "verified-peer");
    hub.shutdown().unwrap();
}

#[test]
fn peer_files_preserve_credentials_and_fail_loudly_on_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.json");
    assert!(
        workspacer_hub::federation::load_peers(&path)
            .unwrap()
            .is_empty()
    );
    std::fs::write(
        &path,
        br#"[{"name":" worker ","url":" ws://localhost:7895/bus ","token":" padded-secret "}]"#,
    )
    .unwrap();
    let peers = workspacer_hub::federation::load_peers(&path).unwrap();
    assert_eq!(peers[0].name, "worker");
    assert_eq!(peers[0].token, " padded-secret ");
    assert!(!peers[0].dispatch);
    std::fs::write(&path, b"{broken").unwrap();
    assert!(workspacer_hub::federation::load_peers(&path).is_err());
    std::fs::write(
        &path,
        br#"[{"name":"x","url":"ws://localhost"},{"name":"x","url":"ws://localhost"}]"#,
    )
    .unwrap();
    assert!(workspacer_hub::federation::load_peers(&path).is_err());
}

#[tokio::test]
async fn owner_peer_configuration_redacts_keeps_and_clears_tokens_and_reloads_links() {
    use workspacer_hub::auth::{self, Scope};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.json");
    let tokens = dir.path().join("tokens.json");
    let operator = auth::mint(&tokens, Scope::Operator, "operator").unwrap();
    let mut options = Options::default();
    options.peers_file = Some(path.clone());
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let owner = Client::connect(&hub.handle()).await.unwrap();
    let scoped = Client::from_connection(
        hub.handle()
            .connect_authenticated(operator.token, false)
            .await
            .unwrap(),
    );
    assert!(
        scoped
            .call("federation.peersConfig", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("server owner")
    );
    assert!(
        scoped
            .call("federation.savePeersConfig", json!({"peers":[]}))
            .await
            .is_err()
    );
    let row = json!({"name":"worker","url":"ws://127.0.0.1:9/bus","token":"secret-token","dispatch":true});
    owner
        .call("federation.savePeersConfig", json!({"peers":[row]}))
        .await
        .unwrap();
    let rows = owner
        .call("federation.peersConfig", json!({}))
        .await
        .unwrap();
    assert_eq!(
        rows,
        json!([{"name":"worker","url":"ws://127.0.0.1:9/bus","hasToken":true,"dispatch":true}])
    );
    let info = owner.call("federation.peers", json!({})).await.unwrap();
    assert_eq!(info[0]["name"], "worker");
    assert_eq!(info[0]["dispatch"], true);
    owner
        .call(
            "federation.savePeersConfig",
            json!({"peers":[{"name":"worker","url":"ws://127.0.0.1:9/bus"}]}),
        )
        .await
        .unwrap();
    assert_eq!(
        workspacer_hub::federation::load_peers(&path).unwrap()[0].token,
        "secret-token"
    );
    let before = std::fs::read(&path).unwrap();
    for rows in [
        json!([{"name":"worker","url":"ws://localhost?token=secret"}]),
        json!([{"name":"worker","url":"ws://localhost"},{"name":"worker","url":"ws://localhost"}]),
    ] {
        assert!(
            owner
                .call("federation.savePeersConfig", json!({"peers":rows}))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    owner
        .call(
            "federation.savePeersConfig",
            json!({"peers":[{"name":"worker","url":"ws://127.0.0.1:9/bus","token":""}]}),
        )
        .await
        .unwrap();
    assert!(
        workspacer_hub::federation::load_peers(&path).unwrap()[0]
            .token
            .is_empty()
    );
    owner
        .call("federation.savePeersConfig", json!({"peers":[]}))
        .await
        .unwrap();
    assert_eq!(
        owner.call("federation.peers", json!({})).await.unwrap(),
        json!([])
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn two_hubs_forward_once_and_reconnect_without_replaying_calls() {
    let local = Hub::start(Options::default()).unwrap();
    local.ready().await.unwrap();
    let watcher = Client::connect(&local.handle()).await.unwrap();
    watcher
        .topics(BTreeSet::from(["agent.*".into(), "layout.*".into()]))
        .await
        .unwrap();
    let mut events = watcher.events();
    let mut options =
        Options::default().handler("fixture.echo", |_, params| async move { Ok(params) });
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "federation-fixture".into();
    let remote = Hub::start(options).unwrap();
    let address = remote.ready().await.unwrap().unwrap();
    let mut manager = Manager::start(
        local.handle(),
        vec![Peer {
            name: "worker".into(),
            url: format!("ws://{address}/bus?peer=0"),
            token: "federation-fixture".into(),
            dispatch: false,
        }],
    )
    .unwrap();
    wait_state(&manager, true).await;
    assert!(!manager.dispatch_enabled("worker"));
    assert!(!manager.dispatch_enabled("unknown"));
    assert!(
        manager
            .forward("unknown", "fixture.echo", json!({}))
            .await
            .is_err()
    );
    assert_eq!(
        manager
            .forward("worker", "fixture.echo", json!("barrier"))
            .await
            .unwrap(),
        json!("barrier")
    );
    let mut already_forwarded = Event::new("agent.updated", "fixture", json!({"id":"foreign"}));
    already_forwarded.hub = "third".into();
    remote
        .handle()
        .publish_wait(already_forwarded)
        .await
        .unwrap();
    remote
        .handle()
        .publish_wait(Event::new("layout.changed", "fixture", json!({})))
        .await
        .unwrap();
    remote
        .handle()
        .publish_wait(Event::new(
            "agent.updated",
            "fixture",
            json!({"id":"owned","cwd":"/peer-only"}),
        ))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.hub, "worker");
    assert_eq!(
        event.data.unwrap(),
        json!({"id":"owned","cwd":"/peer-only"})
    );
    assert!(!event.id.is_empty());
    remote.shutdown().unwrap();
    wait_state(&manager, false).await;
    assert!(
        manager
            .forward("worker", "fixture.echo", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("not submitted")
    );
    assert!(manager.peers()[0].last_seen > 0);
    let mut options =
        Options::default().handler("fixture.echo", |_, params| async move { Ok(params) });
    options.listen = Some(address);
    options.token = "federation-fixture".into();
    let replacement = Hub::start(options).unwrap();
    replacement.ready().await.unwrap();
    wait_state(&manager, true).await;
    manager
        .forward("worker", "fixture.echo", json!(null))
        .await
        .unwrap();
    replacement
        .handle()
        .publish_wait(Event::new(
            "agent.updated",
            "fixture",
            json!({"id":"after-reconnect"}),
        ))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.data.unwrap()["id"], "after-reconnect");
    manager.shutdown().await;
    assert!(!manager.peers()[0].connected);
    replacement.shutdown().unwrap();
    local.shutdown().unwrap();
}
