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
    options.provider_relay = Some(Config {
        url: format!("ws://{address}/bus"),
        token: provider.token,
        scope: Scope::Catalog,
        node_id: "worker-1".into(),
        last_exit_file: None,
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
