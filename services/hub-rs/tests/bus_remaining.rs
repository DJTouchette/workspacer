//! Assertion-level counterparts for retained Go bus tests. Real WebSockets,
//! temporary credentials and actor barriers; no provider processes are started.
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use workspacer_hub::{Handle, Hub, Options, auth, protocol::Event};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn send(socket: &mut Socket, frame: Value) {
    socket.send(Message::Text(frame.to_string())).await.unwrap();
}
async fn receive(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.expect("socket closed").unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("unexpected frame: {other:?}"),
            }
        }
    })
    .await
    .expect("bus response timed out")
}
async fn connect(address: SocketAddr, token: &str, peer: bool) -> (Socket, Value) {
    let mut url = url::Url::parse(&format!("ws://{address}/bus")).unwrap();
    url.query_pairs_mut().append_pair("token", token);
    if peer {
        url.query_pairs_mut().append_pair("peer", "1");
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    let hello = receive(&mut socket).await;
    assert_eq!(hello["op"], "hello");
    (socket, hello)
}
fn options(tokens: &std::path::Path) -> Options {
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "bus-fixture-owner".into();
    options.scoped_tokens = Some(tokens.into());
    let records: Vec<_> = ["view", "triage", "operator", "provider"]
        .into_iter()
        .map(|scope| auth::Record {
            token: format!("bus-fixture-{scope}"),
            scope: scope.into(),
            provides: (scope == "provider").then(|| vec!["*".into()]),
            ..Default::default()
        })
        .collect();
    auth::save(tokens, &records).unwrap();
    options
}

#[tokio::test]
async fn local_handlers_replace_count_once_and_keep_identity_errors_and_publications() {
    let root = tempfile::tempdir().unwrap();
    let handle = Arc::new(std::sync::OnceLock::<Handle>::new());
    let publisher = handle.clone();
    let options = options(&root.path().join("tokens.json"))
        .handler("fixture.replace", |_, _| async { Ok(json!("first")) })
        .handler("fixture.replace", |_, _| async { Ok(json!("second")) })
        .handler("fixture.error", |_, _| async {
            anyhow::bail!("exact local failure")
        })
        .handler("fixture.touch", move |_, _| {
            let publisher = publisher.clone();
            async move {
                publisher.get().unwrap().publish(Event::new(
                    "layout.changed",
                    "hub",
                    json!({"version":7}),
                ))?;
                Ok(json!({"ok":true}))
            }
        })
        .handler("push.subscribe", |caller, _| async move {
            Ok(json!({"scope":caller.scope,"trusted":caller.trusted,
                "host":caller.authenticated_host,"tokenId":caller.token_id}))
        });
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    assert!(handle.set(hub.handle()).is_ok());
    let (mut provider, _) = connect(address, "bus-fixture-owner", false).await;
    send(
        &mut provider,
        json!({"op":"register","methods":["fixture.replace","push.subscribe","fixture.remote"]}),
    )
    .await;
    assert_eq!(
        receive(&mut provider).await["methods"],
        json!(["fixture.remote"])
    );
    let (mut caller, _) = connect(address, "bus-fixture-owner", false).await;
    send(
        &mut caller,
        json!({"op":"call","id":"local","method":"fixture.replace"}),
    )
    .await;
    assert_eq!(
        receive(&mut caller).await,
        json!({"op":"result","id":"local","result":"second"})
    );
    send(
        &mut caller,
        json!({"op":"call","id":"err","method":"fixture.error"}),
    )
    .await;
    assert_eq!(
        receive(&mut caller).await,
        json!({"op":"error","id":"err","error":"exact local failure"})
    );
    let health = hub.handle().health().await.unwrap();
    let names = health["methodNames"].as_array().unwrap();
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "fixture.replace")
            .count(),
        1
    );
    assert!(names.contains(&json!("fixture.remote")) && names.contains(&json!("push.subscribe")));
    assert_eq!(health["methods"].as_u64().unwrap() as usize, names.len());
    assert!(
        names
            .windows(2)
            .all(|pair| pair[0].as_str() < pair[1].as_str())
    );
    send(
        &mut provider,
        json!({"op":"subscribe","topics":["layout.changed"]}),
    )
    .await;
    assert_eq!(receive(&mut provider).await["op"], "subscribed");
    send(
        &mut caller,
        json!({"op":"call","id":"touch","method":"fixture.touch"}),
    )
    .await;
    assert_eq!(receive(&mut caller).await["result"], json!({"ok":true}));
    let event = receive(&mut provider).await;
    assert_eq!(event["event"]["type"], "layout.changed");
    assert_eq!(event["event"]["data"], json!({"version":7}));
    assert!(!event["event"]["id"].as_str().unwrap().is_empty());
    assert!(chrono::DateTime::parse_from_rfc3339(event["event"]["time"].as_str().unwrap()).is_ok());
    for (token, peer, scope, trusted, owner) in [
        ("bus-fixture-triage", false, "triage", false, false),
        ("bus-fixture-triage", false, "triage", false, false),
        ("bus-fixture-operator", false, "operator", true, false),
        ("bus-fixture-owner", false, "operator", true, true),
        ("bus-fixture-owner", true, "operator", true, false),
    ] {
        let (mut socket, _) = connect(address, token, peer).await;
        send(&mut socket, json!({"op":"call","id":"identity","method":"push.subscribe","params":{"authenticatedHost":true}})).await;
        let got = receive(&mut socket).await;
        assert_eq!(
            got["result"],
            json!({"scope":scope,"trusted":trusted,"host":owner,"tokenId":auth::fingerprint(token)})
        );
        assert_ne!(got["result"]["tokenId"], token);
        socket.close(None).await.unwrap();
    }
    assert_eq!(auth::fingerprint(""), "");
    assert_ne!(auth::fingerprint("one"), auth::fingerprint("two"));
    caller.close(None).await.unwrap();
    provider.close(None).await.unwrap();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn scoped_calls_and_hello_are_exact_while_registered_unknown_methods_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let methods = [
        "agents.list",
        "sessions.snapshots",
        "sessions.transcript",
        "claude.approve",
        "agents.sendMessage",
        "push.subscribe",
        "claude.answer",
        "terminals.create",
        "git.push",
        "config.save",
        "future.unknownMethod",
    ];
    let mut options = options(&root.path().join("tokens.json"));
    for method in methods {
        options = options.handler(method, |_, _| async { Ok(json!({"ok":true})) });
    }
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    for (scope, allowed) in [
        (
            "view",
            vec!["agents.list", "sessions.snapshots", "sessions.transcript"],
        ),
        (
            "triage",
            vec![
                "agents.list",
                "sessions.snapshots",
                "sessions.transcript",
                "claude.approve",
                "agents.sendMessage",
                "push.subscribe",
            ],
        ),
        ("operator", methods.to_vec()),
    ] {
        let (mut socket, hello) = connect(address, &format!("bus-fixture-{scope}"), false).await;
        assert_eq!(hello["scope"], scope);
        let advertised = hello["methods"].as_array().unwrap();
        if scope == "operator" {
            assert_eq!(advertised, &vec![json!("*")]);
        } else {
            assert!(!advertised.contains(&json!("agents.spawn")));
        }
        for method in methods {
            send(
                &mut socket,
                json!({"op":"call","id":method,"method":method}),
            )
            .await;
            let reply = receive(&mut socket).await;
            assert_eq!(reply["id"], method);
            if allowed.contains(&method) {
                assert_eq!(
                    reply["result"],
                    json!({"ok":true}),
                    "{scope} {method}: {reply}"
                );
            } else {
                assert_eq!(reply["op"], "error");
                let error = reply["error"].as_str().unwrap();
                assert!(
                    error.contains("not authorized")
                        && error.contains(scope)
                        && error.contains(method),
                    "{reply}"
                );
            }
        }
        if scope != "operator" {
            send(
                &mut socket,
                json!({"op":"call","id":"spawn-denied","method":"agents.spawn"}),
            )
            .await;
            let denied = receive(&mut socket).await;
            assert_eq!(denied["id"], "spawn-denied");
            let error = denied["error"].as_str().unwrap();
            assert!(error.contains("not authorized") && error.contains(scope));
        }
        send(
            &mut socket,
            json!({"op":"register","methods":[format!("fixture.{scope}")]}),
        )
        .await;
        assert_eq!(
            receive(&mut socket).await["methods"],
            if scope == "operator" {
                json!(["fixture.operator"])
            } else {
                Value::Null
            }
        );
        socket.close(None).await.unwrap();
    }
    let failed =
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token=unknown")).await;
    assert!(
        matches!(failed, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status().as_u16()==401)
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn provider_publish_uses_declared_grant_even_when_registration_is_shadowed() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let mut options = options(&tokens).handler("layout.get", |caller, _| async move {
        Ok(json!({"scope":caller.scope,"trusted":caller.trusted,"host":caller.authenticated_host}))
    });
    options = options.handler("sessions.snapshots", |_, _| async { Ok(json!([])) });
    let mut records = auth::load(&tokens).unwrap();
    records
        .iter_mut()
        .find(|r| r.scope == "provider")
        .unwrap()
        .provides = Some(vec!["sessions.snapshots".into()]);
    records.push(auth::Record {
        token: "wide-provider-token".into(),
        scope: "provider".into(),
        provides: Some(vec!["*".into()]),
        ..Default::default()
    });
    auth::save(&tokens, &records).unwrap();
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let (mut wide, wide_hello) = connect(address, "wide-provider-token", false).await;
    assert_eq!(wide_hello["scope"], "provider");
    let registered = json!([
        "agents.list",
        "agents.spawn",
        "sessions.snapshot",
        "sessions.attachTerminal",
        "terminals.open",
        "claude.approve",
        "brain.info"
    ]);
    send(&mut wide, json!({"op":"register","methods":registered})).await;
    assert_eq!(receive(&mut wide).await["methods"], registered);
    wide.close(None).await.unwrap();
    let (mut node, hello) = connect(address, "bus-fixture-provider", false).await;
    assert_eq!(hello["scope"], "provider");
    assert_eq!(
        hello["methods"],
        json!(["layout.get", "plugins.prepareLaunch"])
    );
    send(&mut node,json!({"op":"register","methods":["sessions.snapshots","sessions.attachTerminal","terminals.open"]})).await;
    assert_eq!(receive(&mut node).await, json!({"op":"registered"}));
    send(
        &mut node,
        json!({"op":"call","id":"identity","method":"layout.get"}),
    )
    .await;
    assert_eq!(
        receive(&mut node).await["result"],
        json!({"scope":"provider","trusted":false,"host":false})
    );
    for method in [
        "nodes.wake",
        "agents.spawn",
        "claude.approve",
        "config.save",
        "jobs.upsert",
        "layout.set",
        "fs.write",
        "sessions.terminalInput",
        "plugins.install",
        "sessions.snapshots",
    ] {
        send(&mut node, json!({"op":"call","id":method,"method":method})).await;
        let got = receive(&mut node).await;
        assert_eq!(got["id"], method);
        let error = got["error"].as_str().unwrap();
        assert!(
            error.contains("not authorized") && error.contains("provider"),
            "{got}"
        );
    }
    let (mut watcher, _) = connect(address, "bus-fixture-owner", false).await;
    send(&mut watcher, json!({"op":"subscribe","topics":["*"]})).await;
    assert_eq!(receive(&mut watcher).await["op"], "subscribed");
    for topic in [
        "pty.bytes.secret",
        "facade.openTerminal",
        "layout.changed",
        "plugin.settings.changed",
        "example.clock.tick",
    ] {
        send(&mut node,json!({"op":"publish","event":{"type":topic,"source":"fixture","data":{"forged":true}}})).await;
        assert_eq!(receive(&mut node).await["op"], "error");
    }
    send(&mut node,json!({"op":"publish","event":{"type":"agent.snapshot","source":"fixture","data":{"sentinel":true}}})).await;
    let event = receive(&mut watcher).await;
    assert_eq!(event["event"]["type"], "agent.snapshot");
    assert_eq!(event["event"]["data"], json!({"sentinel":true}));
    // The positive sentinel traverses the same publisher and subscriber queues;
    // any incorrectly admitted prior forgery would have appeared before it.
    node.close(None).await.unwrap();
    watcher.close(None).await.unwrap();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn topic_limit_accepts_exact_boundary_and_rejects_both_oversized_operations_atomically() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(options(&root.path().join("tokens.json"))).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let (mut viewer, _) = connect(address, "bus-fixture-view", false).await;
    let huge: Vec<_> = (0..257).map(|i| format!("topic.{i}")).collect();
    send(&mut viewer, json!({"op":"subscribe","topics":huge})).await;
    assert_eq!(receive(&mut viewer).await["op"], "error");
    let mut boundary = huge[..255].to_vec();
    boundary.push("agent.*".into());
    send(&mut viewer, json!({"op":"subscribe","topics":boundary})).await;
    assert_eq!(receive(&mut viewer).await["topics"], json!(boundary));
    send(&mut viewer, json!({"op":"unsubscribe","topics":huge})).await;
    let refusal = receive(&mut viewer).await;
    assert_eq!(refusal["op"], "error");
    assert!(refusal["error"].as_str().unwrap().contains("unsubscribe"));
    send(
        &mut viewer,
        json!({"op":"unsubscribe","topics":["not.present"]}),
    )
    .await;
    assert_eq!(receive(&mut viewer).await["topics"], json!(boundary));
    hub.handle()
        .publish(Event::new(
            "agent.snapshot",
            "fixture",
            json!({"boundary":256}),
        ))
        .unwrap();
    assert_eq!(
        receive(&mut viewer).await["event"]["data"],
        json!({"boundary":256})
    );
    viewer.close(None).await.unwrap();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn network_listeners_require_credentials_while_embedded_owners_keep_local_identity() {
    for (bus, mcp) in [(true, false), (false, true), (true, true)] {
        let mut options = Options::default();
        if bus {
            options.listen = Some("127.0.0.1:0".parse().unwrap());
        }
        if mcp {
            options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
        }
        let error = match Hub::start(options) {
            Ok(_) => panic!("tokenless listener admitted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("explicit host token"), "{error}");
    }
    let hub=Hub::start(Options::default().handler("fixture.identity",|caller,_|async move {
        Ok(json!({"host":caller.authenticated_host,"trusted":caller.trusted,"scope":caller.scope,"tokenId":caller.token_id}))
    })).unwrap();
    hub.ready().await.unwrap();
    let client = workspacer_hub::client::Client::connect(&hub.handle())
        .await
        .unwrap();
    assert_eq!(
        client.call("fixture.identity", Value::Null).await.unwrap(),
        json!({"host":true,"trusted":true,"scope":"operator","tokenId":""})
    );
    client.close();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn plugin_ambient_calls_keep_paths_but_host_events_and_foreign_provider_names_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let options = options(&root.path().join("tokens.json"))
        .plugin_token(
            "plugin-fixture",
            "acme.tools",
            vec!["acme.tools.*".into(), "agents.*".into()],
        )
        .handler("fs.read", |_, params| async move { Ok(params) })
        .handler(
            "agents.reportProgress",
            |_, params| async move { Ok(params) },
        );
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let (mut plugin, hello) = connect(address, "plugin-fixture", false).await;
    assert!(hello.get("scope").is_none() && hello.get("methods").is_none());
    let path = root.path().join("outside-any-declared-root");
    send(
        &mut plugin,
        json!({"op":"call","id":"ambient","method":"fs.read","params":{"path":path}}),
    )
    .await;
    assert_eq!(receive(&mut plugin).await["result"], json!({"path":path}));
    send(
        &mut plugin,
        json!({"op":"register","methods":["acme.tools.echo","agents.spawn","foreign.echo"]}),
    )
    .await;
    assert_eq!(
        receive(&mut plugin).await["methods"],
        json!(["acme.tools.echo"])
    );
    send(&mut plugin, json!({"op":"subscribe","topics":["*"]})).await;
    assert_eq!(receive(&mut plugin).await["op"], "subscribed");
    hub.handle()
        .publish(Event::new(
            "plugin.settings.changed",
            "fixture",
            json!({"secret":true}),
        ))
        .unwrap();
    hub.handle()
        .publish(Event::new(
            "pty.bytes.session",
            "fixture",
            json!({"ambient":true}),
        ))
        .unwrap();
    let allowed = receive(&mut plugin).await;
    assert_eq!(allowed["event"]["type"], "pty.bytes.session");
    assert_eq!(allowed["event"]["data"], json!({"ambient":true}));
    for topic in ["plugin.loaded", "layout.changed", "agent.snapshot"] {
        send(
            &mut plugin,
            json!({"op":"publish","event":{"type":topic,"source":"plugin"}}),
        )
        .await;
        assert_eq!(receive(&mut plugin).await["op"], "error");
    }
    send(
        &mut plugin,
        json!({"op":"publish","event":{"type":"acme.tools.tick","source":"plugin","data":7}}),
    )
    .await;
    assert_eq!(receive(&mut plugin).await["event"]["data"], 7);
    // Actual routing sees a scope-derived attribution; the payload is not proof.
    for (token, claimed) in [
        ("bus-fixture-view", false),
        ("plugin-fixture", false),
        ("bus-fixture-operator", true),
        ("bus-fixture-owner", true),
    ] {
        let (mut caller, _) = connect(address, token, false).await;
        send(&mut caller,json!({"op":"call","id":"progress","method":"agents.reportProgress","params":{"callerSessionId":"worker","note":"phase complete","needsDecision":true}})).await;
        let got = receive(&mut caller).await["result"].clone();
        assert_eq!(
            got.get("callerSessionId").is_some(),
            claimed,
            "{token}: {got}"
        );
        assert_eq!(got["note"], "phase complete");
        assert_eq!(got["needsDecision"], true);
        if token == "bus-fixture-view" {
            for params in [
                json!({"note":"unchanged"}),
                json!({"callerSessionId":{"bad":1},"note":"unchanged"}),
            ] {
                send(&mut caller, json!({"op":"call","id":"shape","method":"agents.reportProgress","params":params})).await;
                assert_eq!(
                    receive(&mut caller).await["result"],
                    json!({"note":"unchanged"})
                );
            }
        }
        caller.close(None).await.unwrap();
    }
    let (mut sibling, _) = connect(address, "plugin-fixture", false).await;
    hub.handle()
        .revoke_plugin("plugin-fixture".into())
        .await
        .unwrap();
    for socket in [&mut plugin, &mut sibling] {
        let next = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap();
        assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
    }
    assert!(
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token=plugin-fixture"))
            .await
            .is_err()
    );
    let (mut owner, _) = connect(address, "bus-fixture-owner", false).await;
    send(
        &mut owner,
        json!({"op":"register","methods":["acme.tools.echo"]}),
    )
    .await;
    assert_eq!(
        receive(&mut owner).await["methods"],
        json!(["acme.tools.echo"])
    );
    owner.close(None).await.unwrap();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn revocation_serializes_with_admission_and_suppresses_already_queued_streams() {
    use workspacer_hub::protocol::Frame;
    let mut options = Options::default();
    options.event_buffer = 1;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    for revoke_first in [false, true] {
        handle
            .register_plugin("race-token".into(), "race".into(), vec!["race.*".into()])
            .await
            .unwrap();
        // Exercise both serialized command orderings, rather than hoping a
        // concurrent network handshake lands in a microsecond-sized window.
        let connection = if revoke_first {
            let (revoked, admitted) = tokio::join!(biased;
                handle.revoke_plugin("race-token".into()),
                handle.connect_authenticated("race-token".into(), false)
            );
            revoked.unwrap();
            admitted
        } else {
            let (admitted, revoked) = tokio::join!(biased;
                handle.connect_authenticated("race-token".into(), false),
                handle.revoke_plugin("race-token".into())
            );
            revoked.unwrap();
            admitted
        };
        if revoke_first {
            assert!(connection.is_err());
        } else {
            let mut connection = connection.unwrap();
            assert!(
                connection
                    .send(Frame {
                        methods: vec!["race.stale".into()],
                        ..Frame::op("register")
                    })
                    .is_err()
            );
            assert!(connection.recv().await.is_none());
        }
        assert!(
            handle
                .connect_authenticated("race-token".into(), false)
                .await
                .is_err()
        );
    }
    handle
        .register_plugin("queued-token".into(), "queue".into(), vec![])
        .await
        .unwrap();
    let mut connection = handle
        .connect_authenticated("queued-token".into(), false)
        .await
        .unwrap();
    assert_eq!(connection.recv().await.unwrap().op, "hello");
    connection
        .send(Frame {
            topics: vec!["*".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    assert_eq!(connection.recv().await.unwrap().op, "subscribed");
    for _ in 0..8 {
        handle
            .publish(Event::new(
                "pty.bytes.secret-session",
                "fixture",
                json!("secret"),
            ))
            .unwrap();
    }
    handle.health().await.unwrap(); // all admissions/overflow metadata now queued
    handle.revoke_plugin("queued-token".into()).await.unwrap();
    assert!(
        connection.recv().await.is_none(),
        "queued bytes or synthesized desync escaped revocation"
    );
    assert!(
        connection
            .send(Frame {
                method: "fs.read".into(),
                ..Frame::op("call")
            })
            .is_err()
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn control_plane_provider_keeps_spawn_payloads_but_replaces_asserted_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut options = options(&root.path().join("tokens.json")).plugin_token(
        "spawn-plugin",
        "fixture.plugin",
        vec![],
    );
    options.control_plane_only = true;
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let (mut provider, _) = connect(address, "bus-fixture-owner", false).await;
    send(
        &mut provider,
        json!({"op":"register","methods":["agents.spawn","fixture.failure"]}),
    )
    .await;
    assert_eq!(
        receive(&mut provider).await["methods"],
        json!(["agents.spawn", "fixture.failure"])
    );
    for (token, peer, owner) in [
        ("bus-fixture-owner", false, true),
        ("bus-fixture-operator", false, false),
        ("spawn-plugin", false, false),
        ("bus-fixture-owner", true, false),
    ] {
        let (mut caller, _) = connect(address, token, peer).await;
        let params = json!({"dispatchOwnerSessionId":"manager","retrySourceSessionId":"worker","taskId":"task","stage":"fix","afterDispatchId":"first","skipPermissions":true,"yoloGranted":true,"profileGranted":true,"escalationScrubbed":["forged"],"profileId":"work","toolScope":"operator","pluginTools":["legacy.selection"]});
        send(
            &mut caller,
            json!({"op":"call","id":"spawn","method":"agents.spawn","params":params}),
        )
        .await;
        let forwarded = receive(&mut provider).await;
        assert_eq!(forwarded["op"], "call");
        assert_eq!(forwarded["method"], "agents.spawn");
        let got = &forwarded["params"];
        for field in [
            "taskId",
            "stage",
            "afterDispatchId",
            "skipPermissions",
            "profileId",
            "toolScope",
            "pluginTools",
        ] {
            assert_eq!(got[field], params[field], "{token} {field}");
        }
        for field in ["yoloGranted", "profileGranted", "escalationScrubbed"] {
            assert!(got.get(field).is_none(), "{token} {field}");
        }
        for field in ["dispatchOwnerSessionId", "retrySourceSessionId"] {
            assert_eq!(
                got.get(field).is_some(),
                owner,
                "{token} peer={peer}: {got}"
            );
        }
        send(
            &mut provider,
            json!({"op":"result","id":forwarded["id"],"result":{"sessionId":"retained-result"}}),
        )
        .await;
        assert_eq!(
            receive(&mut caller).await,
            json!({"op":"result","id":"spawn","result":{"sessionId":"retained-result"}})
        );
        caller.close(None).await.unwrap();
    }
    let (mut caller, _) = connect(address, "bus-fixture-operator", false).await;
    send(
        &mut caller,
        json!({"op":"call","id":"nonobject","method":"agents.spawn","params":[1,2,3]}),
    )
    .await;
    let forwarded = receive(&mut provider).await;
    assert_eq!(forwarded["params"], json!([1, 2, 3]));
    send(
        &mut provider,
        json!({"op":"result","id":forwarded["id"],"result":null}),
    )
    .await;
    assert_eq!(
        receive(&mut caller).await,
        json!({"op":"result","id":"nonobject","result":null})
    );
    send(
        &mut caller,
        json!({"op":"call","id":"failure","method":"fixture.failure"}),
    )
    .await;
    let forwarded = receive(&mut provider).await;
    send(
        &mut provider,
        json!({"op":"error","id":forwarded["id"],"error":"exact provider error"}),
    )
    .await;
    assert_eq!(
        receive(&mut caller).await,
        json!({"op":"error","id":"failure","error":"exact provider error"})
    );
    send(
        &mut caller,
        json!({"op":"call","id":"pending","method":"fixture.failure"}),
    )
    .await;
    assert_eq!(receive(&mut provider).await["op"], "call");
    provider.close(None).await.unwrap();
    let failed = receive(&mut caller).await;
    assert_eq!(failed["id"], "pending");
    assert!(failed["error"].as_str().unwrap().contains("disconnected"));
    send(
        &mut caller,
        json!({"op":"call","id":"absent","method":"nobody.home"}),
    )
    .await;
    let failed = receive(&mut caller).await;
    assert_eq!(failed["id"], "absent");
    assert!(failed["error"].as_str().unwrap().contains("no provider"));
    caller.close(None).await.unwrap();
    hub.shutdown().unwrap();
}
