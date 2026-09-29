use serde_json::json;
use workspacer_hub::{Hub, Options, protocol::Frame};
async fn recv(connection: &mut workspacer_hub::Connection) -> Frame {
    tokio::time::timeout(std::time::Duration::from_secs(2), connection.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn plugin_namespace_and_revocation_follow_the_live_connection() {
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    handle
        .register_plugin(
            "fixture-plugin".into(),
            "clock".into(),
            vec!["clock.*".into(), "agents.*".into(), "*".into()],
        )
        .await
        .unwrap();
    let mut plugin = handle
        .connect_authenticated("fixture-plugin".into(), false)
        .await
        .unwrap();
    recv(&mut plugin).await;
    plugin
        .send(Frame {
            methods: vec!["clock.now".into(), "agents.spawn".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(recv(&mut plugin).await.methods, vec!["clock.now"]);
    let mut caller = handle.connect().await.unwrap();
    recv(&mut caller).await;
    caller
        .send(Frame {
            id: "one".into(),
            method: "clock.now".into(),
            ..Frame::op("call")
        })
        .unwrap();
    let forwarded = recv(&mut plugin).await;
    plugin
        .send(Frame {
            id: forwarded.id,
            result: Some(json!(42)),
            ..Frame::op("result")
        })
        .unwrap();
    assert_eq!(recv(&mut caller).await.result, Some(json!(42)));
    handle.revoke_plugin("fixture-plugin".into()).await.unwrap();
    assert!(plugin.recv().await.is_none());
    assert!(
        handle
            .connect_authenticated("fixture-plugin".into(), false)
            .await
            .is_err()
    );
    caller
        .send(Frame {
            methods: vec!["clock.now".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(recv(&mut caller).await.methods, vec!["clock.now"]);
    hub.shutdown().unwrap();
}
