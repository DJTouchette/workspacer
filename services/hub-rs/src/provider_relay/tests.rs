use super::*;
#[test]
fn declarations_do_not_export_arbitrary_installed_handlers() {
    let available = BTreeSet::from([
        "config.get".into(),
        "jobs.upsert".into(),
        "plugin.private".into(),
        "agents.spawn".into(),
        "files.upload".into(),
    ]);
    let catalog = methods::offered(Scope::Catalog, &available);
    assert!(catalog.contains_key("config.get"));
    assert!(!catalog.contains_key("jobs.upsert"));
    assert!(!catalog.contains_key("agents.spawn"));
    let full = methods::offered(Scope::Full, &available);
    assert_eq!(full["files.receiveUpload"], "files.upload");
    assert!(!full.contains_key("plugin.private"));
}

#[tokio::test]
async fn catalog_relay_preserves_remote_identity_and_named_liveness() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let provider = crate::auth::mint(&tokens, crate::auth::Scope::Provider, "worker").unwrap();
    let viewer = crate::auth::mint(&tokens, crate::auth::Scope::View, "viewer").unwrap();
    let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "operator").unwrap();
    let mut options = Options::default();
    options.token = "central-owner".into();
    options.scoped_tokens = Some(tokens);
    options.control_plane_only = true;
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    let central = crate::Hub::start(options).unwrap();
    central.ready().await.unwrap();
    let address = match central.handle().status().borrow().clone() {
        crate::Status::Ready {
            address: Some(address),
            ..
        } => address,
        _ => panic!("listener absent"),
    };
    let mut options = Options::default();
    options.token = "different-local-owner".into();
    let last_exit_file = root.path().join("last-exit.json");
    std::fs::write(&last_exit_file,r#"{"bootId":"fixture-previous","reason":"claudemon-died","exitCode":1,"at":"2026-08-24T21:00:00Z","machine":"fixture-node"}"#).unwrap();
    options.provider_relay = Some(Config {
        url: format!("ws://{address}/bus"),
        token: provider.token,
        scope: Scope::Catalog,
        node_id: "worker-1".into(),
        last_exit_file: Some(last_exit_file.clone()),
        caller_token: None,
    });
    let writes = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (write_count, write_entered, write_release) =
        (writes.clone(), entered.clone(), release.clone());
    options = options.handler("config.get", |caller, _| async move {
        Ok(json!({"scope":caller.scope,"host":caller.authenticated_host,"tokenId":caller.token_id,"connectionId":caller.connection_id}))
    }).handler("config.save", move |caller, value| {
        let (writes, entered, release) = (write_count.clone(), write_entered.clone(), write_release.clone());
        async move {
            writes.fetch_add(1, Ordering::SeqCst);
            if value["hold"] == true { entered.notify_one(); release.notified().await; }
            Ok(json!({"scope":caller.scope,"host":caller.authenticated_host,"value":value}))
        }
    });
    let worker = crate::Hub::start(options).unwrap();
    worker.ready().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while central
            .handle()
            .provider_connection("brain.info")
            .await
            .unwrap()
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let view = Client::connect_remote(&format!("ws://{address}/bus"), &viewer.token)
        .await
        .unwrap();
    let operator_client = Client::connect_remote(&format!("ws://{address}/bus"), &operator.token)
        .await
        .unwrap();
    let read = view.call("config.get", json!({})).await.unwrap();
    assert_eq!(read["scope"], "view");
    assert_eq!(read["host"], false);
    assert!(
        view.call("config.save", json!({"forged":true}))
            .await
            .is_err()
    );
    let saved = operator_client
        .call("config.save", json!({"setting":"value"}))
        .await
        .unwrap();
    assert_eq!(saved["scope"], "operator");
    assert_eq!(saved["host"], false);
    let host = Client::from_connection(
        central
            .handle()
            .connect_authenticated("central-owner".into(), false)
            .await
            .unwrap(),
    );
    let info = host.call("brain.info", json!({})).await.unwrap();
    assert_eq!(info["node"], "worker-1");
    assert_eq!(info["runtime"], "rust");
    assert_eq!(
        info["lastExit"],
        json!({"reason":"claudemon-died","exitCode":1,"at":"2026-08-24T21:00:00Z"})
    );
    std::fs::write(&last_exit_file, r#"{"reason":"signal-TERM","exitCode":0}"#).unwrap();
    assert_eq!(
        host.call("brain.info", json!({})).await.unwrap()["lastExit"],
        info["lastExit"],
        "previous-run evidence is cached for this relay lifetime"
    );
    assert!(
        worker
            .handle()
            .provider_connection("brain.info")
            .await
            .unwrap()
            .is_none()
    );
    // Original caller closure tears down only that delegated local connection,
    // even after its last RPC completed (terminal leases use the same owner).
    let local_view_id = read["connectionId"].as_u64().unwrap();
    view.close();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !worker
                .handle()
                .quiescence_clients()
                .await
                .unwrap()
                .iter()
                .any(|client| client.connection_id == local_view_id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // A provider transport loss after acceptance has an unknown acknowledgement;
    // reconnect must not resubmit the mutation or bind its reply to a new link.
    let old = central
        .handle()
        .provider_connection("brain.info")
        .await
        .unwrap()
        .unwrap();
    let pending_client = operator_client.clone();
    let active_local_id =
        operator_client.call("config.get", json!({})).await.unwrap()["connectionId"]
            .as_u64()
            .unwrap();
    let pending = tokio::spawn(async move {
        pending_client
            .call("config.save", json!({"hold":true}))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    assert!(
        central
            .handle()
            .evict_provider("brain.info", old)
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while worker
            .handle()
            .quiescence_clients()
            .await
            .unwrap()
            .iter()
            .any(|client| client.connection_id == active_local_id)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.notify_one();
    assert!(pending.await.unwrap().is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if central
                .handle()
                .provider_connection("brain.info")
                .await
                .unwrap()
                .is_some_and(|id| id != old)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        writes.load(Ordering::SeqCst),
        2,
        "reconnect must not replay an accepted mutation"
    );
    assert_eq!(
        operator_client
            .call("config.save", json!({"afterReconnect":true}))
            .await
            .unwrap()["host"],
        false
    );
    assert_eq!(writes.load(Ordering::SeqCst), 3);
    view.close();
    operator_client.close();
    host.close();
    worker.shutdown().unwrap();
    central.shutdown().unwrap();
}

#[tokio::test]
async fn individual_deadline_keeps_sibling_and_caller_lease_and_never_replays_accepted_effect() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let provider = crate::auth::mint(&tokens, crate::auth::Scope::Provider, "worker").unwrap();
    let mut central_options = Options::default();
    central_options.token = "central-owner".into();
    central_options.scoped_tokens = Some(tokens);
    central_options.control_plane_only = true;
    central_options.listen = Some("127.0.0.1:0".parse().unwrap());
    let central = crate::Hub::start(central_options).unwrap();
    let address = central.ready().await.unwrap().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(tokio::sync::Notify::new());
    let second = Arc::new(tokio::sync::Notify::new());
    let (tx, mut entered) = tokio::sync::mpsc::unbounded_channel();
    let (a, f, one, two) = (
        accepted.clone(),
        finished.clone(),
        first.clone(),
        second.clone(),
    );
    let mut options = Options::default()
        .handler("config.get", |caller, _| async move {
            Ok(json!({"connectionId":caller.connection_id}))
        })
        .handler("config.save", move |caller, value| {
            let (a, f, one, two, tx) = (a.clone(), f.clone(), one.clone(), two.clone(), tx.clone());
            async move {
                a.fetch_add(1, Ordering::SeqCst);
                let key = value["key"].as_str().unwrap().to_string();
                tx.send((key.clone(), caller.connection_id)).unwrap();
                if key == "first" {
                    one.notified().await;
                } else {
                    two.notified().await;
                }
                f.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"key":key}))
            }
        });
    options.provider_relay = Some(Config {
        url: format!("ws://{address}/bus"),
        token: provider.token,
        scope: Scope::Catalog,
        node_id: "cancel-worker".into(),
        last_exit_file: None,
        caller_token: None,
    });
    let worker = crate::Hub::start(options).unwrap();
    worker.ready().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while central
            .handle()
            .provider_connection("config.get")
            .await
            .unwrap()
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let caller = Client::connect(&central.handle()).await.unwrap();
    let local_id = caller.call("config.get", Value::Null).await.unwrap()["connectionId"]
        .as_u64()
        .unwrap();
    let one = caller.clone();
    let one = tokio::spawn(async move {
        one.call_with_timeout(
            "config.save",
            json!({"key":"first"}),
            Duration::from_secs(1),
        )
        .await
    });
    let first_entered = tokio::time::timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_entered, ("first".into(), local_id));
    let two = caller.clone();
    let two = tokio::spawn(async move { two.call("config.save", json!({"key":"second"})).await });
    let second_entered = tokio::time::timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second_entered, ("second".into(), local_id));
    assert!(
        one.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("outcome is unknown")
    );
    // Allow both brokers' cancellation ticks; neither may close the cached identity.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        caller.call("config.get", Value::Null).await.unwrap()["connectionId"],
        local_id
    );
    second.notify_one();
    assert_eq!(two.await.unwrap().unwrap()["key"], "second");
    first.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while finished.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
    assert_eq!(
        caller.call("config.get", Value::Null).await.unwrap()["connectionId"],
        local_id
    );
    caller.close();
    worker.shutdown().unwrap();
    central.shutdown().unwrap();
}

