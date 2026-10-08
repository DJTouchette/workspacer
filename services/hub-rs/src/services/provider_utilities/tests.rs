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
    let target = |config: &Value, agent: &str| {
        let t = text::title_target(config, agent);
        (t.provider, t.model, t.explicit)
    };
    let config = json!({"agents":{"autoTitle":{"model":"haiku","models":{"codex":"gpt-5.4-mini","claude":"gpt-5"}}}});
    // Default: the agent's own harness and that harness's own row.
    assert_eq!(
        target(&config, "codex"),
        ("codex".into(), Some("gpt-5.4-mini".into()), true)
    );
    // An explicit per-harness choice is passed exactly, never swapped for a
    // "servable" one; the CLI's rejection is recorded instead.
    assert_eq!(
        target(&config, "claude"),
        ("claude".into(), Some("gpt-5".into()), true)
    );
    // The legacy single field is only honoured where it can mean something.
    assert_eq!(
        target(&config, "opencode"),
        ("opencode".into(), None, false)
    );
    let legacy = json!({"agents":{"autoTitle":{"model":"haiku"}}});
    assert_eq!(
        target(&legacy, "claude"),
        ("claude".into(), Some("haiku".into()), false)
    );
    assert_eq!(target(&legacy, "codex"), ("codex".into(), None, false));
    assert_eq!(
        target(&json!({}), "claude"),
        ("claude".into(), Some("haiku".into()), false)
    );
    // A fixed title harness overrides the agent's, with that harness's model.
    let fixed = json!({"agents":{"autoTitle":{"provider":"codex","model":"haiku","models":{"codex":"gpt-5.4-mini","claude":"sonnet"}}}});
    assert_eq!(
        target(&fixed, "claude"),
        ("codex".into(), Some("gpt-5.4-mini".into()), true)
    );
    let blank = json!({"agents":{"autoTitle":{"provider":"  ","models":{"claude":"sonnet"}}}});
    assert_eq!(
        target(&blank, "claude"),
        ("claude".into(), Some("sonnet".into()), true)
    );
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
        completion::title(
            "codex",
            text::title_target(&config, "codex").model.as_deref(),
            &config,
            root.path(),
            None,
            &prompt
        )
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

