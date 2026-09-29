use serde_json::{Value, json};
use workspacer_hub::{
    Hub, Options, Status,
    auth::{self, Scope},
};

async fn rpc(
    client: &reqwest::Client,
    address: &str,
    token: &str,
    method: &str,
    params: Value,
) -> reqwest::Response {
    client
        .post(format!("http://{address}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-03-26")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
}
async fn result(response: reqwest::Response) -> Value {
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("expected JSON response: {text}"))
}

async fn optional_rpc(
    client: &reqwest::Client,
    address: &str,
    authorization: Option<&str>,
    params: Value,
) -> reqwest::Response {
    let mut request = client
        .post(format!("http://{address}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-03-26");
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    request
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}))
        .send()
        .await
        .unwrap()
}
fn tool_value(result: &Value) -> Value {
    serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn explicit_guest_policy_is_live_scoped_and_never_fallback_for_bad_credentials() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.yaml");
    let tokens = directory.path().join("tokens.json");
    std::fs::write(&config, "facade: { untokenedAccess: deny }").unwrap();
    let revoked = auth::mint(&tokens, Scope::View, "revoked").unwrap();
    auth::revoke(&tokens, &revoked.token).unwrap();
    let before = std::fs::read(&tokens).unwrap();
    let writes = Arc::new(AtomicUsize::new(0));
    let count = writes.clone();
    let mut options=Options::default().handler("app.getCwd",|caller,_|async move{Ok(json!({"scope":caller.scope,"host":caller.authenticated_host,"tokenId":caller.token_id}))})
        .handler("fs.write",move|caller,_|{let count=count.clone();async move{count.fetch_add(1,Ordering::SeqCst);Ok(json!({"scope":caller.scope,"host":caller.authenticated_host}))}})
        .handler("agents.reportProgress",|caller,params|async move{Ok(json!({"host":caller.authenticated_host,"callerSessionId":params["callerSessionId"]}))});
    options.control_plane_only = true;
    options.config_dir = Some(directory.path().into());
    options.scoped_tokens = Some(tokens.clone());
    options.token = "owner".into();
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("missingMCP"),
    };
    let http = reqwest::Client::new();
    let read = json!({"name":"get_host_cwd","arguments":{}});
    assert_eq!(
        optional_rpc(&http, &address, None, read.clone())
            .await
            .status(),
        401
    );
    std::fs::write(&config, "facade: { untokenedAccess: view }").unwrap();
    let view = tool_value(&result(optional_rpc(&http, &address, None, read.clone()).await).await);
    assert_eq!(view["scope"], "view");
    assert_eq!(view["host"], false);
    assert_eq!(view["tokenId"], "");
    for header in [
        String::new(),
        "Bearer ".into(),
        "Basic unknown".into(),
        "Bearer unknown".into(),
        format!("Bearer {}", revoked.token),
    ] {
        assert_eq!(
            optional_rpc(&http, &address, Some(&header), read.clone())
                .await
                .status(),
            401,
            "{header}"
        );
    }
    for query in ["?t=", "?t=unknown"] {
        let response = http
            .post(format!("http://{address}/mcp{query}"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    let write =
        json!({"name":"write_file","arguments":{"path":"/fixture.txt","contents":"fixture"}});
    assert!(
        result(optional_rpc(&http, &address, None, write.clone()).await)
            .await
            .get("error")
            .is_some()
    );
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    std::fs::write(&config, "facade: { untokenedAccess: operator }").unwrap();
    let operator = tool_value(&result(optional_rpc(&http, &address, None, write).await).await);
    assert_eq!(operator["host"], false);
    assert_eq!(operator["scope"], "operator");
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    let progress = tool_value(
        &result(
            optional_rpc(
                &http,
                &address,
                None,
                json!({"name":"report_progress","arguments":{"note":"fixture"}}),
            )
            .await,
        )
        .await,
    );
    assert_eq!(progress["callerSessionId"], "");
    assert_eq!(progress["host"], false);
    let forged=result(optional_rpc(&http,&address,None,json!({"name":"report_progress","arguments":{"note":"fixture","callerSessionId":"forged"}})).await).await;
    assert_eq!(forged["result"]["isError"], true);
    std::fs::write(&config, "facade: [ malformed").unwrap();
    assert_eq!(
        optional_rpc(&http, &address, None, read).await.status(),
        401
    );
    assert_eq!(
        std::fs::read(tokens).unwrap(),
        before,
        "guest access must not mint a stored token"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn separate_static_facade_credential_disables_guest_but_never_becomes_host() {
    let mut options = Options::default().handler("app.getCwd", |caller, _| async move {
        Ok(json!({"scope":caller.scope,"host":caller.authenticated_host,"tokenId":caller.token_id}))
    });
    options.token = "owner".into();
    options.mcp_static_token = Some("static-facade".into());
    options.mcp_untokened = Some(workspacer_hub::mcp::UntokenedAccess::Operator);
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("missingMCP"),
    };
    let http = reqwest::Client::new();
    let read = json!({"name":"get_host_cwd","arguments":{}});
    assert_eq!(
        optional_rpc(&http, &address, None, read.clone())
            .await
            .status(),
        401
    );
    let value = tool_value(
        &result(optional_rpc(&http, &address, Some("Bearer static-facade"), read.clone()).await)
            .await,
    );
    assert_eq!(value["scope"], "operator");
    assert_eq!(value["host"], false);
    assert_eq!(value["tokenId"], auth::fingerprint("static-facade"));
    let owner =
        tool_value(&result(optional_rpc(&http, &address, Some("Bearer owner"), read).await).await);
    assert_eq!(owner["host"], true);
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn ui_events_require_triage_and_use_only_host_stamped_literal_envelopes() {
    use std::{collections::BTreeSet, time::Duration};
    use workspacer_hub::client::Client;
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "view").unwrap();
    let triage = auth::mint(&tokens, Scope::Triage, "triage").unwrap();
    let mut options = Options::default();
    options.token = "host-fixture".into();
    options.scoped_tokens = Some(tokens);
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let bus = Client::connect(&hub.handle()).await.unwrap();
    let mut events = bus.events();
    bus.topics(BTreeSet::from(["command.open_pane".into()]))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let denied = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"open_browser","arguments":{"url":"https://example.test"}}),
        )
        .await,
    )
    .await;
    assert!(denied.get("error").is_some());
    let accepted = result(
        rpc(
            &client,
            &address,
            &triage.token,
            "tools/call",
            json!({"name":"open_browser","arguments":{"url":"https://example.test"}}),
        )
        .await,
    )
    .await;
    assert_ne!(accepted["result"]["isError"], true);
    assert_eq!(accepted["result"]["content"][0]["text"], "ok");
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "command.open_pane");
    assert_eq!(event.source, "mcp-facade");
    assert!(event.hub.is_empty());
    assert_eq!(
        event.data,
        Some(json!({"paneType":"browser","url":"https://example.test"}))
    );
    bus.close();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn mcp_trusted_proxy_hosts_still_require_matching_browser_origin_and_credentials() {
    let mut options =
        Options::default().handler("config.get", |_, _| async { Ok(json!({"fixture":true})) });
    options.token = "owner".into();
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.trusted_hosts = vec!["mcp.example".into()];
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address,
        _ => panic!("MCP unavailable"),
    };
    let http = reqwest::Client::new();
    for (host, origin, token, status) in [
        ("mcp.example", "https://mcp.example", "owner", 200),
        ("mcp.example", "https://attacker.example", "owner", 403),
        ("attacker.example", "https://attacker.example", "owner", 403),
        ("mcp.example", "https://mcp.example", "invalid", 401),
    ] {
        let response=http.post(format!("http://{address}/mcp")).bearer_auth(token).header("host",host).header("origin",origin).header("accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-03-26").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_config","arguments":{}}})).send().await.unwrap();
        assert_eq!(response.status().as_u16(), status, "{host} {origin}");
        if status == 200 {
            assert_eq!(
                response.json::<Value>().await.unwrap()["result"]["content"][0]["text"],
                "{\"fixture\":true}"
            );
        }
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn enabled_plugin_tools_are_ambient_but_only_delegate_to_the_declared_plugin_method() {
    let directory = tempfile::tempdir().unwrap();
    let plugins = directory.path().join("plugins");
    let plugin = plugins.join("fixture");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("plugin.json"),serde_json::to_vec(&json!({
        "id":"fixture","apiVersion":"1","provides":["fixture.echo"],
        "tools":[{"name":"echo","description":"fixture echo","method":"fixture.echo","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false}}]
    })).unwrap()).unwrap();
    let tokens = directory.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "view fixture").unwrap();
    let provider = auth::mint(&tokens, Scope::Provider, "provider fixture").unwrap();
    let mut options = Options::default().handler("fixture.echo", |caller, params| async move {
        Ok(json!({"host":caller.authenticated_host,"message":params["message"]}))
    });
    options.token = "host-fixture".into();
    options.scoped_tokens = Some(tokens.clone());
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.plugins_dir = Some(plugins);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let client = reqwest::Client::new();
    let tools = result(rpc(&client, &address, &view.token, "tools/list", json!({})).await).await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "fixture_echo"),
        "{tools}"
    );
    let provider_tools =
        result(rpc(&client, &address, &provider.token, "tools/list", json!({})).await).await;
    assert!(
        provider_tools["result"]["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let provider_call = result(
        rpc(
            &client,
            &address,
            &provider.token,
            "tools/call",
            json!({"name":"fixture_echo","arguments":{"message":"not admitted"}}),
        )
        .await,
    )
    .await;
    assert!(provider_call.get("error").is_some());
    let call = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"fixture_echo","arguments":{"message":"hello"}}),
        )
        .await,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(call["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        json!({"host":true,"message":"hello"})
    );
    let denied = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"fixture_echo","arguments":{"extra":true}}),
        )
        .await,
    )
    .await;
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    auth::revoke(&tokens, &view.token).unwrap();
    assert_eq!(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"fixture_echo","arguments":{"message":"revoked"}})
        )
        .await
        .status(),
        401
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn fleet_tools_merge_peers_and_reduce_remote_conversations_at_the_facade() {
    use std::time::Duration;
    use workspacer_hub::{client::Client, federation::Peer};
    let mut remote_options=Options::default()
        .handler("agents.list", |_,_| async {Ok(json!([{"sessionId":"remote","cwd":"/only-on-worker"}]))})
        .handler("sessions.conversation", |_,params| async move {
            anyhow::ensure!(params==json!({"sessionId":"remote","sinceSeq":2}),"facade options leaked to provider: {params}");
            Ok(json!({"seq":9,"items":[{"kind":"user_message","text":"task"},{"kind":"tool_result"},{"kind":"assistant_text","text":"done"},{"kind":"usage"}]}))
        });
    remote_options.listen = Some("127.0.0.1:0".parse().unwrap());
    remote_options.token = "peer-fixture".into();
    let remote = Hub::start(remote_options).unwrap();
    let remote_address = remote.ready().await.unwrap().unwrap();
    let mut options = Options::default()
        .handler("agents.list", |_, _| async {
            Ok(json!([{"sessionId":"local"}]))
        })
        .handler("sessions.conversation", |_, _| async { Ok(json!(null)) });
    options.token = "host-fixture".into();
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.federation_peers = vec![Peer {
        name: "worker".into(),
        url: format!("ws://{remote_address}/bus"),
        token: "peer-fixture".into(),
        dispatch: false,
    }];
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let bus = Client::connect(&hub.handle()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while bus.call("federation.peers", json!({})).await.unwrap()[0]["connected"] != true {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let client = reqwest::Client::new();
    let response = result(
        rpc(
            &client,
            &address,
            "host-fixture",
            "tools/call",
            json!({"name":"list_agents","arguments":{}}),
        )
        .await,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        json!([{"sessionId":"local"},{"sessionId":"remote","cwd":"/only-on-worker","hub":"worker"}])
    );
    let response=result(rpc(&client,&address,"host-fixture","tools/call",json!({"name":"get_conversation","arguments":{"sessionId":"remote","hub":"worker","sinceSeq":2,"lastMessage":true,"textOnly":true}})).await).await;
    assert_eq!(
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        json!({"seq":9,"lastMessage":"done"})
    );
    hub.shutdown().unwrap();
    remote.shutdown().unwrap();
}

#[tokio::test]
async fn progress_identity_comes_from_the_verified_session_token() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let session = auth::mint(&tokens, Scope::View, "session:worker").unwrap();
    let mut options =
        Options::default().handler("agents.reportProgress", |caller, params| async move {
            anyhow::ensure!(caller.authenticated_host, "facade delegation missing");
            Ok(params)
        });
    options.scoped_tokens = Some(tokens);
    options.token = "host-fixture".into();
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let client = reqwest::Client::new();
    let response = result(
        rpc(
            &client,
            &address,
            &session.token,
            "tools/call",
            json!({"name":"report_progress","arguments":{"note":"tests pass"}}),
        )
        .await,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        json!({"note":"tests pass","callerSessionId":"worker"})
    );
    let forged=result(rpc(&client,&address,&session.token,"tools/call",json!({"name":"report_progress","arguments":{"note":"tests pass","callerSessionId":"other"}})).await).await;
    assert_eq!(forged["result"]["isError"], true);
    let raw = workspacer_hub::client::Client::from_connection(
        hub.handle()
            .connect_authenticated(session.token, false)
            .await
            .unwrap(),
    );
    assert!(
        raw.call(
            "agents.reportProgress",
            json!({"note":"forged","callerSessionId":"other"})
        )
        .await
        .is_err()
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn workflow_facade_requires_session_identity_and_raw_scopes_cannot_assert_it() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let session = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    let mut options = Options::default();
    options.config_dir = Some(directory.path().into());
    options.scoped_tokens = Some(tokens);
    options.token = "host-fixture".into();
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let client = reqwest::Client::new();
    let response = result(
        rpc(
            &client,
            &address,
            &session.token,
            "tools/call",
            json!({"name":"list_workflows","arguments":{}}),
        )
        .await,
    )
    .await;
    let catalog: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(catalog["ok"], true, "{catalog}");
    assert!(catalog["catalog"]["definitions"].as_array().unwrap().len() >= 3);
    let denied = result(
        rpc(
            &client,
            &address,
            "host-fixture",
            "tools/call",
            json!({"name":"list_workflows","arguments":{}}),
        )
        .await,
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    let forged = result(
        rpc(
            &client,
            &address,
            &session.token,
            "tools/call",
            json!({"name":"list_workflows","arguments":{"callerSessionId":"other"}}),
        )
        .await,
    )
    .await;
    assert_eq!(forged["result"]["isError"], true);
    let scoped = workspacer_hub::client::Client::from_connection(
        hub.handle()
            .connect_authenticated(session.token, false)
            .await
            .unwrap(),
    );
    assert!(
        scoped
            .call(
                "fleetWorkflows.request",
                json!({"op":"list","callerSessionId":"manager"})
            )
            .await
            .is_err()
    );
    let host = workspacer_hub::client::Client::connect(&hub.handle())
        .await
        .unwrap();
    assert_eq!(
        host.call("fleetWorkflows.request", json!({"op":"list"}))
            .await
            .unwrap()["ok"],
        true
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn mcp_calls_the_in_memory_hub_and_revalidates_each_http_request() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "read-only fixture").unwrap();
    let operator = auth::mint(&tokens, Scope::Operator, "operator fixture").unwrap();
    let mut options = Options::default()
        .handler("config.get", |_, _| async { Ok(json!({"fixture":true})) })
        .handler("fs.read", |_, p| async move {
            Ok(json!({"contents":p["path"]}))
        });
    options.token = "host-fixture".into();
    options.scoped_tokens = Some(tokens.clone());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.jobs_file = Some(directory.path().join("jobs.json"));
    let hub = Hub::start(options).unwrap();
    assert!(hub.ready().await.unwrap().is_none());
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP listener not ready"),
    };
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .post(format!("http://{address}/mcp?t=host-fixture"))
            .header("authorization", "Basic invalid")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let initialized=result(rpc(&client,&address,&view.token,"initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}})).await).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "workspacer");
    let tools = result(rpc(&client, &address, &view.token, "tools/list", json!({})).await).await;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["get_config", "help", "summarize_agent_status"]);
    let help = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"help","arguments":{}}),
        )
        .await,
    )
    .await;
    let guide = help["result"]["content"][0]["text"].as_str().unwrap();
    assert!(guide.contains("tier: view") && guide.contains("get_config"));
    assert!(!guide.contains("spawn_agent"));
    let read = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"get_config","arguments":{}}),
        )
        .await,
    )
    .await;
    assert_eq!(read["result"]["content"][0]["text"], "{\"fixture\":true}");
    let denied = result(
        rpc(
            &client,
            &address,
            &view.token,
            "tools/call",
            json!({"name":"read_file","arguments":{"path":"/fixture"}}),
        )
        .await,
    )
    .await;
    assert!(denied.get("error").is_some());
    let allowed = result(
        rpc(
            &client,
            &address,
            "host-fixture",
            "tools/call",
            json!({"name":"read_file","arguments":{"path":"/fixture"}}),
        )
        .await,
    )
    .await;
    assert!(
        allowed["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("/fixture")
    );
    let proposal=result(rpc(&client,&address,&operator.token,"tools/call",json!({"name":"propose_job","arguments":{"name":"review me","enabled":true,"trigger":{"kind":"manual"},"action":{"kind":"call","call":{"method":"config.get"}}}})).await).await;
    let proposal: Value =
        serde_json::from_str(proposal["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(proposal["enabled"], false);
    assert_eq!(proposal["proposedBy"], "an agent");
    let denied = result(
        rpc(
            &client,
            &address,
            &operator.token,
            "tools/call",
            json!({"name":"run_job","arguments":{"id":proposal["id"]}}),
        )
        .await,
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    auth::revoke(&tokens, &view.token).unwrap();
    assert_eq!(
        rpc(&client, &address, &view.token, "tools/list", json!({}))
            .await
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("http://{address}/health"))
            .header("host", "rebound.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn spawn_first_message_receipts_preserve_legacy_fallback_without_replaying_uncertain_delivery()
 {
    use std::sync::{Arc, Mutex};
    use workspacer_hub::protocol::Frame;
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let provider_token = auth::mint(&tokens, Scope::Provider, "scripted-provider").unwrap();
    let mut options =
        Options::default().handler("config.get", |_, _| async { Ok(json!({"claude":{}})) });
    options.token = "spawn-receipt-owner".into();
    options.control_plane_only = true;
    options.scoped_tokens = Some(tokens);
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let mut provider = hub
        .handle()
        .connect_authenticated(provider_token.token, false)
        .await
        .unwrap();
    assert_eq!(provider.recv().await.unwrap().op, "hello");
    provider
        .send(Frame {
            methods: vec!["agents.spawn".into(), "agents.sendMessage".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(provider.recv().await.unwrap().op, "registered");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let worker = tokio::spawn(async move {
        while let Some(frame) = provider.recv().await {
            if frame.op != "call" {
                continue;
            }
            let params = frame.params.clone().unwrap_or_default();
            captured
                .lock()
                .unwrap()
                .push((frame.method.clone(), params.clone()));
            let response = match frame.method.as_str() {
                "agents.spawn" => match params["label"].as_str().unwrap() {
                    "confirmed" => json!({"sessionId":"confirmed","messageQueued":true}),
                    "uncertain" => json!({"sessionId":"uncertain","messageQueued":false}),
                    "missing-id" => json!({}),
                    label => json!({"sessionId":label}),
                },
                "agents.sendMessage" if params["sessionId"] == "delivery-fails" => {
                    provider
                        .send(Frame::error(frame.id, "delivery acknowledgement lost"))
                        .unwrap();
                    continue;
                }
                "agents.sendMessage" => json!({"ok":true}),
                _ => panic!("unexpected provider call"),
            };
            provider
                .send(Frame {
                    id: frame.id,
                    result: Some(response),
                    ..Frame::op("result")
                })
                .unwrap();
        }
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    for (key, value) in [
        ("profileGranted", json!(true)),
        ("skipPermissionsGranted", json!(true)),
        ("retrySourceSessionId", json!("forged-predecessor")),
        ("dispatchOwnerSessionId", json!("forged-owner")),
    ] {
        let mut arguments = json!({"cwd":directory.path(),"label":"forged"});
        arguments[key] = value;
        let reply = result(
            rpc(
                &client,
                &address,
                "spawn-receipt-owner",
                "tools/call",
                json!({"name":"spawn_agent","arguments":arguments}),
            )
            .await,
        )
        .await;
        assert_eq!(reply["result"]["isError"], true, "{key}: {reply}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "forged authority reached the provider"
        );
    }
    for (label, message, error, deliveries) in [
        ("confirmed", true, false, 0),
        ("legacy", true, false, 1),
        ("delivery-fails", true, true, 1),
        ("missing-id", true, true, 0),
        ("uncertain", true, true, 0),
        ("no-message", false, false, 0),
    ] {
        seen.lock().unwrap().clear();
        let mut arguments = json!({"cwd":directory.path(),"label":label});
        if message {
            arguments["message"] = "the exact first task".into();
        }
        let reply = result(
            rpc(
                &client,
                &address,
                "spawn-receipt-owner",
                "tools/call",
                json!({"name":"spawn_agent","arguments":arguments}),
            )
            .await,
        )
        .await;
        assert!(reply.get("error").is_none(), "{label}: {reply}");
        assert_eq!(
            reply["result"]["isError"] == true,
            error,
            "{label}: {reply}"
        );
        let calls = seen.lock().unwrap();
        let spawns: Vec<_> = calls
            .iter()
            .filter(|(method, _)| method == "agents.spawn")
            .collect();
        assert_eq!(spawns.len(), 1, "{label}: no automatic spawn replay");
        assert_eq!(spawns[0].1.get("message").is_some(), message, "{label}");
        let sends: Vec<_> = calls
            .iter()
            .filter(|(method, _)| method == "agents.sendMessage")
            .collect();
        assert_eq!(sends.len(), deliveries, "{label}");
        for (_, params) in sends {
            assert_eq!(params["sessionId"], label);
            assert_eq!(params["text"], "the exact first task");
        }
        if error {
            assert!(
                reply["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("do not repeat"),
                "{reply}"
            );
        } else if message {
            assert_eq!(tool_value(&reply)["messageQueued"], true);
        }
    }
    hub.shutdown().unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn forwarding_tools_keep_identity_click_targets_and_operator_boundaries() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "view").unwrap();
    let triage = auth::mint(&tokens, Scope::Triage, "worker").unwrap();
    let operator = auth::mint(&tokens, Scope::Operator, "manager").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut options = Options::default();
    options.token = "forwarding-owner".into();
    options.control_plane_only = true;
    options.scoped_tokens = Some(tokens);
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    for method in [
        "agents.sendMessage",
        "agents.reparent",
        "agents.orphans",
        "terminals.open",
        "notifications.post",
        "agents.notifyWhen",
    ] {
        let calls = calls.clone();
        options = options.handler(method, move |_, params| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"method":method,"params":params}))
            }
        });
    }
    options = options.handler("claude.gate", |_, params| async move {
        Ok(json!({"ok":true,"gate_enabled":params["on"],"session_id":params["sessionId"]}))
    }).handler("sessions.snapshot", |_, params| async move {
        match params["sessionId"].as_str() {
            Some("missing") => anyhow::bail!("snapshot unavailable"),
            Some("settings") => Ok(json!({"settings":{"permissionMode":"acceptEdits"}})),
            _ => Ok(json!({"livePermissionMode":"default","settings":{"permissionMode":"bypassPermissions"}})),
        }
    });
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let address = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address.to_string(),
        _ => panic!("MCP unavailable"),
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    for (tool, method, arguments, triage_allowed) in [
        (
            "send_message",
            "agents.sendMessage",
            json!({"sessionId":"manager","text":"phase one landed","fromSessionId":"worker"}),
            true,
        ),
        (
            "adopt_workers",
            "agents.reparent",
            json!({"fromSessionId":"old-manager","toSessionId":"new-manager"}),
            false,
        ),
        ("list_orphans", "agents.orphans", json!({}), false),
        (
            "open_terminal",
            "terminals.open",
            json!({"cwd":directory.path(),"command":"npm run dev","label":"dev server","parentSessionId":"manager"}),
            false,
        ),
        (
            "notify",
            "notifications.post",
            json!({"title":"Job proposed","body":"Approval required","level":"info","key":"proposal","sessionId":"worker","paneType":"settings","paneSection":"jobs","url":"https://example.test/docs","silent":true,"inAppOnly":true}),
            false,
        ),
        (
            "notify_when",
            "agents.notifyWhen",
            json!({"sessionId":"worker","notifySessionId":"manager","contextUsedPct":80}),
            false,
        ),
    ] {
        for (credential, allowed) in [
            (&view.token, false),
            (&triage.token, triage_allowed),
            (&operator.token, true),
        ] {
            let before = calls.load(Ordering::SeqCst);
            let reply = result(
                rpc(
                    &client,
                    &address,
                    credential,
                    "tools/call",
                    json!({"name":tool,"arguments":arguments.clone()}),
                )
                .await,
            )
            .await;
            if allowed {
                assert!(
                    reply.get("error").is_none() && reply["result"]["isError"] != true,
                    "{tool}: {reply}"
                );
                assert_eq!(
                    tool_value(&reply),
                    json!({"method":method,"params":arguments}),
                    "{tool}"
                );
                assert_eq!(calls.load(Ordering::SeqCst), before + 1);
            } else {
                assert!(reply.get("error").is_some(), "{tool}: {reply}");
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    before,
                    "rejected tool reached provider"
                );
            }
        }
    }
    for (session, expected_mode) in [
        ("live", "default"),
        ("settings", "acceptEdits"),
        ("missing", "unknown"),
    ] {
        let reply = result(
            rpc(
                &client,
                &address,
                &operator.token,
                "tools/call",
                json!({"name":"set_approval_gate","arguments":{"sessionId":session,"on":false}}),
            )
            .await,
        )
        .await;
        assert!(
            reply.get("error").is_none() && reply["result"]["isError"] != true,
            "{reply}"
        );
        let receipt = tool_value(&reply);
        assert_eq!(receipt["gate_enabled"], false);
        assert_eq!(receipt["permissionMode"], expected_mode);
        assert!(receipt["note"].as_str().unwrap().contains("still prompts"));
    }
    hub.shutdown().unwrap();
}
