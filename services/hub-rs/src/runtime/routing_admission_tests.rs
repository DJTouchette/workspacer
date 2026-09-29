//! Real external-provider admission records the one gate outcome before forwarding.
use super::*;
async fn next(connection: &mut Connection) -> Frame {
    tokio::time::timeout(Duration::from_secs(3), connection.recv())
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn external_spawn_audits_allow_clamp_and_refusal_once_without_logging_payload_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "fixture").unwrap();
    let routing =
        Arc::new(crate::services::routing::RoutingService::open(directory.path().into()).unwrap());
    let mut options = Options::default();
    options.control_plane_only = true;
    options.routing = Some(routing);
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut provider = hub.handle().connect().await.unwrap();
    next(&mut provider).await;
    provider
        .send(Frame {
            methods: vec!["agents.spawn".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(next(&mut provider).await.methods, vec!["agents.spawn"]);
    let mut caller = hub
        .handle()
        .connect_authenticated(operator.token.clone(), false)
        .await
        .unwrap();
    next(&mut caller).await;
    for (index,(decision,request,outcome)) in [
        ("allowed",json!({"provider":"codex","model":"gpt-5.6-terra","effort":"high","capability":"balanced"}),"allowed"),
        ("clamped",json!({"provider":"claude","model":"fable","capability":"frontier_plus","effort":"max"}),"clamped"),
        ("fresh",json!({"role":"reviewer","resumeSessionId":"existing-review-session"}),"refused"),
        ("exact",json!({"provider":"claude","model":"fable","capability":"frontier_plus","exactModel":true}),"refused"),
    ].into_iter().enumerate() {
        let mut request=request;
        request["cwd"]=json!(directory.path());request["decisionId"]=json!(decision);
        request["message"]=json!("SECRET_PROMPT_MUST_NOT_BE_LOGGED");
        request["env"]=json!({"API_KEY":"SECRET_ENV_MUST_NOT_BE_LOGGED"});
        request["callerTokenId"]=json!("FORGED_CREDENTIAL_MUST_NOT_BE_LOGGED");
        caller.send(Frame{id:decision.into(),method:"agents.spawn".into(),params:Some(request),..Frame::op("call")}).unwrap();
        if outcome=="refused" {
            let refusal=next(&mut caller).await;
            assert_eq!(refusal.id,decision);assert_eq!(refusal.op,"error");
            assert!(refusal.error.contains("routing.yaml"));
            if decision=="fresh" {assert!(refusal.error.contains("existing-review-session"));}
            if decision=="exact" {assert!(refusal.error.contains("no substitute was launched"));}
            provider.send(Frame{..Frame::op("subscribe")}).unwrap();
            assert_eq!(next(&mut provider).await.op,"subscribed","refused spawn reached provider");
        } else {
            let forwarded=next(&mut provider).await;
            assert_eq!(forwarded.op,"call");assert_eq!(forwarded.method,"agents.spawn");
            if outcome=="clamped" {
                assert_eq!(forwarded.params.as_ref().unwrap()["model"],"opus");
                assert_eq!(forwarded.params.as_ref().unwrap()["capability"],"frontier");
            }
            provider.send(Frame{id:forwarded.id,result:Some(json!({"ok":true})),..Frame::op("result")}).unwrap();
            assert_eq!(next(&mut caller).await.result,Some(json!({"ok":true})));
        }
        let raw=std::fs::read_to_string(directory.path().join("routing-decisions.jsonl")).unwrap();
        let rows:Vec<Value>=raw.lines().map(|line|serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(),index+1,"one record per attempted spawn");
        let row=rows.last().unwrap();assert_eq!(row["decisionId"],decision);
        assert_eq!(row["phase"],"routing");assert_eq!(row["spawn"]["outcome"],outcome);
        assert_eq!(row["spawn"]["callerScope"],"operator");
        assert_eq!(row["spawn"]["callerTokenId"],crate::auth::fingerprint(&operator.token));
        assert!(!raw.contains("MUST_NOT_BE_LOGGED"));assert!(!raw.contains(&operator.token));
        if decision=="fresh" {assert_eq!(row["spawn"]["ceiling"]["resumeRefused"],true);assert!(row["spawn"]["ceiling"]["freshCapability"].is_string());}
        if outcome=="refused" {assert_eq!(row["spawn"]["ceiling"]["denied"],true);}
        if outcome=="clamped" {assert_eq!(row["spawn"]["ceiling"]["capabilityRefused"],true);assert!(row["spawn"]["scrubbed"].as_array().unwrap().contains(&json!("model")));}
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn freshness_has_positive_floors_for_new_sessions_ordinary_resumes_and_absent_policy() {
    for policy in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("routing.yaml"),"roles:\n  reviewer: deep_reviewer\nceilings:\n  default: {max_capability: frontier_plus}\n").unwrap();
        let tokens = directory.path().join("tokens.json");
        let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "fixture").unwrap();
        let mut options = Options::default();
        options.control_plane_only = true;
        options.scoped_tokens = Some(tokens);
        if policy {
            options.routing = Some(Arc::new(
                crate::services::routing::RoutingService::open(directory.path().into()).unwrap(),
            ));
        }
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let mut provider = hub.handle().connect().await.unwrap();
        next(&mut provider).await;
        provider
            .send(Frame {
                methods: vec!["agents.spawn".into()],
                ..Frame::op("register")
            })
            .unwrap();
        next(&mut provider).await;
        let mut count = 0;
        for host in [false, true] {
            let mut caller = if host {
                hub.handle().connect().await.unwrap()
            } else {
                hub.handle()
                    .connect_authenticated(operator.token.clone(), false)
                    .await
                    .unwrap()
            };
            next(&mut caller).await;
            for (fields, fresh_resume) in [
                (
                    json!({"role":"reviewer","resumeSessionId":"role-old"}),
                    true,
                ),
                (
                    json!({"capability":"deep_reviewer","resumeSessionId":"cap-old"}),
                    true,
                ),
                (
                    json!({"role":"implementer","resumeSessionId":"ordinary-role"}),
                    false,
                ),
                (json!({"resumeSessionId":"ordinary-unlabelled"}), false),
                (
                    json!({"capability":"frontier","resumeSessionId":"ordinary-capability"}),
                    false,
                ),
                (
                    json!({"role":"reviewer","capability":"deep_reviewer"}),
                    false,
                ),
            ] {
                let mut params = fields.clone();
                params["cwd"] = json!(directory.path());
                params["provider"] = json!("codex");
                params["model"] = json!("gpt-5.6-terra");
                params["effort"] = json!("high");
                caller
                    .send(Frame {
                        id: "fresh-case".into(),
                        method: "agents.spawn".into(),
                        params: Some(params),
                        ..Frame::op("call")
                    })
                    .unwrap();
                if policy && fresh_resume {
                    let refused = next(&mut caller).await;
                    assert_eq!(refused.op, "error");
                    assert_eq!(refused.id, "fresh-case");
                    assert!(refused.error.contains("fresh"));
                    assert!(
                        refused
                            .error
                            .contains(fields["resumeSessionId"].as_str().unwrap())
                    );
                    provider.send(Frame::op("subscribe")).unwrap();
                    assert_eq!(
                        next(&mut provider).await.op,
                        "subscribed",
                        "refusal leaked a spawn to provider"
                    );
                } else {
                    let forwarded = next(&mut provider).await;
                    assert_eq!(forwarded.op, "call");
                    for (key, value) in fields.as_object().unwrap() {
                        assert_eq!(
                            &forwarded.params.as_ref().unwrap()[key],
                            value,
                            "policy={policy} host={host} {key}"
                        );
                    }
                    provider
                        .send(Frame {
                            id: forwarded.id,
                            result: Some(json!({"ok":true})),
                            ..Frame::op("result")
                        })
                        .unwrap();
                    assert_eq!(next(&mut caller).await.result, Some(json!({"ok":true})));
                }
                count += 1;
                if policy {
                    let rows: Vec<Value> =
                        std::fs::read_to_string(directory.path().join("routing-decisions.jsonl"))
                            .unwrap()
                            .lines()
                            .map(|line| serde_json::from_str(line).unwrap())
                            .collect();
                    assert_eq!(rows.len(), count);
                    let last = &rows.last().unwrap()["spawn"];
                    assert_eq!(last["ceiling"]["resumeRefused"], fresh_resume);
                    if fresh_resume {
                        assert_eq!(last["ceiling"]["freshCapability"], "deep_reviewer");
                        assert_eq!(last["outcome"], "refused");
                    } else {
                        assert_eq!(last["outcome"], "allowed");
                    }
                } else {
                    assert!(!directory.path().join("routing-decisions.jsonl").exists());
                }
            }
        }
        hub.shutdown().unwrap();
    }
}
