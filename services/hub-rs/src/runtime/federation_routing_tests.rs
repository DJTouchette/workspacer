//! Source and destination routing are independent gates over a real peer socket.
use super::*;
use crate::services::{
    agent_lifecycle::Operation,
    remote_dispatch::{Delivery, OriginRecord, Update},
};
struct UnownedDelivery;
impl Delivery for UnownedDelivery {
    fn recipient<'a>(&'a self, _: &'a str) -> Operation<'a, Option<String>> {
        Box::pin(async { panic!("unowned forwarding must not book a local owner") })
    }
    fn deliver<'a>(&'a self, _: &'a str, _: &'a OriginRecord, _: &'a Update) -> Operation<'a, ()> {
        Box::pin(async { panic!("unowned forwarding must not deliver a wake") })
    }
}
fn rows(directory: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(directory.join("routing-decisions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[tokio::test]
async fn source_policy_refuses_before_peer_invocation_and_both_hubs_keep_distinct_audits() {
    use crate::{client::Client, federation::Peer};
    let root = tempfile::tempdir().unwrap();
    let destination_dir = root.path().join("destination");
    std::fs::create_dir(&destination_dir).unwrap();
    std::fs::write(destination_dir.join("routing.yaml"),"ceilings:\n  default: {max_capability: frontier_plus}\nprofiles:\n  mixed:\n    reviewer: {fresh: false}\n    deep_reviewer: {fresh: false}\n    frontier_plus: {fresh: false}\n").unwrap();
    let mut destination_options = Options::default();
    destination_options.control_plane_only = true;
    destination_options.token = "destination-owner-fixture".into();
    destination_options.listen = Some("127.0.0.1:0".parse().unwrap());
    destination_options.routing = Some(Arc::new(
        crate::services::routing::RoutingService::open(destination_dir.clone()).unwrap(),
    ));
    let destination = Hub::start(destination_options).unwrap();
    let destination_address = destination.ready().await.unwrap().unwrap();
    let mut provider = destination.handle().connect().await.unwrap();
    provider.recv().await.unwrap();
    provider
        .send(Frame {
            methods: vec!["agents.spawn".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(provider.recv().await.unwrap().methods, vec!["agents.spawn"]);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let provider_task = tokio::spawn(async move {
        while let Some(frame) = provider.recv().await {
            assert_eq!(frame.op, "call");
            observed.fetch_add(1, Ordering::SeqCst);
            provider
                .send(Frame {
                    id: frame.id,
                    result: frame.params,
                    ..Frame::op("result")
                })
                .unwrap();
        }
    });
    let source_dir = root.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("routing.yaml"),
        "ceilings:\n  default: {max_capability: cheap}\n",
    )
    .unwrap();
    let tokens = source_dir.join("tokens.json");
    let operator =
        crate::auth::mint(&tokens, crate::auth::Scope::Operator, "source operator").unwrap();
    let mut source_options = Options::default();
    source_options.token = "source-owner-fixture".into();
    source_options.listen = Some("127.0.0.1:0".parse().unwrap());
    source_options.control_plane_only = true;
    source_options.config_dir = Some(source_dir.clone());
    source_options.scoped_tokens = Some(tokens);
    source_options.remote_dispatch_delivery = Some(Arc::new(UnownedDelivery));
    source_options.federation_peers = vec![Peer {
        name: "destination".into(),
        url: format!("ws://{destination_address}/bus"),
        token: "destination-owner-fixture".into(),
        dispatch: true,
    }];
    let source = Hub::start(source_options).unwrap();
    let source_address = source.ready().await.unwrap().unwrap();
    let client = Client::connect_remote(&format!("ws://{source_address}/bus"), &operator.token)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client.call("federation.peers", Value::Null).await.unwrap()[0]["connected"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let direct = Client::connect_remote(
        &format!("ws://{destination_address}/bus"),
        "destination-owner-fixture",
    )
    .await
    .unwrap();
    let fresh = json!({"cwd":root.path(),"role":"reviewer","resumeSessionId":"remote-existing-session","decisionId":"fresh-origin"});
    // Floor: this exact destination permits the request, proving source refusal
    // is not a denied destination, absent provider or broken federation link.
    assert_eq!(
        direct.call("agents.spawn", fresh.clone()).await.unwrap()["resumeSessionId"],
        "remote-existing-session"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let refused = client
        .call("hub:destination/agents.spawn", fresh)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("remote-existing-session") && refused.contains("fresh"),
        "{refused}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "source-refused request reached destination provider"
    );
    let clamped=client.call("hub:destination/agents.spawn",json!({"cwd":root.path(),"provider":"claude","model":"fable","modelIdentity":"fable","contextWindow":1_000_000,"capability":"frontier_plus","decisionId":"clamped-origin","message":"SECRET_REMOTE_PROMPT","env":{"KEY":"SECRET_REMOTE_ENV"}})).await.unwrap();
    assert_eq!(clamped["model"], "sonnet");
    assert_eq!(clamped["capability"], "cheap");
    assert!(clamped.get("modelIdentity").is_none() && clamped.get("contextWindow").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let exact=client.call("hub:destination/agents.spawn",json!({"cwd":root.path(),"provider":"claude","model":"fable","capability":"frontier_plus","exactModel":true,"decisionId":"exact-origin"})).await.unwrap_err().to_string();
    assert!(exact.contains("no substitute was launched"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let ordinary=client.call("hub:destination/agents.spawn",json!({"cwd":root.path(),"provider":"codex","model":"gpt-5.6-luna","capability":"cheap","role":"implementer","resumeSessionId":"ordinary-resume","decisionId":"allowed-origin"})).await.unwrap();
    assert_eq!(ordinary["resumeSessionId"], "ordinary-resume");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // Canonical-only high selection has an explicit positive destination floor.
    // A cheap capability label must not hide the canonical fable identity.
    let mut canonical = json!({"cwd":root.path(),"provider":"claude","modelIdentity":"fable","contextWindow":1_000_000,"capability":"cheap","exactModel":true,"decisionId":"canonical-origin"});
    assert_eq!(
        direct
            .call("agents.spawn", canonical.clone())
            .await
            .unwrap()["modelIdentity"],
        "fable"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let refused = client
        .call("hub:destination/agents.spawn", canonical.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("no substitute was launched"));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    canonical["exactModel"] = json!(false);
    canonical["decisionId"] = json!("canonical-clamped-origin");
    let canonical = client
        .call("hub:destination/agents.spawn", canonical)
        .await
        .unwrap();
    assert_eq!(canonical["model"], "sonnet");
    assert!(canonical.get("modelIdentity").is_none() && canonical.get("contextWindow").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    let conflicting = client.call("hub:destination/agents.spawn",json!({"cwd":root.path(),"provider":"claude","model":"sonnet","modelIdentity":"fable","contextWindow":1_000_000,"capability":"cheap","decisionId":"conflicting-origin"})).await.unwrap_err().to_string();
    assert!(
        conflicting.contains("conflicting-model-identity"),
        "{conflicting}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    let source_rows = rows(&source_dir);
    assert_eq!(source_rows.len(), 7);
    for (row, expected) in source_rows.iter().zip([
        "refused", "clamped", "refused", "allowed", "refused", "clamped", "refused",
    ]) {
        assert_eq!(row["routingLocation"], "origin");
        assert_eq!(row["spawn"]["outcome"], expected);
        assert_eq!(
            row["spawn"]["callerTokenId"],
            crate::auth::fingerprint(&operator.token)
        );
    }
    let destination_rows = rows(&destination_dir);
    assert_eq!(destination_rows.len(), 5);
    assert!(
        destination_rows
            .iter()
            .all(|row| row["routingLocation"] == "local")
    );
    assert_eq!(
        destination_rows[1]["decisionId"],
        source_rows[1]["decisionId"]
    );
    for directory in [&source_dir, &destination_dir] {
        let log = std::fs::read_to_string(directory.join("routing-decisions.jsonl")).unwrap();
        assert!(
            !log.contains("SECRET_REMOTE")
                && !log.contains(&operator.token)
                && !log.contains("destination-owner-fixture")
        );
    }
    client.close();
    direct.close();
    source.shutdown().unwrap();
    destination.shutdown().unwrap();
    tokio::time::timeout(Duration::from_secs(2), provider_task)
        .await
        .unwrap()
        .unwrap();
}
