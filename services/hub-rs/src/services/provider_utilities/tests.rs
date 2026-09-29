use super::*;
#[test]
fn title_contract_preserves_provider_defaults_and_fallback() {
    assert_eq!(
        text::sanitize("\nTitle: > Fix the flaky test.\nmore"),
        Some("Fix the flaky test".into())
    );
    assert_eq!(
        text::sanitize("“Repair the cache invalidation.”"),
        Some("Repair the cache invalidation".into())
    );
    assert_eq!(text::sanitize("Sorry, I cannot help"), None);
    assert_eq!(
        text::sanitize("Investigate the intermittent failure in the"),
        Some("Investigate the intermittent failure".into())
    );
    assert_eq!(
        text::fallback("# Repair login\nfollow-up"),
        Some("Repair login".into())
    );
    let config = json!({"agents":{"autoTitle":{"model":"haiku","models":{"codex":"gpt-5.4-mini","claude":"gpt-5"}}}});
    assert_eq!(
        text::title_model(&config, "codex"),
        Some("gpt-5.4-mini".into())
    );
    assert_eq!(text::title_model(&config, "claude"), Some("haiku".into()));
    assert_eq!(text::title_model(&config, "opencode"), None);
    assert!(text::prompt(&"😀".repeat(2000), "").encode_utf16().count() < 1500);
}
#[test]
fn completion_output_never_uses_json_bookkeeping_as_title() {
    assert_eq!(text::extract("codex", "{\"type\":\"turn.completed\"}"), "");
    assert_eq!(
        text::extract(
            "codex",
            "{\"item\":{\"type\":\"agent_message\",\"text\":\"old\"}}\n{\"item\":{\"type\":\"agent_message\",\"text\":\"new\"}}"
        ),
        "new"
    );
    assert_eq!(
        text::extract(
            "opencode",
            "{\"type\":\"text\",\"part\":{\"text\":\"Repair \"}}\n{\"type\":\"text\",\"part\":{\"text\":\"cache\"}}"
        ),
        "Repair cache"
    );
    assert_eq!(
        text::extract("codex", "plain older CLI answer"),
        "plain older CLI answer"
    );
    assert_eq!(
        text::classify("network authentication failed"),
        text::Failure::Authentication
    );
    assert_eq!(text::classify("429 out of credits"), text::Failure::Limited);
    assert_eq!(
        text::classify("model not available"),
        text::Failure::Unsupported
    );
}
#[test]
fn tools_status_is_discovery_not_inference() {
    let rows = tools::status();
    assert_eq!(rows.as_array().unwrap().len(), 7);
    for row in rows.as_array().unwrap() {
        assert!(row["available"].is_boolean());
        assert!(row.get("responding").is_none());
        assert!(row["features"].is_array());
    }
}
struct FakePing {
    count: std::sync::atomic::AtomicUsize,
    active: Arc<std::sync::atomic::AtomicUsize>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Ping for FakePing {
    fn ping<'a>(&'a self, _: Context, _: PathBuf) -> Operation<'a, completion::Outcome<()>> {
        Box::pin(async move {
            use std::sync::atomic::Ordering;
            struct Active(Arc<std::sync::atomic::AtomicUsize>);
            impl Drop for Active {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            self.count.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            self.entered.notify_one();
            self.release.notified().await;
            Ok(Ok(()))
        })
    }
}
fn fixture() -> (tempfile::TempDir, Arc<Service>, Arc<FakePing>) {
    let root = tempfile::tempdir().unwrap();
    let config = Arc::new(Config::open(root.path().join("config.yaml")));
    config.save(json!({"agents":{"checkProviderOnStartup":false,"managerProvider":"codex","binaries":{"codex":std::env::current_exe().unwrap()}}}),true).unwrap();
    let ping = Arc::new(FakePing {
        count: 0.into(),
        active: Arc::new(0.into()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut service = Service::new(config, root.path().into(), None);
    let value = Arc::get_mut(&mut service).unwrap();
    value.owned = true;
    value.ping = ping.clone();
    value.startup_delay = Duration::from_millis(15);
    (root, service, ping)
}
#[tokio::test]
async fn readiness_reads_never_spend_and_manual_checks_coalesce_when_startup_disabled() {
    use std::sync::atomic::Ordering;
    let (_root, service, ping) = fixture();
    let task = tokio::spawn(service.clone().run());
    tokio::time::sleep(Duration::from_millis(40)).await;
    for _ in 0..5 {
        assert_eq!(service.read("codex")["state"], "unchecked");
    }
    assert_eq!(ping.count.load(Ordering::SeqCst), 0);
    let first = {
        let service = service.clone();
        tokio::spawn(async move { service.check("codex").await })
    };
    tokio::time::timeout(Duration::from_secs(1), ping.entered.notified())
        .await
        .unwrap();
    let second = {
        let service = service.clone();
        tokio::spawn(async move { service.check("codex").await })
    };
    assert_eq!(service.read("codex")["state"], "checking");
    ping.release.notify_one();
    let result = first.await.unwrap();
    assert_eq!(result["state"], "responding");
    assert!(result["checkedAt"].as_i64().unwrap() > 0);
    assert_eq!(second.await.unwrap(), result);
    assert_eq!(service.check("codex").await, result);
    assert_eq!(ping.count.load(Ordering::SeqCst), 1);
    service.close();
    task.await.unwrap().unwrap();
    assert_eq!(ping.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn configuration_change_and_shutdown_cancel_owned_readiness_and_hide_old_facts() {
    use std::sync::atomic::Ordering;
    let (_root, service, ping) = fixture();
    let task = tokio::spawn(service.clone().run());
    let request = {
        let service = service.clone();
        tokio::spawn(async move { service.check("codex").await })
    };
    tokio::time::timeout(Duration::from_secs(1), ping.entered.notified())
        .await
        .unwrap();
    service
        .config
        .save(json!({"agents":{"autoTitle":{"enabled":false}}}), true)
        .unwrap();
    assert_eq!(service.read("codex")["state"], "unchecked");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .unwrap()
            .unwrap()["state"],
        "unchecked"
    );
    assert_eq!(ping.active.load(Ordering::SeqCst), 0);
    let request = {
        let service = service.clone();
        tokio::spawn(async move { service.check("codex").await })
    };
    tokio::time::timeout(Duration::from_secs(1), ping.entered.notified())
        .await
        .unwrap();
    service.close();
    task.await.unwrap().unwrap();
    assert_eq!(request.await.unwrap()["state"], "unchecked");
    assert_eq!(ping.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn no_owned_engine_never_borrows_account_readiness_and_titles_degrade() {
    let root = tempfile::tempdir().unwrap();
    let config = Arc::new(Config::open(root.path().join("config.yaml")));
    let service = Service::new(config, root.path().into(), None);
    assert_eq!(service.check("codex").await, json!({"state":"unsupported"}));
    assert_eq!(
        service
            .title(&json!({"userMessage":"# Repair login\nbody","provider":"unknown"}))
            .await
            .unwrap(),
        "Repair login"
    );
    assert_eq!(
        service.title(&json!({"userMessage":" "})).await.unwrap(),
        Value::Null
    );
}
#[tokio::test]
async fn startup_check_is_inert_until_first_readiness_request_and_runs_only_once() {
    use std::sync::atomic::Ordering;
    let (_root, service, ping) = fixture();
    service
        .config
        .save(json!({"agents":{"checkProviderOnStartup":true}}), true)
        .unwrap();
    let task = tokio::spawn(service.clone().run());
    assert_eq!(
        service
            .title(&json!({"provider":"unknown","userMessage":"Repair startup"}))
            .await
            .unwrap(),
        "Repair startup"
    );
    let _ = tools::status();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        ping.count.load(Ordering::SeqCst),
        0,
        "constructing services/title/tools must not invoke a provider"
    );
    assert_eq!(service.read("codex")["state"], "unchecked");
    tokio::time::timeout(Duration::from_secs(1), ping.entered.notified())
        .await
        .unwrap();
    ping.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.read("codex")["state"] != "responding" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ping.count.load(Ordering::SeqCst), 1);
    service.close();
    task.await.unwrap().unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn direct_completion_uses_owned_stdin_and_never_interprets_model_or_prompt_as_shell() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    let args = root.path().join("args");
    let input = root.path().join("input");
    let marker = root.path().join("must-not-exist");
    std::fs::write(&binary,format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf '%s\\n' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"Repair cache\"}}}}'\n",args.display(),input.display())).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let malicious = format!("gpt-5;touch {}", marker.display());
    let config =
        json!({"agents":{"binaries":{"codex":binary},"autoTitle":{"models":{"codex":malicious}}}});
    let prompt = format!("Title this: $(touch {})\nquotes \" ' & |", marker.display());
    assert_eq!(
        completion::title("codex", &config, root.path(), None, &prompt)
            .await
            .unwrap(),
        "Repair cache"
    );
    assert_eq!(std::fs::read_to_string(input).unwrap(), prompt);
    let argv = std::fs::read_to_string(args).unwrap();
    assert!(argv.contains(&malicious));
    assert!(!argv.contains("Title this"));
    assert!(!marker.exists());
}
#[cfg(unix)]
#[tokio::test]
async fn readiness_never_executes_unrecognized_script_launchers() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("provider");
    let marker = root.path().join("executed");
    std::fs::write(
        &binary,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        completion::claude_ping(&binary).await,
        Err(text::Failure::Unsupported)
    );
    assert_eq!(
        codex::ping(&binary, root.path()).await,
        Err(text::Failure::Unsupported)
    );
    assert!(!marker.exists());
}
#[test]
fn windows_shim_resolution_uses_declared_package_without_shell_or_path_escape() {
    let root = tempfile::tempdir().unwrap();
    let shim = root.path().join("codex.cmd");
    let package = root.path().join("node_modules/@openai/codex");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(&shim, "never interpreted shell text").unwrap();
    std::fs::write(root.path().join("node.exe"), "").unwrap();
    std::fs::write(package.join("cli.js"), "").unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@openai/codex","bin":{"codex":"cli.js"}}"#,
    )
    .unwrap();
    let argv = tools::launcher("codex", &shim).unwrap();
    assert_eq!(argv[0], root.path().join("node.exe").to_string_lossy());
    assert_eq!(
        argv[1],
        package
            .join("cli.js")
            .canonicalize()
            .unwrap()
            .to_string_lossy()
    );
    std::fs::write(root.path().join("outside.js"), "").unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@openai/codex","bin":{"codex":"../../../outside.js"}}"#,
    )
    .unwrap();
    assert!(tools::launcher("codex", &shim).is_err());
}
