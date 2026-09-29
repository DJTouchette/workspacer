use serde_json::json;
use std::time::Duration;
use workspacer_hub::{
    Hub, Options,
    auth::{self, Record, Scope, Store},
    protocol::{Event, Frame},
};

#[test]
fn scopes_use_ascii_trimming_and_exact_stored_names() {
    for value in [" operator ", "\t\nOperator\r\x0b\x0c"] {
        assert_eq!(Scope::parse(value).unwrap(), Scope::Operator);
    }
    for value in [
        "\u{feff}operator",
        "operator\u{0085}",
        "\u{00a0}operator",
        "admin",
        "",
    ] {
        assert!(Scope::parse(value).is_err());
    }
    assert!(
        Record {
            scope: " Operator ".into(),
            ..Default::default()
        }
        .scope()
        .is_none()
    );
    assert!(
        Record {
            scope: "view".into(),
            provides: Some(vec!["*".into()]),
            ..Default::default()
        }
        .provides()
        .is_empty()
    );
}

#[test]
fn mint_revoke_and_roundtrip_keep_legacy_metadata_and_private_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let mut first = auth::mint(&path, Scope::View, "phone").unwrap();
    assert_eq!(first.token.len(), 32);
    first.metadata.insert("role".into(), json!("reviewer"));
    first
        .metadata
        .insert("profilesAllowed".into(), json!(["existing-account"]));
    first
        .metadata
        .insert("futureMetadata".into(), json!({"preserved":true}));
    auth::save(&path, &[first.clone()]).unwrap();
    let provider = auth::mint(&path, Scope::Provider, "worker").unwrap();
    assert_eq!(provider.provides(), &["*".to_owned()]);
    let records = auth::load(&path).unwrap();
    assert!(records[0] == first);
    assert_eq!(records.len(), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(auth::revoke(&path, &first.token[..4]).is_err());
    assert_eq!(
        auth::revoke(&path, &first.token[..12]).unwrap().token,
        first.token
    );
    assert!(auth::revoke(&path, &first.token).is_err());
    assert_eq!(auth::load(&path).unwrap()[0].token, provider.token);
}

#[test]
fn corrupt_deleted_and_ambiguous_token_stores_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let store = Store { path: path.clone() };
    assert!(store.lookup("anything").is_none());
    let first = auth::mint(&path, Scope::View, "phone").unwrap();
    assert!(store.lookup(&first.token).is_some());
    std::fs::write(&path, b"{not-json").unwrap();
    assert!(store.lookup(&first.token).is_none());
    assert!(auth::mint(&path, Scope::Operator, "must-not-overwrite").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{not-json");
    std::fs::remove_file(&path).unwrap();
    assert!(store.lookup(&first.token).is_none());
    let records = ["prefix-aaaaaaaa-1", "prefix-aaaaaaaa-2"].map(|token| Record {
        token: token.into(),
        scope: "view".into(),
        ..Default::default()
    });
    auth::save(&path, &records).unwrap();
    assert!(auth::revoke(&path, "prefix-aaaaaaaa").is_err());
    assert_eq!(auth::load(&path).unwrap().len(), 2);
}

async fn recv(client: &mut workspacer_hub::Connection) -> Frame {
    tokio::time::timeout(Duration::from_secs(2), client.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn scoped_consumers_cannot_receive_unknown_or_terminal_topics_or_desync_side_channels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let view = auth::mint(&path, Scope::View, "phone").unwrap();
    let provider = auth::mint(&path, Scope::Provider, "worker").unwrap();
    let mut options = Options::default();
    options.scoped_tokens = Some(path);
    options.event_buffer = 1;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut host = hub.handle().connect().await.unwrap();
    recv(&mut host).await;
    for record in [view, provider] {
        let mut client = hub
            .handle()
            .connect_authenticated(record.token, false)
            .await
            .unwrap();
        recv(&mut client).await;
        client
            .send(Frame {
                topics: vec!["*".into()],
                ..Frame::op("subscribe")
            })
            .unwrap();
        recv(&mut client).await;
        for topic in [
            "pty.bytes.secret",
            "pty.bytes.secret",
            "future.secret",
            "plugin.settings.changed",
        ] {
            host.send(Frame {
                event: Some(Event::new(topic, "fixture", json!({"private":true}))),
                ..Frame::op("publish")
            })
            .unwrap();
        }
        hub.handle().health().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), client.recv())
                .await
                .is_err()
        );
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn live_provider_grant_change_closes_connection_and_releases_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let mut provider = auth::mint(&path, Scope::Provider, "worker").unwrap();
    let mut options = Options::default();
    options.scoped_tokens = Some(path.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut client = hub
        .handle()
        .connect_authenticated(provider.token.clone(), false)
        .await
        .unwrap();
    recv(&mut client).await;
    client
        .send(Frame {
            methods: vec!["fixture.answer".into()],
            ..Frame::op("register")
        })
        .unwrap();
    recv(&mut client).await;
    provider.provides = Some(vec![]);
    auth::save(&path, &[provider]).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(7), client.recv())
            .await
            .unwrap()
            .is_none()
    );
    let mut host = hub.handle().connect().await.unwrap();
    recv(&mut host).await;
    host.send(Frame {
        methods: vec!["fixture.answer".into()],
        ..Frame::op("register")
    })
    .unwrap();
    assert_eq!(recv(&mut host).await.methods, vec!["fixture.answer"]);
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn local_handler_observes_verified_identity_instead_of_caller_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let operator = auth::mint(&path, Scope::Operator, "phone").unwrap();
    let mut options = Options::default().handler("fixture.identity", |caller,_| async move {
        Ok(json!({"host":caller.authenticated_host,"trusted":caller.trusted,"tokenId":caller.token_id}))
    });
    options.scoped_tokens = Some(path);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut client = hub
        .handle()
        .connect_authenticated(operator.token.clone(), false)
        .await
        .unwrap();
    recv(&mut client).await;
    client
        .send(Frame {
            id: "identity".into(),
            method: "fixture.identity".into(),
            params: Some(json!({"authenticatedHost":true})),
            ..Frame::op("call")
        })
        .unwrap();
    assert_eq!(
        recv(&mut client).await.result.unwrap(),
        json!({"host":false,"trusted":true,"tokenId":auth::fingerprint(&operator.token)})
    );
    hub.shutdown().unwrap();
}

