//! Facade assertions retained from cmd/mcp, exercised through real HTTP and bus calls.
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
struct Facade {
    hub: Hub,
    address: String,
    http: reqwest::Client,
}
impl Facade {
    async fn start(mut options: Options) -> Self {
        options.control_plane_only = true;
        options.token = "parity-owner".into();
        options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let address = match *hub.handle().status().borrow() {
            Status::Ready {
                mcp_address: Some(address),
                ..
            } => address.to_string(),
            _ => panic!("missing facade"),
        };
        Self {
            hub,
            address,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }
    async fn call(&self, token: &str, name: &str, args: Value) -> Value {
        result(
            rpc(
                &self.http,
                &self.address,
                token,
                "tools/call",
                json!({"name":name,"arguments":args}),
            )
            .await,
        )
        .await
    }
    async fn list(&self, token: &str) -> Value {
        result(rpc(&self.http, &self.address, token, "tools/list", json!({})).await).await["result"]
            ["tools"]
            .clone()
    }
    fn close(self) {
        self.hub.shutdown().unwrap();
    }
}
async fn provider(
    facade: &Facade,
    methods: &[&str],
    answer: impl Fn(&str, Value) -> anyhow::Result<Value> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    use workspacer_hub::protocol::Frame;
    let mut connection = facade.hub.handle().connect().await.unwrap();
    assert_eq!(connection.recv().await.unwrap().op, "hello");
    connection
        .send(Frame {
            op: "register".into(),
            methods: methods.iter().map(|m| (*m).into()).collect(),
            ..Frame::default()
        })
        .unwrap();
    assert_eq!(connection.recv().await.unwrap().op, "registered");
    tokio::spawn(async move {
        while let Some(frame) = connection.recv().await {
            if frame.op != "call" {
                continue;
            }
            let response = match answer(&frame.method, frame.params.unwrap_or_default()) {
                Ok(value) => Frame {
                    op: "result".into(),
                    id: frame.id,
                    result: Some(value),
                    ..Frame::default()
                },
                Err(error) => Frame::error(frame.id, error.to_string()),
            };
            connection.send(response).unwrap();
        }
    })
}
fn success(body: &Value) -> Value {
    assert!(body.get("error").is_none(), "{body}");
    assert_ne!(body["result"]["isError"], true, "{body}");
    tool_value(body)
}
#[tokio::test]
async fn wholesale_config_bad_types_never_reach_provider_but_open_patches_do() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let facade = Facade::start(Options::default().handler("config.save", move |_, params| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(params.clone());
            Ok(params)
        }
    }))
    .await;
    let listed = facade.list("parity-owner").await;
    let schema = &listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "save_config")
        .unwrap()["inputSchema"];
    assert_eq!(schema["additionalProperties"], true);
    assert!(schema.get("required").is_none());
    for invalid in [
        json!({"projects":"{}"}),
        json!({"projects":null}),
        json!({"projects":[]}),
        json!({"ui":{"customThemes":"nord"}}),
        json!({"claude":{"budgets":0}}),
    ] {
        let response = facade.call("parity-owner", "save_config", invalid).await;
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(seen.lock().unwrap().is_empty());
    }
    for patch in [
        json!({"ui":{"guiFontScale":1.3}}),
        json!({"projects":{"/repo":{"label":"A","yolo":true}}}),
        json!({"projects":{}}),
        json!({"ui":{"theme":"nord","customThemes":{"nord":{"bg":"#2e3440"}}}}),
        json!({"pluginSettings":{"x":1}}),
    ] {
        assert_eq!(
            success(
                &facade
                    .call("parity-owner", "save_config", patch.clone())
                    .await
            ),
            patch
        );
        assert_eq!(seen.lock().unwrap().last(), Some(&patch));
    }
    assert_eq!(seen.lock().unwrap().len(), 5);
    facade.close();
}
#[tokio::test]
async fn task_reference_requests_preserve_cas_conflicts_and_refuse_invalid_identity_before_calls() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let session = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let mut options = Options::default();
    options.scoped_tokens = Some(tokens.clone());
    let facade = Facade::start(options).await;
    let worker=provider(&facade,&["fleetWorkflows.request"],move|_,params|{captured.lock().unwrap().push(params);Ok(json!({"ok":false,"code":"conflict","currentRevision":8,"references":{"tickets":[{"id":"HUMAN"}]}}))}).await;

    for (tool, args, op) in [
        (
            "get_task_references",
            json!({"taskId":"task","cwd":"/project"}),
            "taskReferences",
        ),
        (
            "update_task_references",
            json!({"taskId":"task","cwd":"/project","expectedTaskRevision":7,"upsert":[{"kind":"ticket","id":"JIRA-9"}]}),
            "setTaskReferences",
        ),
        (
            "update_task_references",
            json!({"taskId":"task","cwd":"/project","expectedTaskRevision":0,"remove":[{"kind":"ticket","id":"JIRA-9"}]}),
            "setTaskReferences",
        ),
    ] {
        let body = success(&facade.call(&session.token, tool, args.clone()).await);
        assert_eq!(body["currentRevision"], 8);
        assert_eq!(body["references"]["tickets"][0]["id"], "HUMAN");
        let wire = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(wire["op"], op);
        assert_eq!(wire["callerSessionId"], "manager");
        for (key, value) in args.as_object().unwrap() {
            assert_eq!(wire[key], *value);
        }
    }
    for (token, tool, args) in [
        (
            "parity-owner",
            "get_task_references",
            json!({"taskId":"task","cwd":"/project"}),
        ),
        (
            session.token.as_str(),
            "get_task_references",
            json!({"taskId":"","cwd":"/project"}),
        ),
        (
            session.token.as_str(),
            "get_task_references",
            json!({"taskId":"task","cwd":""}),
        ),
        (
            session.token.as_str(),
            "update_task_references",
            json!({"taskId":"","cwd":"/project","expectedTaskRevision":0,"remove":[{"kind":"ticket","id":"X"}]}),
        ),
        (
            session.token.as_str(),
            "update_task_references",
            json!({"taskId":"task","cwd":"/project","upsert":[{"kind":"ticket","id":"X"}]}),
        ),
        (
            session.token.as_str(),
            "update_task_references",
            json!({"taskId":"task","cwd":"/project","expectedTaskRevision":1}),
        ),
        (
            session.token.as_str(),
            "get_task_references",
            json!({"taskId":"task","cwd":"/project","callerSessionId":"victim"}),
        ),
    ] {
        let before = seen.lock().unwrap().len();
        let body = facade.call(token, tool, args).await;
        assert_eq!(body["result"]["isError"], true, "{body}");
        assert_eq!(seen.lock().unwrap().len(), before);
    }
    for scope in [Scope::View, Scope::Triage, Scope::Operator] {
        let token = auth::mint(&tokens, scope, "session:tier").unwrap();
        let tools = facade.list(&token.token).await;
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in [
            "get_task_references",
            "update_task_references",
            "decide_workflow_step",
        ] {
            assert_eq!(names.contains(&name), scope == Scope::Operator);
        }
        for name in names {
            if name == "update_task_references" {
                continue;
            }
            assert!(
                !["waive", "skip_step", "edit_task", "update_task", "set_task"]
                    .iter()
                    .any(|p| name.contains(p)),
                "{name}"
            );
        }
    }
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn project_status_keeps_sorted_partial_rows_and_proves_concurrent_fanout() {
    let config = Arc::new(Mutex::new(
        json!({"projects":{"/d":{},"/b":{},"/a":{},"/c":{}," ":{}}}),
    ));
    let reads = Arc::new(AtomicUsize::new(0));
    let config_copy = config.clone();
    let read_count = reads.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let options=Options::default().handler("config.get",move|_,_|{let config=config_copy.clone();let count=read_count.clone();async move{count.fetch_add(1,Ordering::SeqCst);Ok(config.lock().unwrap().clone())}})
        .handler("git.status",move|_,p|{let barrier=barrier.clone();async move{
            let dir=p["cwd"].as_str().unwrap();if ["/a","/b","/c","/d"].contains(&dir){barrier.wait().await;}
            match dir{"/a"=>Ok(json!({"branch":"work","upstream":"origin/work","ahead":3,"behind":1,"files":[{"path":"a"},{"path":"b"}]})),"/b"=>Ok(json!({"branch":null,"upstream":null,"files":[]})),"/d"=>anyhow::bail!("not a git checkout"),_=>Ok(json!({"branch":"main","files":[]}))}
        }});
    let facade = Facade::start(options).await;
    let body = success(
        &facade
            .call("parity-owner", "project_status", json!({}))
            .await,
    );
    let rows = body["projects"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["dir"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["/a", "/b", "/c", "/d"]
    );
    assert_eq!(rows[0]["unpushed"], 3);
    assert_eq!(rows[0]["behind"], 1);
    assert_eq!(rows[0]["changedFiles"], 2);
    assert_eq!(rows[0]["dirty"], true);
    assert!(rows[1].get("unpushed").is_none());
    assert!(rows[1].get("upstream").is_none());
    assert!(rows[1].get("branch").is_none());
    assert_eq!(rows[1]["dirty"], false);
    assert!(
        rows[3]["error"]
            .as_str()
            .unwrap()
            .contains("not a git checkout")
    );
    let explicit = success(
        &facade
            .call(
                "parity-owner",
                "project_status",
                json!({"dirs":["/explicit","  "]}),
            )
            .await,
    );
    assert_eq!(explicit["projects"].as_array().unwrap().len(), 1);
    assert_eq!(explicit["projects"][0]["dir"], "/explicit");
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    for missing in [json!({}), json!({"projects":{}}), json!("unreadable")] {
        *config.lock().unwrap() = missing;
        let body = facade
            .call("parity-owner", "project_status", json!({}))
            .await;
        assert_eq!(body["result"]["isError"], true);
        assert!(
            body["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no projects are configured")
        );
    }
    facade.close();
}
#[tokio::test]
async fn respawn_clones_exact_task_and_canonical_selection_without_duplicate_delivery() {
    let snapshot = Arc::new(Mutex::new(
        json!({"cwd":"/repo/worktree","label":"parser","provider":"claude","parentSessionId":"prior-manager","livePermissionMode":"default","settings":{"model":"stale-model","permissionMode":"bypassPermissions","effort":"high"},"requestedSelection":{"model":"opus","contextWindow":1000000},"routing":{"role":"reviewer","capability":"reviewer","decisionId":"stale-decision"},"resultSchema":{"type":"object","required":["commit"]}}),
    ));
    let conversation = Arc::new(Mutex::new(
        json!({"items":[{"kind":"assistant_text","text":"booting"},{"kind":"user_message","text":" original task\nexactly "},{"kind":"user_message","text":"later task"}]}),
    ));
    let behavior = Arc::new(Mutex::new("confirmed".to_string()));
    let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let facade = Facade::start(Options::default()).await;
    let captured_snapshot = snapshot.clone();
    let captured_conversation = conversation.clone();
    let captured_seen = seen.clone();
    let captured_behavior = behavior.clone();
    let worker=provider(&facade,&["sessions.snapshot","sessions.conversation","config.get","agents.spawn","agents.sendMessage"],move|method,params|{
            captured_seen.lock().unwrap().push((method.into(),params));
            let behavior=captured_behavior.lock().unwrap().clone();
            match method{
                "sessions.snapshot"=>Ok(captured_snapshot.lock().unwrap().clone()),"sessions.conversation"=>Ok(captured_conversation.lock().unwrap().clone()),"config.get"=>Ok(json!({"claude":{"skipPermissionsDefault":true}})),
                "agents.spawn"=>match behavior.as_str(){"spawn-fails"=>anyhow::bail!("spawn refused"),"bare"=>Ok(json!("successor")),"legacy"|"send-fails"=>Ok(json!({"sessionId":"successor"})),"missing-id"=>Ok(json!({"messageQueued":true})),"no-history"=>Ok(json!({"sessionId":"successor","messageQueued":true})),_=>Ok(json!({"sessionId":"successor","messageQueued":true,"taskId":"task","dispatchId":"dispatch"}))},
                "agents.sendMessage" if behavior=="send-fails"=>anyhow::bail!("delivery acknowledgement lost"),"agents.sendMessage"=>Ok(json!({"ok":true})),_=>unreachable!()
            }
    }).await;
    let args = json!({"sessionId":"predecessor","amendment":"only repair parser"});
    let body = success(
        &facade
            .call("parity-owner", "respawn_with", args.clone())
            .await,
    );
    assert_eq!(body["taskId"], "task");
    assert_eq!(body["dispatchId"], "dispatch");
    assert_eq!(body["clonedFrom"], "predecessor");
    let spawn = seen
        .lock()
        .unwrap()
        .iter()
        .find(|(m, _)| m == "agents.spawn")
        .unwrap()
        .1
        .clone();
    for (key, value) in [
        ("cwd", json!("/repo/worktree")),
        ("provider", json!("claude")),
        ("parentSessionId", json!("prior-manager")),
        ("effort", json!("high")),
        ("label", json!("parser (redispatch)")),
        ("role", json!("reviewer")),
        ("capability", json!("reviewer")),
        ("retrySourceSessionId", json!("predecessor")),
        ("model", json!("opus[1m]")),
        ("modelIdentity", json!("opus")),
        ("contextWindow", json!(1000000)),
        ("skipPermissions", json!(false)),
    ] {
        assert_eq!(spawn[key], value, "{key}: {spawn}");
    }
    let task = spawn["message"].as_str().unwrap();
    assert!(task.starts_with(" original task\nexactly "));
    assert!(task.ends_with("only repair parser"));
    assert!(task.contains("CORRECTION") && task.contains("supersedes"));
    assert!(!task.contains("later task"));
    assert!(spawn.get("decisionId").is_none());
    assert_eq!(spawn["resultSchema"]["required"], json!(["commit"]));
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "agents.sendMessage")
    );
    for (mode, is_error, spawn_count, send_count) in [
        ("legacy", false, 1, 1),
        ("bare", false, 1, 1),
        ("no-history", false, 1, 0),
        ("send-fails", true, 1, 1),
        ("spawn-fails", true, 1, 0),
        ("missing-id", true, 1, 0),
    ] {
        *behavior.lock().unwrap() = mode.into();
        seen.lock().unwrap().clear();
        let result = facade
            .call("parity-owner", "respawn_with", args.clone())
            .await;
        assert_eq!(
            result["result"]["isError"] == true,
            is_error,
            "{mode}: {result}"
        );
        if !is_error {
            let body = success(&result);
            assert_eq!(body["sessionId"], "successor");
            assert!(body.get("taskId").is_none());
            assert!(body.get("dispatchId").is_none());
        }
        let calls = seen.lock().unwrap();
        assert_eq!(
            calls.iter().filter(|(m, _)| m == "agents.spawn").count(),
            spawn_count
        );
        assert_eq!(
            calls
                .iter()
                .filter(|(m, _)| m == "agents.sendMessage")
                .count(),
            send_count
        );
        for (_, p) in calls.iter().filter(|(m, _)| m == "agents.sendMessage") {
            assert_eq!(p["sessionId"], "successor");
            assert_eq!(p["text"], spawn["message"]);
        }
    }
    *behavior.lock().unwrap() = "no-history".into();
    seen.lock().unwrap().clear();
    let overridden=success(&facade.call("parity-owner","respawn_with",json!({"sessionId":"predecessor","amendment":"start clean","model":"sonnet","label":"fresh","cwd":"/repo","worktree":true,"toolScope":"view","role":"diagnostician","trackTask":false})).await);
    assert_eq!(overridden["taskTracking"], false);
    let spawn = seen
        .lock()
        .unwrap()
        .iter()
        .find(|(m, _)| m == "agents.spawn")
        .unwrap()
        .1
        .clone();
    assert_eq!(spawn["modelIdentity"], "sonnet");
    assert!(spawn.get("contextWindow").is_none());
    assert_eq!(spawn["role"], "diagnostician");
    assert_eq!(spawn["label"], "fresh");
    assert_eq!(spawn["worktree"], true);
    assert_eq!(spawn["cwd"], "/repo");
    assert!(spawn.get("retrySourceSessionId").is_none());
    assert!(spawn.get("capability").is_none());
    for bad in [
        json!({"sessionId":"predecessor","amendment":" "}),
        json!({"sessionId":" ","amendment":"redo"}),
    ] {
        seen.lock().unwrap().clear();
        assert_eq!(
            facade.call("parity-owner", "respawn_with", bad).await["result"]["isError"],
            true
        );
        assert!(seen.lock().unwrap().is_empty());
    }
    *conversation.lock().unwrap() = json!({"items":[]});
    seen.lock().unwrap().clear();
    assert_eq!(
        facade.call("parity-owner", "respawn_with", args).await["result"]["isError"],
        true
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "agents.spawn")
    );
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn spawn_defaults_and_legacy_grant_metadata_preserve_the_provider_wire() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let plain = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    let legacy = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    auth::update_records(&tokens, |records| {
        let record = records
            .iter_mut()
            .find(|r| r.token == legacy.token)
            .unwrap();
        record
            .metadata
            .insert("profilesAllowed".into(), json!(["different-profile"]));
        record.metadata.insert("yoloAllowed".into(), json!(true));
        Ok(())
    })
    .unwrap();
    let config = Arc::new(Mutex::new(
        json!({"claude":{"defaultModel":"opus[1m]","skipPermissionsDefault":true}}),
    ));
    let current = config.clone();
    let mut options = Options::default().handler("config.get", move |_, _| {
        let current = current.clone();
        async move { Ok(current.lock().unwrap().clone()) }
    });
    options.scoped_tokens = Some(tokens);
    let facade = Facade::start(options).await;
    let worker = provider(&facade, &["agents.spawn"], |_, params| {
        Ok(json!({"sessionId":"worker","params":params}))
    })
    .await;
    assert_eq!(
        facade.list(&plain.token).await,
        facade.list(&legacy.token).await,
        "obsolete per-token grants cannot change the tier catalog"
    );
    for token in [&plain.token, &legacy.token] {
        let p = success(
            &facade
                .call(
                    token,
                    "spawn_agent",
                    json!({"profileId":"selected-profile","parentSessionId":"forged"}),
                )
                .await,
        )["params"]
            .clone();
        assert_eq!(p["model"], "opus[1m]");
        assert_eq!(p["modelIdentity"], "opus");
        assert_eq!(p["contextWindow"], 1000000);
        assert_eq!(p["skipPermissions"], true);
        assert_eq!(p["profileId"], "selected-profile");
        assert_eq!(p["parentSessionId"], "manager");
        assert_eq!(p["dispatchOwnerSessionId"], "manager");
        for key in ["profileGranted", "yoloGranted", "skipPermissionsGranted"] {
            assert!(p.get(key).is_none());
        }
        let p = success(
            &facade
                .call(
                    token,
                    "spawn_agent",
                    json!({"model":"haiku","skipPermissions":false}),
                )
                .await,
        )["params"]
            .clone();
        assert_eq!(p["model"], "haiku");
        assert_eq!(p["skipPermissions"], false);
        assert!(p.get("contextWindow").is_none());
    }
    for provider_name in ["codex", "copilot", "opencode", "pi"] {
        let p = success(
            &facade
                .call(
                    &plain.token,
                    "spawn_agent",
                    json!({"provider":provider_name}),
                )
                .await,
        )["params"]
            .clone();
        assert!(p.get("model").is_none(), "{p}");
        if provider_name == "codex" {
            assert_eq!(p["contextWindow"], 1000000);
        } else {
            assert!(p.get("contextWindow").is_none());
        }
    }
    for args in [
        json!({"provider":"claude"}),
        json!({"provider":"  CLAUDE "}),
    ] {
        let p = success(&facade.call(&plain.token, "spawn_agent", args).await)["params"].clone();
        assert_eq!(p["model"], "opus[1m]");
    }
    let p = success(
        &facade
            .call(
                &plain.token,
                "spawn_agent",
                json!({"provider":"codex","model":"gpt-5.1-codex","contextWindow":400000}),
            )
            .await,
    )["params"]
        .clone();
    assert_eq!(p["modelIdentity"], "gpt-5.1-codex");
    assert_eq!(p["contextWindow"], 400000);
    for cfg in [
        json!({"claude":{"skipPermissionsDefault":false,"defaultPermissionMode":"bypassPermissions"}}),
        json!({"claude":{"skipPermissionsDefault":false,"defaultPermissionMode":"yolo"}}),
    ] {
        *config.lock().unwrap() = cfg;
        assert_eq!(
            success(&facade.call(&plain.token, "spawn_agent", json!({})).await)["params"]["skipPermissions"],
            true
        );
    }
    *config.lock().unwrap() = json!({"claude":{}});
    let p = success(&facade.call(&legacy.token, "spawn_agent", json!({})).await)["params"].clone();
    assert_eq!(p["skipPermissions"], false);
    assert!(p.get("model").is_none());
    for (args, code) in [
        (
            json!({"provider":"opencode","contextWindow":1000000}),
            "unsupported-context-window",
        ),
        (
            json!({"provider":"claude","model":"opus[1m]","modelIdentity":"sonnet","contextWindow":1000000}),
            "conflicting-model-identity",
        ),
    ] {
        let result = facade.call(&plain.token, "spawn_agent", args).await;
        assert_eq!(result["result"]["isError"], true);
        assert!(
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(code),
            "{result}"
        );
    }
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn routing_forecast_fields_reach_weighted_selection_and_help_tracks_live_tiers() {
    use workspacer_hub::services::routing::RoutingService;
    let root = tempfile::tempdir().unwrap();
    let service = Arc::new(RoutingService::open(root.path().into()).unwrap());
    let routing = service.clone();
    let tokens = root.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "viewer").unwrap();
    let mut options = Options::default().handler("routing.select", move |_, p| {
        let routing = routing.clone();
        async move { routing.select(p, &json!({}), 0) }
    });
    options.scoped_tokens = Some(tokens);
    let facade = Facade::start(options).await;
    let worker = provider(
        &facade,
        &["agents.spawn", "fleetWorkflows.request"],
        |_, _| Ok(json!({})),
    )
    .await;
    let input = json!({"role":"implementer","expectedWork":[{"phase":"implementation","count":2},{"phase":"review","count":4},{"phase":"haruspicy","count":3}]});
    let choice = success(&facade.call("parity-owner", "select_model", input).await);
    assert_eq!(choice["demand"]["units"], 16.0);
    assert_eq!(choice["demand"]["known"], false);
    assert_eq!(choice["demand"]["unweightedPhases"], json!(["haruspicy"]));
    let listed = facade.list("parity-owner").await;
    let schema = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "select_model")
        .unwrap()["inputSchema"]
        .to_string();
    for field in ["expectedWork", "phase", "count"] {
        assert!(schema.contains(field));
    }
    let help = facade
        .call("parity-owner", "help", json!({"topic":"  ROUTING "}))
        .await;
    let text = help["result"]["content"][0]["text"].as_str().unwrap();
    for phrase in [
        "select_model",
        "decisionId",
        "eligible:false",
        "escalationScrubbed",
        "resumeSessionId",
        "effortStep",
        "no launchable model",
        "unknown rather than unavailable",
        "docs/limit-aware-routing.md",
        "Plain spawn_agent accepts an explicit provider/model",
        "dispatch_workflow_step already performs",
    ] {
        assert!(text.contains(phrase), "missing {phrase}");
    }
    let help = facade
        .call("parity-owner", "help", json!({"topic":"workflows"}))
        .await;
    let text = help["result"]["content"][0]["text"].as_str().unwrap();
    for phrase in [
        "get_task_references",
        "update_task_references",
        "expectedTaskRevision",
        "Azure DevOps",
        "unverified references",
        "ADDITIVE",
        "standalone",
        "embedded",
    ] {
        assert!(text.contains(phrase), "missing {phrase}");
    }
    let help = facade
        .call(&view.token, "help", json!({"topic":"routing"}))
        .await;
    assert!(
        help["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown topic")
    );
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn status_summary_federation_never_substitutes_local_lookalikes_or_hides_denial() {
    use workspacer_hub::{client::Client, federation::Peer};
    fn summary(reason: &str) -> Value {
        json!({"contract":"agent-status-summary/v1","status":"unavailable","reason":reason,"activity":null,"progress":null,"blocker":null,"nextStep":null,"earliestRetainedTask":null,"latestExplicitProgress":null,"source":null,"provider":null,"model":null,"cached":false,"unknowns":[reason]})
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut peer_options =
        Options::default().handler("agents.summarizeStatus", move |_, params| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert!(params.get("hub").is_none());
                if params["sessionId"] == "denied" {
                    anyhow::bail!("permission denied: no provider for agents.summarizeStatus")
                }
                Ok(summary("peer-only"))
            }
        });
    peer_options.token = "summary-peer".into();
    peer_options.listen = Some("127.0.0.1:0".parse().unwrap());
    let peer = Hub::start(peer_options).unwrap();
    let peer_addr = peer.ready().await.unwrap().unwrap();
    let mut old_options = Options::default();
    old_options.token = "summary-old".into();
    old_options.listen = Some("127.0.0.1:0".parse().unwrap());
    let old = Hub::start(old_options).unwrap();
    let old_addr = old.ready().await.unwrap().unwrap();
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "summary-reader").unwrap();
    let mut options = Options::default().handler("agents.summarizeStatus", |_, _| async {
        Ok(summary("LOCAL-LOOKALIKE"))
    });
    options.scoped_tokens = Some(tokens.clone());
    options.federation_peers = vec![
        Peer {
            name: "peer".into(),
            url: format!("ws://{peer_addr}/bus"),
            token: "summary-peer".into(),
            dispatch: false,
        },
        Peer {
            name: "old".into(),
            url: format!("ws://{old_addr}/bus"),
            token: "summary-old".into(),
            dispatch: false,
        },
    ];
    let facade = Facade::start(options).await;
    let bus = Client::connect(&facade.hub.handle()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let peers = bus.call("federation.peers", json!({})).await.unwrap();
            if peers
                .as_array()
                .is_some_and(|p| p.len() == 2 && p.iter().all(|p| p["connected"] == true))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for scope in [Scope::View, Scope::Triage, Scope::Operator] {
        let token = auth::mint(&tokens, scope, "summary-tier").unwrap();
        assert!(
            facade
                .list(&token.token)
                .await
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "summarize_agent_status")
        );
    }
    let remote = success(
        &facade
            .call(
                &view.token,
                "summarize_agent_status",
                json!({"hub":"peer","sessionId":"same-id"}),
            )
            .await,
    );
    assert_eq!(remote["reason"], "peer-only");
    let unsupported = success(
        &facade
            .call(
                &view.token,
                "summarize_agent_status",
                json!({"hub":"old","sessionId":"same-id"}),
            )
            .await,
    );
    assert_eq!(unsupported["reason"], "desktop-summary-unavailable");
    let denied = facade
        .call(
            &view.token,
            "summarize_agent_status",
            json!({"hub":"peer","sessionId":"denied"}),
        )
        .await;
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(denied["result"]["content"][0]["text"], "permission denied");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    bus.close();
    facade.close();
    peer.shutdown().unwrap();
    old.shutdown().unwrap();
}
#[tokio::test]
async fn progress_schema_tiers_non_session_identity_and_provider_refusal_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let mut options = Options::default().handler("agents.reportProgress", |_, p| async move {
        if p["callerSessionId"].as_str().unwrap_or("").is_empty() {
            anyhow::bail!(
                "report_progress: the host could not identify your session from your credential"
            )
        }
        Ok(p)
    });
    options.scoped_tokens = Some(tokens.clone());
    let facade = Facade::start(options).await;
    for scope in [Scope::View, Scope::Triage, Scope::Operator] {
        let session = auth::mint(&tokens, scope, "session:worker").unwrap();
        let listed = facade.list(&session.token).await;
        let tool = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "report_progress")
            .unwrap();
        assert!(
            !tool["inputSchema"]
                .to_string()
                .to_ascii_lowercase()
                .contains("session")
        );
        assert!(
            scope
                .methods()
                .iter()
                .any(|p| workspacer_hub::protocol::matches(p, "agents.reportProgress"))
        );
        let body = success(
            &facade
                .call(
                    &session.token,
                    "report_progress",
                    json!({"note":"phase landed","needsDecision":true}),
                )
                .await,
        );
        assert_eq!(
            body,
            json!({"note":"phase landed","needsDecision":true,"callerSessionId":"worker"})
        );
    }
    let non_session = auth::mint(&tokens, Scope::Operator, "plugin:shiplight").unwrap();
    for token in ["parity-owner", non_session.token.as_str()] {
        let refused = facade
            .call(token, "report_progress", json!({"note":"phase landed"}))
            .await;
        assert_eq!(refused["result"]["isError"], true);
        assert!(
            refused["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("could not identify your session")
        );
    }
    facade.close();
}
#[tokio::test]
async fn actual_wildcard_facade_pins_health_and_mcp_hosts_to_the_accepting_socket() {
    let mut options =
        Options::default().handler("app.getCwd", |_, _| async { Ok(json!("/fixture")) });
    options.control_plane_only = true;
    options.token = "host-pin".into();
    options.mcp_listen = Some("0.0.0.0:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let port = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(a),
            ..
        } => a.port(),
        _ => panic!("MCP listener"),
    };
    let http = reqwest::Client::new();
    for host in [
        "evil.example.com",
        "evil.example.com:7897",
        "127.0.0.1.evil.tld:7897",
        "0.0.0.0",
        "0.0.0.0:7897",
        "[::]:7897",
    ] {
        for path in ["/health", "/mcp"] {
            let response = http
                .get(format!("http://127.0.0.1:{port}{path}"))
                .header("host", host)
                .bearer_auth("host-pin")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 403, "{host} {path}");
        }
    }
    for host in [
        "localhost",
        "LOCALHOST:7897",
        "127.0.0.1",
        "127.0.0.2:7897",
        "[::1]:7897",
    ] {
        let response = http
            .get(format!("http://127.0.0.1:{port}/health"))
            .header("host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{host}");
    }
    let missing = result(
        rpc(
            &http,
            &format!("127.0.0.1:{port}"),
            "host-pin",
            "tools/call",
            json!({"name":"list_agents","arguments":{}}),
        )
        .await,
    )
    .await;
    assert_eq!(missing["result"]["isError"], true);
    assert!(
        missing["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no provider")
    );
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn composed_workflow_inputs_cannot_forge_host_metadata_or_start_unbounded_context_reads() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let session = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    let mut options = Options::default();
    options.scoped_tokens = Some(tokens.clone());
    let facade = Facade::start(options).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let worker = provider(
        &facade,
        &["fleetWorkflows.request", "agents.spawn"],
        move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(json!({}))
        },
    )
    .await;
    for key in [
        "callerSessionId",
        "parentSessionId",
        "role",
        "model",
        "capability",
        "template",
        "hub",
        "resumeSessionId",
    ] {
        let mut args =
            json!({"taskId":"task","cwd":"/repo","stepId":"review","expectedTaskRevision":4});
        args[key] = json!("forged");
        let result = facade
            .call(&session.token, "dispatch_workflow_step", args)
            .await;
        assert_eq!(result["result"]["isError"], true, "{key}: {result}");
    }
    for args in [
        json!({"tasks":[{"taskId":"1","cwd":"/r"},{"taskId":"2","cwd":"/r"},{"taskId":"3","cwd":"/r"},{"taskId":"4","cwd":"/r"},{"taskId":"5","cwd":"/r"}]}),
        json!({"tasks":[{"taskId":"same","cwd":"/r"},{"taskId":"same","cwd":"/r"}]}),
        json!({"tasks":[{"taskId":"id","cwd":""}]}),
        json!({"callerSessionId":"foreign"}),
    ] {
        assert_eq!(
            facade.call(&session.token, "manager_context", args).await["result"]["isError"],
            true
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for scope in [Scope::View, Scope::Triage, Scope::Operator] {
        let token = auth::mint(&tokens, scope, "session:scoped").unwrap();
        let list = facade.list(&token.token).await;
        for name in ["dispatch_workflow_step", "manager_context"] {
            assert_eq!(
                list.as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == name),
                scope == Scope::Operator
            );
        }
    }
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn typed_legacy_wire_omits_value_zeros_but_keeps_pointer_values_and_opaque_objects() {
    let facade = Facade::start(Options::default()).await;
    let worker = provider(
        &facade,
        &[
            "agents.notifyWhen",
            "terminals.create",
            "sessions.terminalResize",
            "sessions.terminalInput",
            "claude.answer",
            "routing.select",
            "config.save",
            "library.save",
        ],
        |method, params| Ok(json!({"method":method,"params":params})),
    )
    .await;
    for (tool, args, expected) in [
        (
            "notify_when",
            json!({"sessionId":"worker","contextUsedPct":80,"tokens":0,"usd":0,"idleSeconds":0,"notifySessionId":""}),
            json!({"sessionId":"worker","contextUsedPct":80}),
        ),
        (
            "create_terminal",
            json!({"shell":"","cwd":"","cols":0,"rows":0}),
            json!({}),
        ),
        (
            "terminal_resize",
            json!({"sessionId":"s","cols":0,"rows":0}),
            json!({"sessionId":"s","cols":0,"rows":0}),
        ),
        (
            "terminal_input",
            json!({"sessionId":"s","data":""}),
            json!({"sessionId":"s","data":""}),
        ),
        (
            "answer",
            json!({"sessionId":"s","option":0,"text":"","answers":[]}),
            json!({"sessionId":"s","option":0,"text":""}),
        ),
        (
            "select_model",
            json!({"role":"scout","forecastDemandBeforeResetPct":0,"requireIndependentFamily":false,"expectedWork":[{"phase":"implementation","count":0}]}),
            json!({"role":"scout","forecastDemandBeforeResetPct":0,"expectedWork":[{"phase":"implementation","count":0}]}),
        ),
        (
            "save_config",
            json!({"ui":{"guiFontScale":0},"pluginSettings":{"enabled":false,"empty":"","array":[]}}),
            json!({"ui":{"guiFontScale":0},"pluginSettings":{"enabled":false,"empty":"","array":[]}}),
        ),
        (
            "save_library",
            json!({"scope":"global","kind":"prompt","id":"item","body":"","extra":{"false":false,"zero":0,"empty":{},"null":null}}),
            json!({"scope":"global","kind":"prompt","id":"item","body":"","extra":{"false":false,"zero":0,"empty":{},"null":null}}),
        ),
    ] {
        let result = success(&facade.call("parity-owner", tool, args).await);
        assert_eq!(result["params"], expected, "{tool}: {result}");
    }
    facade.close();
    worker.await.unwrap();
}
#[tokio::test]
async fn explicit_null_context_remains_provider_default_across_canonical_mcp_spawns() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let manager = auth::mint(&tokens, Scope::Operator, "session:manager").unwrap();
    let mut options = Options::default();
    options.scoped_tokens = Some(tokens);
    let facade = Facade::start(options).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let calls = seen.clone();
    let worker=provider(&facade,&["config.get","agents.spawn","sessions.snapshot","sessions.conversation","claude.setModel","fleetWorkflows.request","agents.notifyWhen"],move|method,params|{
        calls.lock().unwrap().push((method.to_owned(),params.clone()));
        match method{
            "config.get"=>Ok(json!({"claude":{}})),
            "sessions.snapshot"=>Ok(json!({"provider":"codex","cwd":"/repo","requestedSelection":{"model":"gpt-fixture","contextWindow":400000}})),
            "sessions.conversation"=>Ok(json!({"items":[{"kind":"user_message","text":"original task"}]})),
            "fleetWorkflows.request"=>Ok(json!({"ok":true,"dispatch":{"taskId":"task","cwd":"/repo","stepId":"implement","expectedTaskRevision":0,"role":"implementer","stage":"implement","template":"ship-task","toolScope":"operator"}})),
            "agents.spawn"=>Ok(json!({"sessionId":"successor","messageQueued":true,"params":params})),
            "claude.setModel"|"agents.notifyWhen"=>Ok(params),_=>panic!("unexpected composed call")
        }
    }).await;
    for (args, window) in [
        (json!({"provider":"codex"}), json!(1000000)),
        (
            json!({"provider":"codex","contextWindow":null}),
            Value::Null,
        ),
        (
            json!({"provider":"codex","modelIdentity":"gpt-fixture","contextWindow":null}),
            Value::Null,
        ),
        (
            json!({"provider":"codex","modelIdentity":"gpt-fixture"}),
            json!(1000000),
        ),
    ] {
        let receipt = success(&facade.call(&manager.token, "spawn_agent", args).await);
        assert_eq!(
            receipt["params"].get("contextWindow"),
            Some(&window),
            "{receipt}"
        );
    }
    for args in [
        json!({"sessionId":"prior","amendment":"redo","contextWindow":null}),
        json!({"sessionId":"prior","amendment":"redo","modelIdentity":"gpt-fixture","contextWindow":null}),
    ] {
        let receipt = success(&facade.call(&manager.token, "respawn_with", args).await);
        assert_eq!(receipt["params"].get("contextWindow"), Some(&Value::Null));
        assert_eq!(receipt["params"]["modelIdentity"], "gpt-fixture");
    }
    let switched = success(
        &facade
            .call(
                &manager.token,
                "set_model",
                json!({"sessionId":"s","modelIdentity":"gpt-fixture","contextWindow":null}),
            )
            .await,
    );
    assert_eq!(switched.get("contextWindow"), Some(&Value::Null));
    let dispatched=success(&facade.call(&manager.token,"dispatch_workflow_step",json!({"taskId":"task","cwd":"/repo","stepId":"implement","expectedTaskRevision":0,"modelSelection":{"provider":"codex","model":"gpt-fixture","contextWindow":null}})).await);
    assert_eq!(
        dispatched["params"].get("contextWindow"),
        Some(&Value::Null)
    );
    for (tool, args) in [
        (
            "notify_when",
            json!({"sessionId":"worker","contextUsedPct":80,"tokens":null}),
        ),
        ("spawn_agent", json!({"provider":"codex","model":null})),
        (
            "spawn_agent",
            json!({"provider":"codex","contextWindow":0,"skipPermissions":false}),
        ),
    ] {
        let before = seen.lock().unwrap().len();
        let invalid = facade.call(&manager.token, tool, args).await;
        assert_eq!(invalid["result"]["isError"], true);
        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "invalid scalar null reached provider or config read"
        );
    }
    facade.close();
    worker.await.unwrap();
}
