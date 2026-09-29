use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use workspacer_hub::services::routing::RoutingService;
async fn ask(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    args: Value,
    sse: bool,
) -> Value {
    let message = json!({"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":name,"arguments":args}});
    if sse {
        let (mut events, endpoint) = open(http, base, token).await;
        initialize(http, &endpoint, token, &mut events).await;
        let response = http
            .post(&endpoint)
            .bearer_auth(token)
            .header("X-Workspacer-Internal-Routing-Host", "authenticated")
            .json(&message)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 202);
        let (_, data) = events.event().await;
        serde_json::from_str(&data).unwrap()
    } else {
        let response = http
            .post(format!("{base}/mcp"))
            .bearer_auth(token)
            .header("X-Workspacer-Internal-Routing-Host", "authenticated")
            .header("accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .json(&message)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
}
fn value(response: &Value) -> Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[tokio::test]
async fn both_transports_preserve_routing_host_boundary_and_applied_select_model_policy() {
    for sse in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let tokens = root.path().join("tokens.json");
        let operator = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
        let service = Arc::new(RoutingService::open(root.path().into()).unwrap());
        let writes = Arc::new(AtomicUsize::new(0));
        let mut opts = options(&tokens);
        opts.control_plane_only = true;
        for method in ["routing.preferences.get", "routing.preferences.save"] {
            let service = service.clone();
            let writes = writes.clone();
            opts = opts.handler(method, move |caller, p| {
                let service = service.clone();
                let writes = writes.clone();
                async move {
                    let result = service.preferences(&caller, method, p)?;
                    if method.ends_with("save") && result["status"] == "applied" {
                        writes.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(result)
                }
            });
        }
        let routing = service.clone();
        opts = opts.handler("routing.select", move |_, p| {
            let routing = routing.clone();
            async move { routing.select(p, &json!({}), 0) }
        });
        let hub = Hub::start(opts).unwrap();
        hub.ready().await.unwrap();
        let base = address(&hub);
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        for (token, allowed) in [(operator.token.as_str(), false), ("host-key", true)] {
            let current = value(
                &ask(
                    &http,
                    &base,
                    token,
                    "routing_preferences_get",
                    json!({}),
                    sse,
                )
                .await,
            );
            assert_eq!(current["configurable"], allowed);
            let saved=ask(&http,&base,token,"routing_preferences_save",json!({"baseRevision":current["revision"],"patch":{"activeProfile":"codex_only","roles":{"scout":"cheap"}}}),sse).await;
            if allowed {
                let saved = value(&saved);
                assert_eq!(saved["status"], "applied", "{saved}");
                let selected = value(
                    &ask(
                        &http,
                        &base,
                        token,
                        "select_model",
                        json!({"role":"scout"}),
                        sse,
                    )
                    .await,
                );
                assert_eq!(selected["provider"], "codex");
                assert_eq!(selected["capability"], "cheap");
                assert_eq!(
                    selected["model"],
                    service.matrix()["profiles"]["codex_only"]["cheap"]["model"]
                );
            } else {
                assert_eq!(saved["result"]["isError"], true, "{saved}");
                assert_eq!(writes.load(Ordering::SeqCst), 0);
            }
        }
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        hub.shutdown().unwrap();
    }
}
