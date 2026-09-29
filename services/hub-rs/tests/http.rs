use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use workspacer_hub::{Hub, Options};

#[tokio::test]
async fn health_hides_topology_and_websocket_checks_credentials_host_and_origin() {
    let mut options = Options::default().handler("fixture.echo", |_, p| async move { Ok(p) });
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "fixture-secret".into();
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let http = reqwest::Client::new();
    let public = http
        .get(format!("http://{address}/health"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(public, json!({"status":"ok"}));
    let private = http
        .get(format!("http://{address}/health"))
        .bearer_auth("fixture-secret")
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(private["methodNames"], json!(["federation.peers", "federation.peersConfig", "federation.resumePeer", "federation.savePeersConfig", "fixture.echo", "fleet.dispatchTargets", "fleet.quiescence", "machine.power", "machine.stop", "plugins.tools"]));
    for (token, origin, host, status) in [
        ("bad", None, None, 401),
        (
            "fixture-secret",
            Some("https://attacker.example"),
            None,
            403,
        ),
        ("fixture-secret", Some("null"), None, 403),
        ("fixture-secret", None, Some("rebound.example"), 403),
    ] {
        let mut request = format!("ws://{address}/bus?token={token}")
            .into_client_request()
            .unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
        }
        if let Some(host) = host {
            request.headers_mut().insert("host", host.parse().unwrap());
        }
        let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
        match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                assert_eq!(response.status().as_u16(), status)
            }
            other => panic!("unexpected {other}"),
        }
    }
    for credential in ["Bearer wrong", "Basic invalid"] {
    let mut request = format!("ws://{address}/bus?token=fixture-secret")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", credential.parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    }
    let mut request = format!("ws://{address}/bus?token=fixture-secret")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "http://localhost:3000".parse().unwrap());
    let (mut client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    client.close(None).await.unwrap();
    hub.shutdown().unwrap();
}