#[tokio::test]
async fn full_caller_requires_separate_facade_authority_and_reads_central_layout() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let ordinary = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "ordinary").unwrap();
    let facade = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "facade").unwrap();
    crate::auth::update_records(&tokens, |rows| {
        rows.iter_mut()
            .find(|row| row.token == facade.token)
            .unwrap()
            .facade_authority = true;
        Ok(())
    })
    .unwrap();
    let mut options = Options::default();
    options.token = "actual-central-owner".into();
    options.scoped_tokens = Some(tokens);
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options = options.handler("layout.get", |_, _| async {
        Ok(json!({"source":"central"}))
    });
    let central = crate::Hub::start(options).unwrap();
    central.ready().await.unwrap();
    let address = match central.handle().status().borrow().clone() {
        crate::Status::Ready {
            address: Some(address),
            ..
        } => address,
        _ => panic!("listener absent"),
    };
    let url = format!("ws://{address}/bus");
    for token in ["actual-central-owner".to_owned(), ordinary.token] {
        let caller = UpstreamCaller::new(url.clone(), token);
        let task = tokio::spawn(caller.clone().run());
        let error = caller
            .client()
            .await
            .err()
            .expect("owner/ordinary operator rejected");
        assert!(error.to_string().contains("scoped operator facade"));
        caller.close();
        task.await.unwrap().unwrap();
    }
    let layout = Arc::new(std::sync::RwLock::new(Value::Null));
    let caller = UpstreamCaller::with_layout(url, facade.token, layout.clone());
    let task = tokio::spawn(caller.clone().run());
    let client = caller.client().await.unwrap();
    assert_eq!(layout.read().unwrap()["source"], "central");
    assert_eq!(
        client.call("layout.get", json!({})).await.unwrap()["source"],
        "central"
    );
    caller.close();
    task.await.unwrap().unwrap();
    assert_eq!(
        layout.read().unwrap()["source"],
        "central",
        "last good visibility survives disconnect"
    );
    central.shutdown().unwrap();
}

