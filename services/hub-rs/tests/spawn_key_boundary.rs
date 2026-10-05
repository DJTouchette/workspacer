use serde_json::{Value, json};
#[path = "support/sweepguard.rs"]
mod sweepguard;
use workspacer_hub::{Hub, Options, client::Client};

#[tokio::test]
async fn every_recognized_spawn_root_requires_its_canonical_spelling_on_the_real_bus() {
    let mut options = Options::default();
    options.control_plane_only = true;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut provider = hub.handle().connect().await.unwrap();
    provider.recv().await.unwrap();
    provider
        .send(serde_json::from_value(json!({"op":"register","methods":["agents.spawn"]})).unwrap())
        .unwrap();
    let registered = serde_json::to_value(provider.recv().await.unwrap()).unwrap();
    assert_eq!(registered["op"], "registered");
    let echo = tokio::spawn(async move {
        while let Some(frame) = provider.recv().await {
            let frame = serde_json::to_value(frame).unwrap();
            if frame["op"] == "call" {
                provider
                    .send(
                        serde_json::from_value(
                            json!({"op":"result","id":frame["id"],"result":frame["params"]}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
        }
    });
    let client = Client::connect(&hub.handle()).await.unwrap();
    let contract: Value =
        serde_json::from_str(include_str!("../../../contracts/spawn-parameter-keys.json")).unwrap();
    let mut tally = sweepguard::Tally::default();
    for case in contract["cases"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let canonical = json!({key:null});
        assert_eq!(
            client
                .call("agents.spawn", canonical.clone())
                .await
                .unwrap(),
            canonical
        );
        let alias = case["alias"].as_str().unwrap();
        assert_eq!(alias, key.to_ascii_uppercase());
        let result = client.call("agents.spawn", json!({alias:null})).await;
        assert!(
            result.is_err(),
            "noncanonical {alias} reached provider: {result:?}"
        );
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("non-canonical") && error.contains(key),
            "{error}"
        );
        tally.ran("deny");
    }
    tally
        .require_every("recognized spawn root aliases", 6)
        .unwrap();
    // Unknown extension keys are still opaque to this spelling gate.
    assert_eq!(
        client
            .call("agents.spawn", json!({"futureExtension":"kept"}))
            .await
            .unwrap(),
        json!({"futureExtension":"kept"})
    );
    drop(client);
    hub.shutdown().unwrap();
    echo.await.unwrap();
}