#[test]
fn desktop_shaped_legacy_metadata_and_provider_grants_survive_every_store_rewrite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tokens.json");
    let legacy = json!([
        {"token":"manager-existing-token","scope":"operator","label":"session:manager","created":"2026-08-20T00:00:00Z","plugins":["fixture.plugin"],"profilesAllowed":["work","personal"],"role":"manager","yoloAllowed":true},
        {"token":"plain-existing-token","scope":"view","created":"2026-08-20T00:00:00Z"}
    ]);
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let node = auth::mint(&path, Scope::Provider, "node").unwrap();
    let store = Store { path: path.clone() };
    let manager = store.lookup("manager-existing-token").unwrap();
    for key in ["plugins", "profilesAllowed", "role", "yoloAllowed"] {
        assert_eq!(manager.metadata.get(key), legacy[0].get(key), "{key}");
        assert!(
            !store
                .lookup("plain-existing-token")
                .unwrap()
                .metadata
                .contains_key(key)
        );
    }
    assert_eq!(
        store.lookup(&node.token).unwrap().provides(),
        &["*".to_owned()]
    );
    let phone = auth::mint(&path, Scope::Triage, "phone").unwrap();
    auth::revoke(&path, &phone.token).unwrap();
    assert_eq!(
        store.lookup(&node.token).unwrap().provides(),
        &["*".to_owned()]
    );
    assert_eq!(
        store.lookup("manager-existing-token").unwrap().metadata,
        manager.metadata
    );
    for scope in [Scope::View, Scope::Triage, Scope::Operator] {
        let record = auth::mint(&path, scope, "human").unwrap();
        assert!(record.provides.is_none());
        assert!(
            serde_json::to_value(&record)
                .unwrap()
                .get("provides")
                .is_none()
        );
        let mut injected = record.clone();
        injected.provides = Some(vec!["*".into()]);
        assert!(injected.provides().is_empty(), "{}", scope.name());
    }
    for scope in ["", "bogus"] {
        assert!(
            Record {
                scope: scope.into(),
                provides: Some(vec!["*".into()]),
                ..Default::default()
            }
            .provides()
            .is_empty()
        );
    }
}
