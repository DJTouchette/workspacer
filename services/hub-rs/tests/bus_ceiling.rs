//! Retained ceiling assertions at the actual external-provider dispatch boundary.
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use workspacer_hub::{Hub, Options, auth, client::Client, protocol::Frame};

#[tokio::test]
async fn directory_ceiling_reaches_provider_as_safe_tuple_and_records_policy_remedy() {
    let root = tempfile::tempdir().unwrap();
    let current = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let mut policy = json!({"ceilings":{"default":{"max_capability":"cheap"}}});
    policy["ceilings"][current.to_str().unwrap()] = json!({"max_capability":"frontier_plus"});
    let locked = root.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    policy["ceilings"][locked.to_str().unwrap()] = json!({"max_capability":"balanced"});
    let bad = root.path().join("bad");
    std::fs::create_dir(&bad).unwrap();
    policy["ceilings"][bad.to_str().unwrap()] = json!({"max_capability":"frontierr"});
    std::fs::write(
        root.path().join("routing.yaml"),
        serde_yaml::to_string(&policy).unwrap(),
    )
    .unwrap();
    let tokens = root.path().join("tokens.json");
    let operator = auth::mint(&tokens, auth::Scope::Operator, "ceiling fixture").unwrap();
    let mut options = Options::default();
    options.control_plane_only = true;
    options.token = "ceiling-owner".into();
    options.scoped_tokens = Some(tokens);
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.config_dir = Some(root.path().into());
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let mut provider = hub.handle().connect().await.unwrap();
    provider.recv().await.unwrap();
    provider
        .send(Frame {
            methods: vec!["agents.spawn".into(), "fixture.barrier".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(provider.recv().await.unwrap().op, "registered");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let task = tokio::spawn(async move {
        while let Some(frame) = provider.recv().await {
            assert_eq!(frame.op, "call");
            if frame.method == "agents.spawn" {
                observed.fetch_add(1, Ordering::SeqCst);
            }
            provider
                .send(Frame {
                    id: frame.id,
                    result: frame.params,
                    ..Frame::op("result")
                })
                .unwrap();
        }
    });
    let client = Client::connect_remote(&format!("ws://{address}/bus"), &operator.token)
        .await
        .unwrap();
    let original = json!({"cwd":current,"provider":"claude","capability":"frontier_plus","model":"fable","effort":"max","role":"judge","decisionId":"unchanged","toolScope":"operator"});
    assert_eq!(
        client.call("agents.spawn", original.clone()).await.unwrap(),
        original
    );
    let mut paths = vec![
        Value::Null,
        json!(""),
        json!("."),
        json!("relative/missing"),
        json!(locked),
    ];
    #[cfg(unix)]
    {
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&locked, &alias).unwrap();
        paths.push(json!(alias));
    }
    for (index, cwd) in paths.into_iter().enumerate() {
        let mut params = original.clone();
        params["decisionId"] = json!(format!("clamp-{index}"));
        if cwd.is_null() {
            params.as_object_mut().unwrap().remove("cwd");
        } else {
            params["cwd"] = cwd.clone();
        }
        let result = client.call("agents.spawn", params.clone()).await.unwrap();
        assert_eq!(
            result["capability"],
            if index >= 4 { "balanced" } else { "cheap" }
        );
        assert_eq!(result["provider"], "claude");
        assert_ne!(result["model"], "fable");
        assert_eq!(result["model"], "sonnet");
        assert_eq!(
            result["effort"],
            if index >= 4 {
                json!("high")
            } else {
                Value::Null
            }
        );
        assert!(result["model"].as_str().is_some_and(|s| !s.is_empty()));
        for key in ["cwd", "role", "decisionId", "toolScope"] {
            assert_eq!(result[key], params[key]);
        }
        let scrub = result["escalationScrubbed"].as_array().unwrap();
        for key in ["capability", "model", "effort"] {
            assert!(scrub.contains(&json!(key)), "{result}");
        }
        let raw = std::fs::read_to_string(root.path().join("routing-decisions.jsonl")).unwrap();
        let row: Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(row["decisionId"], params["decisionId"]);
        assert_eq!(row["spawn"]["outcome"], "clamped");
        assert!(
            row["spawn"]["ceiling"]["because"][0]
                .as_str()
                .unwrap()
                .contains("routing.yaml")
        );
        if index >= 4 {
            assert_eq!(
                row["spawn"]["cwd"],
                json!(std::fs::canonicalize(&locked).unwrap())
            );
        }
    }
    for (id, params, reason) in [
        (
            "bad",
            json!({"cwd":bad,"capability":"balanced"}),
            "frontierr",
        ),
        (
            "unsupported",
            json!({"provider":"copilot","capability":"frontier_plus","model":"unknown"}),
            "explicitly requested provider",
        ),
    ] {
        let before = calls.load(Ordering::SeqCst);
        let mut params = params;
        params["decisionId"] = json!(id);
        assert!(
            client
                .call("agents.spawn", params)
                .await
                .unwrap_err()
                .to_string()
                .contains(reason)
        );
        client.call("fixture.barrier", json!({})).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before,
            "refusal reached provider"
        );
        let raw = std::fs::read_to_string(root.path().join("routing-decisions.jsonl")).unwrap();
        let row: Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(row["decisionId"], id);
        assert_eq!(row["spawn"]["outcome"], "refused");
        assert_eq!(row["spawn"]["ceiling"]["denied"], true);
    }
    let raw = std::fs::read_to_string(root.path().join("routing-decisions.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), calls.load(Ordering::SeqCst) + 2);
    client.close();
    hub.shutdown().unwrap();
    task.await.unwrap();
}