/// Records every call and answers from a script, so title routing can be
/// pinned without a provider binary or a paid model call.
struct FakeTitles {
    calls: Mutex<Vec<(String, Option<String>, String)>>,
    answer: completion::Outcome<String>,
}
impl Generate for FakeTitles {
    fn generate<'a>(
        &'a self,
        provider: &'a str,
        model: Option<&'a str>,
        _config: &'a Value,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>
    {
        self.calls
            .lock()
            .unwrap()
            .push((provider.into(), model.map(str::to_owned), prompt.into()));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
    fn brief<'a>(
        &'a self,
        provider: &'a str,
        model: Option<&'a str>,
        config: &'a Value,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>
    {
        let _ = config;
        self.calls.lock().unwrap().push((
            provider.into(),
            model.map(str::to_owned),
            format!("[brief] {prompt}"),
        ));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}
fn titled(
    config: Value,
    answer: completion::Outcome<String>,
) -> (tempfile::TempDir, Arc<Service>, Arc<FakeTitles>) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Config::open(root.path().join("config.yaml")));
    store.save(config, true).unwrap();
    let fake = Arc::new(FakeTitles {
        calls: Mutex::new(Vec::new()),
        answer,
    });
    let service = Service::with_generator(store, root.path().into(), None, fake.clone());
    (root, service, fake)
}
#[tokio::test]
async fn suggest_routes_the_configured_harness_and_exact_model() {
    let (_root, service, fake) = titled(
        json!({"agents":{"autoTitle":{"provider":"codex","models":{"codex":"gpt-5.4-mini"}}}}),
        Ok("Title: Fix the login redirect.".into()),
    );
    let outcome = service
        .suggest(
            "claude",
            "fix the login redirect please",
            "Looking at it",
            true,
        )
        .await;
    assert_eq!(outcome.title.as_deref(), Some("Fix the login redirect"));
    assert_eq!(outcome.source, "model");
    assert_eq!(
        (outcome.provider.as_str(), outcome.model.as_deref()),
        ("codex", Some("gpt-5.4-mini"))
    );
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "codex");
    assert_eq!(calls[0].1.as_deref(), Some("gpt-5.4-mini"));
    assert!(calls[0].2.contains("User: fix the login redirect please"));
    assert!(calls[0].2.contains("Assistant: Looking at it"));
}
#[tokio::test]
async fn suggest_failure_is_a_labelled_fallback_never_a_model_title() {
    for (failure, reason) in [
        (text::Failure::Missing, "missing"),
        (text::Failure::Unsupported, "unsupported"),
        (text::Failure::Authentication, "unauthenticated"),
        (text::Failure::Timeout, "timeout"),
    ] {
        let (_root, service, fake) = titled(
            json!({"agents":{"autoTitle":{"models":{"claude":"claude-opus-5-5"}}}}),
            Err(failure),
        );
        let outcome = service
            .suggest("claude", "# Repair the cache\nmore detail", "", true)
            .await;
        assert_eq!(outcome.source, "fallback");
        assert_eq!(outcome.reason, Some(reason));
        assert_eq!(outcome.title.as_deref(), Some("Repair the cache"));
        assert_eq!(outcome.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }
    // A model that answers with prose or a refusal is not a title either.
    let (_root, service, _) = titled(json!({}), Ok("Sorry, I can't help with that".into()));
    let outcome = service.suggest("claude", "do the thing", "", true).await;
    assert_eq!(
        (outcome.source, outcome.reason),
        ("fallback", Some("empty"))
    );
    // Nothing to title from: no call at all.
    let (_root, service, fake) = titled(json!({}), Ok("unused".into()));
    let outcome = service.suggest("claude", "   ", "", true).await;
    assert_eq!((outcome.title, outcome.source), (None, "none"));
    assert!(fake.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn legacy_title_rpc_honours_the_fixed_harness_and_the_off_switch() {
    let (_root, service, fake) = titled(
        json!({"agents":{"autoTitle":{"provider":"codex","models":{"codex":"gpt-5.4-mini"}}}}),
        Ok("Repair the cache".into()),
    );
    assert_eq!(
        service
            .title(&json!({"userMessage":"fix cache","provider":"claude"}))
            .await
            .unwrap(),
        "Repair the cache"
    );
    assert_eq!(fake.calls.lock().unwrap()[0].0, "codex");
    service
        .config
        .save(json!({"agents":{"autoTitle":{"enabled":false}}}), true)
        .unwrap();
    assert!(!service.titles_enabled());
    assert_eq!(
        service
            .title(&json!({"userMessage":"fix cache","provider":"claude"}))
            .await
            .unwrap(),
        Value::Null
    );
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn handoff_summaries_use_the_title_target_and_report_failures_by_reason() {
    // Default: the session's own harness, Haiku for Claude.
    let (_root, service, fake) = titled(json!({}), Ok("## Goal\nShip it".into()));
    let outcome = service.summarize_handoff("claude", "the prompt").await;
    assert_eq!(outcome.text, Ok("## Goal\nShip it".into()));
    assert_eq!(
        (outcome.provider.as_str(), outcome.model.as_deref()),
        ("claude", Some("haiku"))
    );
    assert_eq!(fake.calls.lock().unwrap()[0].2, "[brief] the prompt");
    // An explicit per-harness choice is passed exactly; the titles' off
    // switch does not apply to a brief the user asked for.
    let (_root, service, fake) = titled(
        json!({"agents":{"autoTitle":{"enabled":false,"models":{"codex":"gpt-5.4-mini"}}}}),
        Err(text::Failure::Authentication),
    );
    let outcome = service.summarize_handoff("codex", "p").await;
    assert_eq!(outcome.text, Err("unauthenticated"));
    assert_eq!(outcome.model.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(fake.calls.lock().unwrap()[0].0, "codex");
    let (_root, service, _) = titled(json!({}), Ok("   ".into()));
    assert_eq!(
        service.summarize_handoff("claude", "p").await.text,
        Err("empty")
    );
}
#[test]
fn brief_limits_are_wider_than_titles_but_bounded() {
    let (title, brief) = (completion::Limits::TITLE, completion::Limits::BRIEF);
    assert_eq!((title.timeout.as_secs(), title.chars), (25, 416));
    assert!(!title.no_tools && brief.no_tools);
    assert!(brief.timeout > title.timeout && brief.timeout.as_secs() <= 120);
    assert!(brief.chars > 4_000 && brief.output <= 1024 * 1024);
}
