use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
#[derive(Default)]
struct Fake {
    sent: Mutex<Vec<(String, Value)>>,
    statuses: Mutex<BTreeMap<String, u16>>,
}
impl Sender for Fake {
    fn send<'a>(
        &'a self,
        _: &'a crypto::Keys,
        sub: &'a Subscription,
        payload: Vec<u8>,
    ) -> Operation<'a, u16> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap()
                .push((sub.endpoint.clone(), serde_json::from_slice(&payload)?));
            Ok(*self
                .statuses
                .lock()
                .unwrap()
                .get(&sub.endpoint)
                .unwrap_or(&201))
        })
    }
}
fn row(endpoint: &str) -> Value {
    let keys = crypto::Keys::generate();
    json!({"endpoint":endpoint,"keys":{"p256dh":keys.public_key,"auth":URL_SAFE_NO_PAD.encode([7u8;16])}})
}
fn caller() -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        connection_id: 1,
        authenticated_host: false,
        federated: false,
        trusted: false,
        scope: "triage".into(),
        plugin_id: String::new(),
        token_id: "device-fingerprint".into(),
    }
}
fn event(kind: Kind) -> Notification {
    Notification {
        kind,
        title: "repo".into(),
        body: "safe body".into(),
        detail: "private words".into(),
        session: "worker".into(),
        ran_for: 70_000,
    }
}
#[test]
fn endpoints_reject_every_private_literal_family() {
    for endpoint in [
        "http://push.example/sub",
        "file:///tmp/private",
        "https://127.0.0.1/sub",
        "https://10.1.1.1/sub",
        "https://169.254.169.254/latest",
        "https://[::1]/sub",
        "https://[::ffff:127.0.0.1]/sub",
        "https://[::127.0.0.1]/sub",
        "https://[fd00::1]/sub",
        "https://[fe80::1]/sub",
        "https://224.0.0.1/sub",
        "https://user:secret@push.example/sub",
    ] {
        assert!(crypto::endpoint(endpoint).is_err(), "{endpoint}");
    }
    for endpoint in [
        "https://web.push.apple.com/sub",
        "https://updates.push.services.mozilla.com/sub",
        "https://8.8.8.8/sub",
        "https://[2606:4700:4700::1111]/sub",
    ] {
        assert!(crypto::endpoint(endpoint).is_ok(), "{endpoint}");
    }
}
#[test]
fn encrypted_request_roundtrips_and_vapid_preserves_audience_port_without_network() {
    let keys = crypto::Keys::generate();
    keys.validate().unwrap();
    let user = web_push_native::p256::SecretKey::random(&mut rand::rngs::OsRng);
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
    let subscription:Subscription=serde_json::from_value(json!({"endpoint":"https://push.example:8443/opaque","keys":{"p256dh":URL_SAFE_NO_PAD.encode(user.public_key().to_encoded_point(false).as_bytes()),"auth":URL_SAFE_NO_PAD.encode([7u8;16])}})).unwrap();
    let request = crypto::request(&keys, &subscription, b"notification fixture".to_vec()).unwrap();
    assert_eq!(request.headers()["ttl"], "60");
    assert_eq!(request.headers()["urgency"], "high");
    assert_eq!(request.headers()["content-encoding"], "aes128gcm");
    let (_, auth) = crypto::subscription_keys(&subscription.keys).unwrap();
    let plain = web_push_native::decrypt(request.body().clone(), &user, &auth).unwrap();
    assert_eq!(plain, b"notification fixture");
    let authorization = request.headers()["authorization"].to_str().unwrap();
    let jwt = authorization
        .strip_prefix("vapid t=")
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    let claims: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(jwt.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claims["aud"], "https://push.example:8443");
    assert_eq!(claims["sub"], "https://github.com/DJTouchette/workspacer");
}
#[tokio::test]
async fn preferences_revocation_proof_and_test_accounting_use_one_bounded_transport() {
    let root = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let valid = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = valid.clone();
    let manager = Manager::open(
        root.path().into(),
        Arc::new(move |_| flag.load(std::sync::atomic::Ordering::SeqCst)),
        fake.clone(),
    )
    .unwrap();
    let mut first = row("https://push.example/one");
    first["prefs"] = json!({"preview":false,"needs":false});
    first["tokenId"] = "forged".into();
    first["scope"] = "operator".into();
    manager.subscribe(caller(), first.clone()).unwrap();
    assert_eq!(
        manager.list()["subscriptions"][0]["tokenId"],
        "device-fingerprint"
    );
    assert_eq!(manager.list()["subscriptions"][0]["scope"], "triage");
    assert!(manager.list()["subscriptions"][0].get("keys").is_none());
    let second = row("https://push.example/two");
    manager.subscribe(caller(), second.clone()).unwrap();
    manager.broadcast(event(Kind::Needs)).await.unwrap();
    assert_eq!(fake.sent.lock().unwrap().len(), 1);
    assert!(
        fake.sent.lock().unwrap()[0].1["body"]
            .as_str()
            .unwrap()
            .contains("private words")
    );
    fake.sent.lock().unwrap().clear();
    fake.statuses
        .lock()
        .unwrap()
        .insert("https://push.example/one".into(), 410);
    let response = manager.broadcast(event(Kind::Test)).await.unwrap();
    assert_eq!(response["devices"], 2);
    assert_eq!(response["delivered"], 1);
    assert_eq!(response["gone"], 1);
    assert_eq!(manager.list()["subscriptions"].as_array().unwrap().len(), 1);
    assert!(
        manager
            .unsubscribe(json!({"endpoint":second["endpoint"],"auth":"wrong"}))
            .is_err()
    );
    valid.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        manager.broadcast(event(Kind::Test)).await.unwrap()["devices"],
        0
    );
    assert_eq!(manager.list()["subscriptions"][0]["revoked"], true);
    manager
        .unsubscribe(json!({"endpoint":second["endpoint"],"auth":second["keys"]["auth"]}))
        .unwrap();
    assert!(
        manager.list()["subscriptions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn keys_survive_restart_and_lost_keys_drop_only_unusable_subscriptions() {
    let root = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let manager = Manager::open(root.path().into(), Arc::new(|_| true), fake.clone()).unwrap();
    let original = manager.keys.public_key.clone();
    manager
        .subscribe(caller(), row("https://push.example/one"))
        .unwrap();
    drop(manager);
    let manager = Manager::open(root.path().into(), Arc::new(|_| true), fake.clone()).unwrap();
    assert_eq!(manager.keys.public_key, original);
    assert_eq!(manager.list()["subscriptions"].as_array().unwrap().len(), 1);
    drop(manager);
    std::fs::remove_file(root.path().join("vapid.json")).unwrap();
    let manager = Manager::open(root.path().into(), Arc::new(|_| true), fake).unwrap();
    assert_ne!(manager.keys.public_key, original);
    assert!(
        manager.list()["subscriptions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn snapshot_edges_preferences_checkpoints_and_unicode_previews_match_source_contract() {
    let mut watcher = watch::Watcher::default();
    assert!(
        watcher
            .snapshot(&json!({"sessionId":"old","status":"ended"}), 0)
            .is_empty()
    );
    assert!(
        watcher
            .snapshot(
                &json!({"sessionId":"worker","ambientState":"idle","cwd":"/project"}),
                0
            )
            .is_empty()
    );
    assert!(
        watcher
            .snapshot(
                &json!({"sessionId":"worker","ambientState":"thinking","label":"Task"}),
                1000
            )
            .is_empty()
    );
    let events = watcher.snapshot(
        &json!({"sessionId":"worker","ambientState":"streaming","label":"Task"}),
        1_801_000,
    );
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.kind == Kind::Checkpoint));
    let blocked = json!({"sessionId":"worker","ambientState":"waiting_approval","label":"Task","pendingApproval":{"toolName":"Bash","toolInput":{"command":"build\n --check"}}});
    let events = watcher.snapshot(&blocked, 1_802_000);
    assert_eq!(events[0].detail, "Bash build --check");
    assert!(watcher.snapshot(&blocked, 1_803_000).is_empty());
    let events=watcher.snapshot(&json!({"sessionId":"worker","ambientState":"idle","label":"Task","conversation":[{"role":"assistant","content":"All done"}]}),1_804_000);
    assert_eq!(events[0].kind, Kind::Finished);
    assert_eq!(events[0].detail, "All done");
    assert_eq!(events[0].ran_for, 1_803_000);
    assert_eq!(watch::clip(&"😀".repeat(141)).chars().count(), 141);
    assert_eq!(
        watcher.snapshot(&json!({"sessionId":"worker","status":"ended"}), 1_805_000)[0].kind,
        Kind::Ended
    );
}

#[tokio::test]
async fn runtime_rpc_scope_and_subscription_identity_are_server_owned_without_sending() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let view = crate::auth::mint(&tokens, crate::auth::Scope::View, "view").unwrap();
    let triage = crate::auth::mint(&tokens, crate::auth::Scope::Triage, "phone").unwrap();
    let mut options = Options::default();
    options.token = "host-fixture".into();
    options.scoped_tokens = Some(tokens.clone());
    options.push_dir = Some(root.path().into());
    let hub = crate::Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let owner = Client::connect(&hub.handle()).await.unwrap();
    let viewer = Client::from_connection(
        hub.handle()
            .connect_authenticated(view.token, false)
            .await
            .unwrap(),
    );
    let phone = Client::from_connection(
        hub.handle()
            .connect_authenticated(triage.token.clone(), false)
            .await
            .unwrap(),
    );
    let subscription = row("https://push.example/no-live-delivery");
    let key = viewer.call("push.key", Value::Null).await.unwrap();
    assert_eq!(key.as_object().unwrap().len(), 1);
    assert!(key["publicKey"].is_string());
    assert!(
        viewer
            .call("push.subscribe", subscription.clone())
            .await
            .is_err()
    );
    phone.call("push.subscribe", subscription).await.unwrap();
    assert!(phone.call("push.list", Value::Null).await.is_err());
    let listing = owner.call("push.list", Value::Null).await.unwrap();
    assert_eq!(
        listing["subscriptions"][0]["tokenId"],
        crate::auth::fingerprint(&triage.token)
    );
    assert_eq!(listing["subscriptions"][0]["scope"], "triage");
    crate::auth::revoke(&tokens, &triage.token).unwrap();
    assert_eq!(
        owner.call("push.list", Value::Null).await.unwrap()["subscriptions"][0]["revoked"],
        true
    );
    assert_eq!(
        owner
            .call(
                "push.revoke",
                json!({"tokenId":crate::auth::fingerprint(&triage.token)})
            )
            .await
            .unwrap()["removed"],
        1
    );
    hub.shutdown().unwrap();
}
