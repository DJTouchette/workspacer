use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{
    Hub, Options, Status,
    auth::{self, Scope},
};
struct Events {
    response: reqwest::Response,
    buffer: String,
}
impl Events {
    async fn next(&mut self) -> Option<(String, String)> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let raw = self.buffer[..end].to_owned();
                self.buffer.drain(..end + 2);
                let mut event = String::new();
                let mut data = String::new();
                for line in raw.lines() {
                    if let Some(value) = line.strip_prefix("event:") {
                        event = value.trim().into()
                    }
                    if let Some(value) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n')
                        }
                        data.push_str(value.trim_start());
                    }
                }
                if !event.is_empty() || !data.is_empty() {
                    return Some((event, data));
                }
                continue;
            }
            match self.response.chunk().await.unwrap() {
                Some(bytes) => self.buffer.push_str(&String::from_utf8_lossy(&bytes)),
                None => return None,
            }
        }
    }
    async fn event(&mut self) -> (String, String) {
        tokio::time::timeout(Duration::from_secs(3), self.next())
            .await
            .unwrap()
            .expect("SSE closed")
    }
}
fn options(tokens: &std::path::Path) -> Options {
    let mut options = Options::default().handler("app.getCwd", |caller, _| async move {
        Ok(json!({"cwd":"/fixture","host":caller.authenticated_host}))
    });
    options.token = "host-key".into();
    options.scoped_tokens = Some(tokens.into());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options
}
fn address(hub: &Hub) -> String {
    match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => format!("http://{address}"),
        _ => panic!("missing MCP listener"),
    }
}
async fn open(client: &reqwest::Client, base: &str, token: &str) -> (Events, String) {
    let response = client
        .get(format!("{base}/sse"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut events = Events {
        response,
        buffer: String::new(),
    };
    let (kind, endpoint) = events.event().await;
    assert_eq!(kind, "endpoint");
    (events, format!("{base}{endpoint}"))
}
async fn post(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    message: Value,
) -> reqwest::Response {
    client
        .post(url)
        .bearer_auth(token)
        .json(&message)
        .send()
        .await
        .unwrap()
}
async fn initialize(client: &reqwest::Client, endpoint: &str, token: &str, events: &mut Events) {
    assert_eq!(post(client,endpoint,token,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).await.status(),202);
    let (event, data) = events.event().await;
    assert_eq!(event, "message");
    assert!(
        serde_json::from_str::<Value>(&data)
            .unwrap()
            .get("result")
            .is_some(),
        "{data}"
    );
    assert_eq!(
        post(
            client,
            endpoint,
            token,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .status(),
        202
    );
}
#[tokio::test]
async fn guest_sse_cannot_change_tier_and_is_closed_when_live_policy_changes() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    std::fs::write(&config, "facade: { untokenedAccess: view }").unwrap();
    let mut options = options(&root.path().join("tokens.json"));
    options.control_plane_only = true;
    options.config_dir = Some(root.path().into());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let base = address(&hub);
    let http = reqwest::Client::new();
    let response = http.get(format!("{base}/sse")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let mut events = Events {
        response,
        buffer: String::new(),
    };
    let (_, endpoint) = events.event().await;
    assert!(!endpoint.contains("t="));
    let endpoint = format!("{base}{endpoint}");
    let initialized=http.post(&endpoint).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"guest-fixture","version":"1"}}})).send().await.unwrap();
    assert_eq!(initialized.status(), 202);
    assert_eq!(events.event().await.0, "message");
    assert_eq!(
        http.post(&endpoint)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    std::fs::write(&config, "facade: { untokenedAccess: operator }").unwrap();
    let changed = http
        .post(&endpoint)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert!(
        [403, 404].contains(&changed),
        "an old view transport cannot inherit a later operator policy: {changed}"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .unwrap()
            .is_none(),
        "idle guest stream survived a policy change"
    );
    std::fs::write(&config, "facade: { untokenedAccess: deny }").unwrap();
    assert_eq!(
        http.get(format!("{base}/sse"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn legacy_sse_uses_rmcp_tools_and_binds_session_to_exact_credential() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let operator = auth::mint(&tokens, Scope::Operator, "operator").unwrap();
    let other = auth::mint(&tokens, Scope::Operator, "other").unwrap();
    let hub = Hub::start(options(&tokens)).unwrap();
    hub.ready().await.unwrap();
    let base = address(&hub);
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/sse"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let (mut events, endpoint) = open(&client, &base, &operator.token).await;
    initialize(&client, &endpoint, &operator.token, &mut events).await;
    for token in [&other.token, "host-key"] {
        assert_eq!(
            post(
                &client,
                &endpoint,
                token,
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
            )
            .await
            .status(),
            403
        );
    }
    assert_eq!(post(&client,&endpoint,&operator.token,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_host_cwd","arguments":{}}})).await.status(),202);
    let (_, reply) = events.event().await;
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["id"], 3);
    let result: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["cwd"], "/fixture");
    assert_eq!(result["host"], false);
    let (mut host_events, host_endpoint) = open(&client, &base, "host-key").await;
    initialize(&client, &host_endpoint, "host-key", &mut host_events).await;
    assert_eq!(
        post(
            &client,
            &host_endpoint,
            &operator.token,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
        )
        .await
        .status(),
        403
    );
    let response=client.post(&host_endpoint).bearer_auth("host-key").header("content-type","application/json").body(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"routing_preferences_save","arguments":{"patch":{"activeProfile":"safe","activeProfile":"unsafe"}}}}"#).send().await.unwrap();
    assert_eq!(response.status(), 202);
    let (_, raw) = host_events.event().await;
    let response: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(response["id"], 9);
    assert_eq!(response["result"]["isError"], true);
    drop(events);
    drop(host_events);
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn revocation_closes_idle_legacy_stream_and_disconnect_removes_session() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let operator = auth::mint(&tokens, Scope::Operator, "operator").unwrap();
    let hub = Hub::start(options(&tokens)).unwrap();
    hub.ready().await.unwrap();
    let base = address(&hub);
    let client = reqwest::Client::new();
    let (mut events, endpoint) = open(&client, &base, &operator.token).await;
    initialize(&client, &endpoint, &operator.token, &mut events).await;
    auth::revoke(&tokens, &operator.token).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), events.next())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        post(
            &client,
            &endpoint,
            &operator.token,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
        )
        .await
        .status(),
        401
    );
    let (host_events, endpoint) = open(&client, &base, "host-key").await;
    drop(host_events);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if post(
                &client,
                &endpoint,
                "host-key",
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            )
            .await
            .status()
                == 404
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn url_only_credentials_survive_endpoint_discovery() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let operator = auth::mint(&tokens, Scope::Operator, "operator").unwrap();
    let hub = Hub::start(options(&tokens)).unwrap();
    hub.ready().await.unwrap();
    let base = address(&hub);
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/sse?t={}", operator.token))
            .header("Authorization", "Basic invalid")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = client
        .get(format!("{base}/sse?t={}", operator.token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut events = Events {
        response,
        buffer: String::new(),
    };
    let (_, endpoint) = events.event().await;
    assert!(endpoint.contains("&t="));
    let response=client.post(format!("{base}{endpoint}")).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).send().await.unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(events.event().await.0, "message");
    drop(events);
    hub.shutdown().unwrap();
}