#[tokio::test]
async fn withheld_registration_recovers_after_other_owner_eviction_without_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let provider =
        crate::auth::mint(&tokens, crate::auth::Scope::Provider, "recovering-worker").unwrap();
    let mut options = Options::default();
    options.token = "central-fixture".into();
    options.scoped_tokens = Some(tokens);
    options.control_plane_only = true;
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    let central = crate::Hub::start(options).unwrap();
    let address = central.ready().await.unwrap().unwrap();
    let url = format!("ws://{address}/bus");
    let mut request = url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Authorization", "Bearer central-fixture".parse().unwrap());
    let (mut predecessor, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(receive(&mut predecessor).await.unwrap().op, "hello");
    send(
        &mut predecessor,
        &Frame {
            methods: vec!["brain.info".into()],
            wants_caller_context: true,
            ..Frame::op("register")
        },
    )
    .await
    .unwrap();
    let first_ack = receive(&mut predecessor).await.unwrap();
    assert_eq!(first_ack.op, "registered");
    assert_eq!(first_ack.methods, ["brain.info"]);
    let predecessor_id = central
        .handle()
        .provider_connection("brain.info")
        .await
        .unwrap()
        .unwrap();

    let mut options = Options::default();
    options.token = "worker-fixture".into();
    options.provider_relay = Some(Config {
        url,
        token: provider.token,
        scope: Scope::Catalog,
        node_id: "recovering-worker".into(),
        last_exit_file: None,
        caller_token: None,
    });
    options = options
        .handler("config.get", |_, _| async { Ok(json!({"worker":true})) })
        .handler("fixture.private", |_, _| async { Ok(json!("local only")) });
    let worker = crate::Hub::start(options).unwrap();
    worker.ready().await.unwrap();
    let admin = Client::connect(&worker.handle()).await.unwrap();
    let partial = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = admin
                .call("providerRelay.status", Value::Null)
                .await
                .unwrap();
            if status["connected"] == true {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(partial["registeredMethods"], json!(["config.get"]));
    assert_eq!(
        central
            .handle()
            .provider_connection("brain.info")
            .await
            .unwrap(),
        Some(predecessor_id)
    );
    let relay_id = central
        .handle()
        .provider_connection("config.get")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(relay_id, predecessor_id);
    let caller = Client::connect(&central.handle()).await.unwrap();
    assert_eq!(
        caller.call("config.get", Value::Null).await.unwrap(),
        json!({"worker":true})
    );
    assert!(
        central
            .handle()
            .provider_connection("fixture.private")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        central
            .handle()
            .provider_connection("config.save")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        central
            .handle()
            .evict_provider("brain.info", predecessor_id)
            .await
            .unwrap()
    );

    let recovered = tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            assert_eq!(
                central
                    .handle()
                    .provider_connection("config.get")
                    .await
                    .unwrap(),
                Some(relay_id),
                "relay reconnected instead of repairing registration"
            );
            if central
                .handle()
                .provider_connection("brain.info")
                .await
                .unwrap()
                == Some(relay_id)
            {
                let status = admin
                    .call("providerRelay.status", Value::Null)
                    .await
                    .unwrap();
                if status["registeredMethods"] == json!(["brain.info", "config.get"]) {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    let after = admin
        .call("providerRelay.status", Value::Null)
        .await
        .unwrap();
    let info = if recovered {
        assert_eq!(
            caller.call("config.get", Value::Null).await.unwrap(),
            json!({"worker":true})
        );
        Some(caller.call("brain.info", Value::Null).await.unwrap())
    } else {
        None
    };
    caller.close();
    admin.close();
    let _ = tokio::time::timeout(Duration::from_millis(100), predecessor.close(None)).await;
    tokio::task::spawn_blocking(move || worker.shutdown())
        .await
        .unwrap()
        .unwrap();
    tokio::task::spawn_blocking(move || central.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(
        recovered,
        "stale predecessor was evicted but relay never repaired its partial registration: {after}"
    );
    assert_eq!(
        after["registeredMethods"],
        json!(["brain.info", "config.get"])
    );
    assert_eq!(info.unwrap()["scope"], "catalog");
}

#[test]
fn declared_scope_partition_and_adopted_overlap_match_portable_contracts() {
    let reference: Value =
        serde_json::from_str(include_str!("../../assets/brain-capabilities.json")).unwrap();
    let names = |value: &Value| -> BTreeSet<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    let expected_full = names(&reference["full"]);
    let expected_catalog = names(&reference["catalog"]);
    assert!(expected_full.len() >= 100 && expected_catalog.len() >= 20);
    let mut available = expected_full.clone();
    available.extend(names(&reference["hub"]));
    available.insert("fixture.private".into());
    let full: BTreeSet<String> = methods::offered(Scope::Full, &available)
        .into_keys()
        .collect();
    let catalog: BTreeSet<String> = methods::offered(Scope::Catalog, &available)
        .into_keys()
        .collect();
    assert_eq!(full, expected_full);
    assert_eq!(catalog, expected_catalog);
    assert!(catalog.is_subset(&full) && catalog.len() < full.len());
    for method in [
        "agents.spawn",
        "sessions.transcript",
        "claude.approve",
        "notifications.post",
        "search.project",
    ] {
        assert!(!catalog.contains(method));
    }
    let desktop = include_str!("../../../../apps/desktop/src/main/services/hubCapabilities.ts");
    let declarations = |name: &str| -> BTreeSet<String> {
        regex::Regex::new(&format!(r"(?m)^\s*{name}\(\s*'([A-Za-z][A-Za-z0-9_.]*)'"))
            .unwrap()
            .captures_iter(desktop)
            .map(|c| c[1].to_owned())
            .collect()
    };
    let delegated = declarations("cat");
    let main = declarations("registerCapability");
    assert!(
        delegated.len() >= 20 && main.len() >= 50,
        "desktop declarations were not read"
    );
    assert!(delegated.is_subset(&catalog));
    assert!(main.is_disjoint(&catalog));
    assert!(main.contains("fs.readImage") && !delegated.contains("fs.readImage"));
    assert!(main.contains("agents.summarizeStatus") && !full.contains("agents.summarizeStatus"));
    for methods in [&full, &catalog] {
        assert!(methods.contains("brain.info"));
    }
    assert!(!main.contains("brain.info") && !delegated.contains("brain.info"));
    let overlaps: Value =
        serde_json::from_str(include_str!("../../assets/provider-scope-overlaps.json")).unwrap();
    let rows = overlaps["overlaps"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        55,
        "review the declared overlap contract, do not silently shrink it"
    );
    let allowed: BTreeSet<String> = rows
        .iter()
        .map(|r| r["method"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        full.intersection(&main).cloned().collect::<BTreeSet<_>>(),
        allowed
    );
    for row in rows {
        assert!(!row["rationale"].as_str().unwrap().trim().is_empty());
    }
    for row in rows.iter().filter(|r| r["requiresAdoptedNote"] == true) {
        assert!(desktop.contains(&format!(
            "ADOPTED-DEGRADED: {}",
            row["method"].as_str().unwrap()
        )));
    }
    // The public typed boundary deliberately rejects unknown scopes rather than
    // silently broadening a misspelled selection to full registration.
    assert!(serde_json::from_str::<Scope>("\"bogus\"").is_err());
}

#[tokio::test]
async fn desktop_hub_only_composition_leaves_notifications_with_desktop_provider() {
    let root = tempfile::tempdir().unwrap();
    let argv: Vec<std::ffi::OsString> = vec![
        "workspacer-rust".into(),
        "--config-dir".into(),
        root.path().join("config").into_os_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--hub-port".into(),
        "0".into(),
        "serve".into(),
        "--hub-only".into(),
        "--no-mcp".into(),
        "--home-dir".into(),
        root.path().into(),
    ];
    let args = crate::cli::CommandLine::try_parse_compatible_from(argv).unwrap();
    let crate::cli::Command::Serve(serve) = &args.command else {
        panic!("not a serve plan");
    };
    let plan = crate::cli::plan_serve(&args, serve).unwrap();
    assert!(plan.hub_only);
    let mut options = Options::default();
    options.control_plane_only = plan.hub_only;
    options.config_dir = Some(plan.config);
    options.home_dir = Some(plan.home);
    options.data_dir = Some(root.path().join("hub-state"));
    let hub = crate::Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let methods = hub.handle().health().await.unwrap()["methodNames"].clone();
    assert!(
        !methods
            .as_array()
            .unwrap()
            .contains(&json!("notifications.post")),
        "hub-only must not claim log-only delivery"
    );
    let mut desktop = hub.handle().connect().await.unwrap();
    desktop.recv().await.unwrap();
    desktop
        .send(Frame {
            methods: vec!["notifications.post".into()],
            ..Frame::op("register")
        })
        .unwrap();
    let accepted = desktop.recv().await.unwrap();
    assert_eq!(accepted.methods, ["notifications.post"]);
    let delivery = tokio::spawn(async move {
        let call = desktop.recv().await.unwrap();
        assert_eq!(call.method, "notifications.post");
        assert_eq!(
            call.params,
            Some(json!({"title":"fixture","body":"desktop owned"}))
        );
        desktop
            .send(Frame {
                id: call.id,
                result: Some(json!({"delivery":"desktop-fixture"})),
                ..Frame::op("result")
            })
            .unwrap();
        // Keep the provider alive until the response has crossed the broker.
        desktop
    });
    let caller = Client::connect(&hub.handle()).await.unwrap();
    let result = caller
        .call(
            "notifications.post",
            json!({"title":"fixture","body":"desktop owned"}),
        )
        .await
        .unwrap();
    assert_eq!(result, json!({"delivery":"desktop-fixture"}));
    let desktop = delivery.await.unwrap();
    caller.close();
    drop(desktop);
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn partial_duplicate_ack_filters_ungranted_methods_and_shutdown_stops_retry() {
    async fn frame(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> Frame {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return Frame::decode(text.as_bytes()).unwrap(),
                Message::Ping(_) => socket.flush().await.unwrap(),
                other => panic!("unexpected frame {other:?}"),
            }
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (confirmed, barrier) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(json!({"op":"hello"}).to_string()))
            .await
            .unwrap();
        let first = frame(&mut socket).await;
        assert_eq!(first.op, "register");
        assert_eq!(first.methods, ["brain.info", "config.get"]);
        assert!(first.wants_caller_context);
        let partial = json!({"op":"registered","methods":["config.get"],"callerContextVersion":1})
            .to_string();
        socket.send(Message::Text(partial.clone())).await.unwrap();
        let retry = tokio::time::timeout(Duration::from_secs(7), frame(&mut socket))
            .await
            .unwrap();
        assert_eq!(retry.op, "register");
        assert_eq!(retry.methods, first.methods);
        assert!(
            retry.wants_caller_context,
            "retry must preserve identity negotiation"
        );
        let duplicate = json!({"op":"registered","methods":["config.get","config.get","fixture.private"],"callerContextVersion":1}).to_string();
        socket.send(Message::Text(duplicate.clone())).await.unwrap();
        socket.send(Message::Text(duplicate)).await.unwrap();
        for (id, method) in [("ungranted", "brain.info"), ("phantom", "fixture.private")] {
            socket
                .send(Message::Text(
                    json!({"op":"call","id":id,"method":method,"params":{}}).to_string(),
                ))
                .await
                .unwrap();
            let refusal = frame(&mut socket).await;
            assert_eq!(refusal.op, "error");
            assert_eq!(refusal.id, id);
            assert!(
                refusal.error.contains("not registered"),
                "{}",
                refusal.error
            );
        }
        confirmed.send(()).unwrap();
        // The connection-scoped interval cannot keep an independently owned
        // retry task alive after cancellation; require actual socket retirement.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) => {
                        let request = Frame::decode(text.as_bytes()).unwrap();
                        assert_eq!(request.op, "register");
                        socket.send(Message::Text(partial.clone())).await.unwrap();
                    }
                    Some(Ok(Message::Ping(_))) => socket.flush().await.unwrap(),
                    other => panic!("unexpected shutdown frame {other:?}"),
                }
            }
        })
        .await
        .unwrap();
    });
    let mut options = Options::default();
    options.provider_relay = Some(Config {
        url: format!("ws://{address}/bus"),
        token: "fixture".into(),
        scope: Scope::Catalog,
        node_id: String::new(),
        last_exit_file: None,
        caller_token: None,
    });
    options = options
        .handler("config.get", |_, _| async { Ok(json!({})) })
        .handler("fixture.private", |_, _| async {
            Ok(json!("not exported"))
        });
    let worker = crate::Hub::start(options).unwrap();
    worker.ready().await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), barrier)
        .await
        .unwrap()
        .unwrap();
    let admin = Client::connect(&worker.handle()).await.unwrap();
    let status = admin
        .call("providerRelay.status", Value::Null)
        .await
        .unwrap();
    assert_eq!(status["registeredMethods"], json!(["config.get"]));
    admin.close();
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || worker.shutdown()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    server.await.unwrap();
}
