use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct Inner {
    prepares: AtomicUsize,
    revokes: AtomicUsize,
    sweeps: AtomicUsize,
}
impl LaunchPreparation for Inner {
    fn sweep<'a>(&'a self, _: &'a std::collections::BTreeSet<String>) -> Operation<'a, ()> {
        Box::pin(async move {
            self.sweeps.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn prepare<'a>(&'a self, plan: &'a mut Plan, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.prepares.fetch_add(1, Ordering::SeqCst);
            plan.request["env"]["FACADE_TOKEN"] = "private-facade".into();
            plan.request["extra_args"]
                .as_array_mut()
                .unwrap()
                .push("--facade-first".into());
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.revokes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
struct FakeHost {
    valid: AtomicBool,
    consumed: AtomicBool,
    discovery: AtomicUsize,
    revoke_on_reply: bool,
    contexts: Mutex<Vec<Value>>,
    patch: Value,
    gate: Mutex<Option<Arc<HookGate>>>,
}
struct HookGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Host for FakeHost {
    fn check<'a>(&'a self, _: &'a LaunchPermit, finish: bool) -> Operation<'a, ()> {
        Box::pin(async move {
            if !self.valid.load(Ordering::SeqCst) || self.consumed.load(Ordering::SeqCst) {
                bail!("owner launch is no longer pending")
            };
            if finish {
                self.consumed.store(true, Ordering::SeqCst);
            }
            Ok(())
        })
    }
    fn manifest<'a>(&'a self, _: &'a str) -> Operation<'a, Manifest> {
        Box::pin(async move {
            self.discovery.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::from_value(
                json!({"id":"fixture.route","apiVersion":"1","server":{"command":"fixture"},"provides":["fixture.route.prepare"],"launchIntegration":{"version":1,"agents":["claude","codex"],"prepareMethod":"fixture.route.prepare"}}),
            )?)
        })
    }
    fn call<'a>(&'a self, method: &'a str, context: Value) -> Operation<'a, Value> {
        Box::pin(async move {
            assert_eq!(method, "fixture.route.prepare");
            self.contexts.lock().unwrap().push(context);
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if self.revoke_on_reply {
                self.valid.store(false, Ordering::SeqCst);
            }
            Ok(self.patch.clone())
        })
    }
}
struct FakeProbe(AtomicUsize);
impl codex::Probe for FakeProbe {
    fn read<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Provider> {
        Box::pin(async move {
            assert_eq!(plan.request["env"]["FACADE_TOKEN"], "private-facade");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Provider {
                id: "openai".into(),
                base_url: None,
            })
        })
    }
}
fn plan(provider: &str) -> Plan {
    Plan {
        provider: provider.into(),
        session_id: "session-one".into(),
        endpoint: "/sessions/spawn-managed",
        request: json!({"session_id":"session-one","cwd":"/project","bin":"fixture","model":"model-one","env":{"API_SECRET":"private-secret"},"extra_args":["--profile-first"]}),
        metadata: json!({"settings":{"launchIntegrationId":"fixture.route"},"launchIntegrationResume":true}),
        full_access: false,
        mcp_item_ids: vec![],
    }
}
fn permit() -> LaunchPermit {
    LaunchPermit {
        call_id: 7,
        connection_id: 3,
        session_id: "session-one".into(),
        plugin_id: "fixture.route".into(),
        nonce: "opaque-broker-nonce".into(),
    }
}
fn fixture(revoke: bool) -> (Arc<Preparation>, Arc<Inner>, Arc<FakeHost>, Arc<FakeProbe>) {
    let inner = Arc::new(Inner {
        prepares: AtomicUsize::new(0),
        revokes: AtomicUsize::new(0),
        sweeps: AtomicUsize::new(0),
    });
    let host = Arc::new(FakeHost {
        valid: AtomicBool::new(true),
        consumed: AtomicBool::new(false),
        discovery: AtomicUsize::new(0),
        revoke_on_reply: revoke,
        contexts: Mutex::new(vec![]),
        patch: json!({"env":{"ROUTE":"http://localhost"},"args":["--integration-last"]}),
        gate: Mutex::new(None),
    });
    let probe = Arc::new(FakeProbe(AtomicUsize::new(0)));
    let service = Arc::new(Preparation {
        inner: inner.clone(),
        host: host.clone(),
        probe: probe.clone(),
        pending: Mutex::new(BTreeMap::new()),
    });
    (service, inner, host, probe)
}
#[test]
fn patch_shape_matches_typescript_and_never_changes_cwd_or_executable() {
    for value in [
        Value::Null,
        json!([]),
        json!({"cwd":"/elsewhere"}),
        json!({"executable":"sh"}),
        json!({"env":null}),
        json!({"args":null}),
        json!({"env":{"BAD":1}}),
        json!({"env":{"BAD":"nul\0"}}),
        json!({"args":["nul\0"]}),
        json!({"args":vec!["a";65]}),
        json!({"env":{"__proto__":"x"}}),
    ] {
        assert!(validate_patch(value).is_err());
    }
    assert!(
        validate_patch(json!({"env":{"KEY":"😀".repeat(16384)},"args":["😀".repeat(4096)]}))
            .is_ok()
    );
    assert!(validate_patch(json!({"args":["😀".repeat(4097)]})).is_err());
}
#[tokio::test]
async fn enabled_hook_uses_bound_permit_after_facade_and_sends_only_context() {
    let (service, inner, host, probe) = fixture(false);
    let _lease = service.admit(permit()).unwrap();
    let mut plan = plan("codex");
    service.prepare(&mut plan, "generation").await.unwrap();
    assert_eq!(
        plan.request["extra_args"],
        json!(["--profile-first", "--facade-first", "--integration-last"])
    );
    assert_eq!(plan.request["env"]["API_SECRET"], "private-secret");
    assert_eq!(plan.request["env"]["ROUTE"], "http://localhost");
    assert_eq!(probe.0.load(Ordering::SeqCst), 1);
    let contexts = host.contexts.lock().unwrap();
    assert_eq!(
        contexts[0],
        json!({"version":1,"agent":"codex","cwd":"/project","model":"model-one","resume":true,"provider":{"id":"openai"}})
    );
    assert!(
        !serde_json::to_string(&*contexts)
            .unwrap()
            .contains("private")
    );
    drop(contexts);
    assert!(host.consumed.load(Ordering::SeqCst));
    assert!(service.pending.lock().unwrap().is_empty());
    service.revoke("session-one", "generation").await.unwrap();
    service.sweep(&Default::default()).await.unwrap();
    assert_eq!(inner.revokes.load(Ordering::SeqCst), 1);
    assert_eq!(inner.sweeps.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn revoked_owner_reply_cannot_apply_patch_and_claude_never_probes_codex() {
    let (service, inner, host, probe) = fixture(true);
    let _lease = service.admit(permit()).unwrap();
    let mut plan = plan("claude");
    assert!(
        service
            .prepare(&mut plan, "generation")
            .await
            .unwrap_err()
            .to_string()
            .contains("Restore the plugin/service")
    );
    assert!(plan.request["env"].get("ROUTE").is_none());
    assert_eq!(probe.0.load(Ordering::SeqCst), 0);
    assert_eq!(host.contexts.lock().unwrap().len(), 1);
    assert!(service.pending.lock().unwrap().is_empty());
    service.revoke("session-one", "generation").await.unwrap();
    assert_eq!(inner.revokes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn canceled_preparation_after_slow_hook_never_attempts_engine_and_revokes_credentials() {
    use crate::services::agent_lifecycle::{LaunchEngine, Lifecycle, Phase};
    struct Engine(AtomicUsize);
    impl LaunchEngine for Engine {
        fn sessions(&self) -> Operation<'_, Value> {
            Box::pin(async { Ok(json!([])) })
        }
        fn spawn<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Value> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"session_id":plan.session_id}))
            })
        }
        fn stop<'a>(&'a self, _: &'a str) -> Operation<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (service, inner, host, _) = fixture(false);
    let gate = Arc::new(HookGate {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    *host.gate.lock().unwrap() = Some(gate.clone());
    let _lease = service.admit(permit()).unwrap();
    let engine = Arc::new(Engine(AtomicUsize::new(0)));
    let lifecycle = Lifecycle::open(
        root.path().join("launches.json"),
        engine.clone(),
        service.clone(),
    )
    .unwrap();
    let mut planned = plan("claude");
    planned.request["cwd"] = json!(root.path());
    let running = lifecycle.clone();
    let pending = tokio::spawn(async move { running.launch(planned).await });
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(engine.0.load(Ordering::SeqCst), 0);
    // Models the broker's now-removed pending permit while the plugin is slow.
    host.valid.store(false, Ordering::SeqCst);
    gate.release.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("no longer pending")
    );
    assert_eq!(engine.0.load(Ordering::SeqCst), 0);
    assert_eq!(inner.revokes.load(Ordering::SeqCst), 1);
    let record = lifecycle.records()["session-one"].clone();
    assert_eq!(record.phase, Phase::Failed);
    assert!(!record.engine_attempted);
    assert!(record.receipt.is_none());
    assert!(!record.revocation_pending);
}

#[tokio::test]
async fn no_selection_does_no_discovery_and_json_selection_cannot_forge_permit() {
    let (service, inner, host, probe) = fixture(false);
    let mut requested = plan("claude");
    assert!(service.prepare(&mut requested, "generation").await.is_err());
    assert_eq!(inner.prepares.load(Ordering::SeqCst), 0);
    requested.metadata = json!({"settings":{}});
    service.prepare(&mut requested, "generation").await.unwrap();
    assert_eq!(host.discovery.load(Ordering::SeqCst), 0);
    assert_eq!(probe.0.load(Ordering::SeqCst), 0);
    let lease = service.admit(permit()).unwrap();
    drop(lease);
    assert!(service.pending.lock().unwrap().is_empty());
}
#[test]
fn codex_route_projection_rejects_credential_urls_and_presets() {
    let env = BTreeMap::from([("OPENAI_BASE_URL".into(), "http://localhost:9000/v1/".into())]);
    assert_eq!(
        provider_from_config(
            &json!({"api_key":"secret"}),
            &env,
            Some("https://inherited.invalid")
        )
        .unwrap(),
        Provider {
            id: "openai".into(),
            base_url: Some("http://localhost:9000/v1".into())
        }
    );
    for config in [
        json!({"profile":"named"}),
        json!({"model_provider":"proxy"}),
        json!({"openai_base_url":"https://user:secret@example.com"}),
        json!({"openai_base_url":"https://example.com?key=secret"}),
        json!({"openai_base_url":"file:///tmp/key"}),
    ] {
        assert!(provider_from_config(&config, &BTreeMap::new(), None).is_err());
    }
    assert!(routing_config_args(&["-p".into(), "named".into()]).is_err());
    assert!(routing_config_args(&["-c".into()]).is_err());
    assert_eq!(
        routing_config_args(&[
            "--model".into(),
            "m".into(),
            "-c".into(),
            "model_provider=\"proxy\"".into(),
            "--config=x=1".into()
        ])
        .unwrap(),
        vec!["-c", "model_provider=\"proxy\"", "--config=x=1"]
    );
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_codex_probe_uses_bounded_stdio_and_reaps_child() {
    use codex::Probe;
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    std::fs::write(&binary,"#!/bin/sh\necho $$ > probe.pid\nprintf '%s' \"$HUB_TOKEN\" > host-token.txt\nread initialize\nprintf '%s\\n' '{\"id\":1,\"result\":{}}'\nread initialized\nread config\nprintf '%s\\n' '{\"id\":2,\"result\":{\"config\":{\"model_provider\":\"proxy\",\"model_providers\":{\"proxy\":{\"base_url\":\"http://localhost:9000/v1\",\"api_key\":\"private\"}}}}}'\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut plan = plan("codex");
    plan.request["cwd"] = json!(root.path());
    plan.request["bin"] = json!(binary);
    plan.request["extra_args"] = json!([]);
    plan.request["env"]["HUB_TOKEN"] = "must-not-reach-probe".into();
    let provider = codex::NativeProbe.read(&plan).await.unwrap();
    assert_eq!(provider.id, "proxy");
    assert!(
        std::fs::read_to_string(root.path().join("host-token.txt"))
            .unwrap()
            .is_empty(),
        "probe inherited a host credential"
    );
    assert_eq!(
        provider.base_url.as_deref(),
        Some("http://localhost:9000/v1")
    );
    let pid = std::fs::read_to_string(root.path().join("probe.pid")).unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn plugin_proxy_configuration_cannot_override_resolved_identity_or_routing() {
    for value in [
        json!({"args":["--model","bypass"]}),
        json!({"args":["--resume","another"]}),
        json!({"args":["-c","model_reasoning_effort=\"high\""]}),
        json!({"args":["--config","mcp_servers={}"]}),
        json!({"env":{"CODEX_HOME":"/another-account"}}),
        json!({"env":{"PATH":"/another-binary"}}),
    ] {
        assert!(validate_overlay(&validate_patch(value).unwrap(), "codex").is_err());
    }
    assert!(validate_overlay(&validate_patch(json!({"env":{"OPENAI_BASE_URL":"http://localhost:8787/v1"},"args":["--config","model_providers.openai.base_url=\"http://localhost:8787/v1\"","--config","model_providers.openai.env_http_headers.X-Headroom-Base-Url=\"HEADROOM_CODEX_UPSTREAM_BASE_URL\""]})).unwrap(),"codex").is_ok());
}
