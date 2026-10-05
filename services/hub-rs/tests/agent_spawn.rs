use anyhow::bail;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use workspacer_hub::services::{
    agent_lifecycle::{LaunchEngine, LaunchPreparation, Lifecycle, Operation},
    agent_spawn::SpawnCoordinator,
    config::Config,
    routing::RoutingService,
    spawn_plan::Plan,
    task_store::TaskStore,
    workflow_runtime::WorkflowRuntime,
    workflows::WorkflowStore,
    worktrees::Worktrees,
};
#[derive(Default)]
struct Fake {
    plans: Mutex<Vec<Plan>>,
    fail_prepare: AtomicBool,
    fail_spawn: AtomicBool,
    leave_message_unqueued: AtomicBool,
    fail_message: AtomicBool,
    messages: Mutex<Vec<(String, String)>>,
    definitive_rejection: AtomicBool,
    end_owner: Mutex<Option<Arc<RwLock<Value>>>>,
}
impl LaunchPreparation for Fake {
    fn prepare<'a>(&'a self, _: &'a mut Plan, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            if self.fail_prepare.load(Ordering::SeqCst) {
                bail!("facade unavailable");
            }
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
impl LaunchEngine for Fake {
    fn sessions(&self) -> Operation<'_, Value> {
        Box::pin(async move {
            Ok(json!(
                self.plans
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|p| json!({"session_id":p.session_id,"mode":"input"}))
                    .collect::<Vec<_>>()
            ))
        })
    }
    fn spawn<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Value> {
        Box::pin(async move {
            if self.definitive_rejection.load(Ordering::SeqCst) {
                return Err(claudemon::daemon::embedded::CommandRejected {
                    status: 400,
                    message: "definitive provider refusal".into(),
                }
                .into());
            }
            if self.fail_spawn.load(Ordering::SeqCst) {
                bail!("engine acknowledgement lost");
            }
            self.plans.lock().unwrap().push(plan.clone());
            if let Some(owner) = self.end_owner.lock().unwrap().as_ref() {
                owner.write().unwrap()["status"] = json!("ended");
            }
            Ok(
                json!({"session_id":plan.session_id,"first_message_queued":!self.leave_message_unqueued.load(Ordering::SeqCst)}),
            )
        })
    }
    fn message<'a>(&'a self, session: &'a str, content: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.messages
                .lock()
                .unwrap()
                .push((session.into(), content.into()));
            if self.fail_message.load(Ordering::SeqCst) {
                bail!("message acknowledgement lost");
            }
            Ok(())
        })
    }
    fn stop<'a>(&'a self, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    project: String,
    coordinator: Arc<SpawnCoordinator>,
    fake: Arc<Fake>,
    owner: Arc<RwLock<Value>>,
    parents: Arc<RwLock<std::collections::BTreeMap<String, Value>>>,
    config: Arc<Config>,
}
fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
impl Fixture {
    fn new(ceiling: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("source.txt"), "fixture").unwrap();
        git(&project, &["init", "-q"]);
        git(&project, &["config", "user.email", "fixture@example.test"]);
        git(&project, &["config", "user.name", "Fixture"]);
        git(&project, &["add", "."]);
        git(&project, &["commit", "-qm", "initial"]);
        let config_dir = dir.path().join("config");
        std::fs::create_dir(&config_dir).unwrap();
        if let Some(ceiling) = ceiling {
            std::fs::write(
                config_dir.join("routing.yaml"),
                format!("ceilings:\n  default: {{max_capability: {ceiling}}}\n"),
            )
            .unwrap();
        }
        let cfg = Arc::new(Config::open(config_dir.join("config.yaml")));
        cfg.save(
            json!({"agents":{"worktreeRoot":dir.path().join("trees")}}),
            true,
        )
        .unwrap();
        let fake = Arc::new(Fake::default());
        let lifecycle =
            Lifecycle::open(dir.path().join("launches.json"), fake.clone(), fake.clone()).unwrap();
        let owner = Arc::new(RwLock::new(
            json!({"sessionId":"manager","isWakeTarget":true,"status":"active","cwd":project}),
        ));
        let observed = owner.clone();
        let parents = Arc::new(RwLock::new(
            std::collections::BTreeMap::<String, Value>::new(),
        ));
        let observed_parents = parents.clone();
        let lookup = Arc::new(move |id: &str| {
            if id == "manager" {
                Some(observed.read().unwrap().clone())
            } else {
                observed_parents.read().unwrap().get(id).cloned()
            }
        });
        let workflow = Arc::new(WorkflowRuntime::new(
            Arc::new(WorkflowStore::new(config_dir.clone(), cfg.clone())),
            Arc::new(TaskStore::open(config_dir.join("dispatch-history.json")).unwrap()),
            lookup,
        ));
        let trees = Worktrees::new(dir.path().join("home"), cfg.clone(), None);
        let routing = Arc::new(RoutingService::open(config_dir.clone()).unwrap());
        let coordinator = SpawnCoordinator::new(
            config_dir,
            dir.path().join("home"),
            cfg.clone(),
            lifecycle,
            workflow,
            trees,
            routing,
        );
        Self {
            project: project.to_string_lossy().into_owned(),
            dir,
            coordinator,
            fake,
            owner,
            parents,
            config: cfg,
        }
    }
    fn workflow_params(&self) -> Value {
        let started = self.coordinator.workflow.request(
            &json!({"op":"start","cwd":self.project,"title":"Implement fixture"}),
            "manager",
        );
        assert_eq!(started["ok"], true, "{started}");
        let plan = &started["dispatch"];
        json!({"cwd":self.project,"parentSessionId":"manager","dispatchOwnerSessionId":"manager","taskId":plan["taskId"],"workflowStepId":plan["stepId"],"expectedTaskRevision":plan["expectedTaskRevision"],"stage":plan["stage"],"role":plan["role"],"template":plan["template"],"provider":"codex","model":"gpt-5.4","templateParams":{"task":"Implement fixture"}})
    }
}
#[tokio::test]
async fn actual_worktree_template_and_workflow_are_bound_before_engine_admission() {
    let fixture = Fixture::new(None);
    let params = fixture.workflow_params();
    let receipt = fixture
        .coordinator
        .spawn_sanitized(params.clone())
        .await
        .unwrap();
    assert_eq!(receipt["messageQueued"], true);
    assert_eq!(receipt["worktree"]["allocated"], true);
    assert!(receipt["dispatchId"].is_string());
    let plans = fixture.fake.plans.lock().unwrap();
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    let cwd = plan.request["cwd"].as_str().unwrap();
    assert_ne!(cwd, fixture.project);
    assert!(receipt["renderedMessage"].as_str().unwrap().contains(cwd));
    assert!(
        plan.request["instructions"]
            .as_str()
            .unwrap()
            .contains("wks-result")
    );
    assert!(
        plan.request["instructions"]
            .as_str()
            .unwrap()
            .contains("wks-escalation")
    );
    drop(plans);
    let task = fixture
        .coordinator
        .workflow
        .tasks
        .task(params["taskId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(task["workflow"]["steps"][0]["state"], "dispatched");
    assert!(task.get("dispatchReservation").is_none());
    assert_eq!(task["attempts"].as_array().unwrap().len(), 1);
    fixture.coordinator.close().await;
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(json!({"cwd":fixture.project}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn early_failure_releases_workflow_claim_but_uncertain_engine_failure_does_not() {
    let fixture = Fixture::new(None);
    let params = fixture.workflow_params();
    fixture.fake.fail_prepare.store(true, Ordering::SeqCst);
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(params.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("facade unavailable")
    );
    let id = params["taskId"].as_str().unwrap().to_owned();
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(&id)
            .unwrap()
            .unwrap()
            .get("dispatchReservation")
            .is_none()
    );
    fixture.fake.fail_prepare.store(false, Ordering::SeqCst);
    fixture.fake.fail_spawn.store(true, Ordering::SeqCst);
    let task = fixture
        .coordinator
        .workflow
        .tasks
        .task(&id)
        .unwrap()
        .unwrap();
    let mut retry = params;
    retry["expectedTaskRevision"] = task["revision"].clone();
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(retry)
            .await
            .unwrap_err()
            .to_string()
            .contains("do not retry blindly")
    );
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(&id)
            .unwrap()
            .unwrap()
            .get("dispatchReservation")
            .is_some()
    );
}
#[tokio::test]
async fn committed_engine_acceptance_remains_a_receipt_when_tracking_becomes_unavailable() {
    let fixture = Fixture::new(None);
    let params = fixture.workflow_params();
    *fixture.fake.end_owner.lock().unwrap() = Some(fixture.owner.clone());
    let receipt = fixture
        .coordinator
        .spawn_sanitized(params.clone())
        .await
        .unwrap();
    assert!(receipt["sessionId"].is_string());
    assert_eq!(receipt["workflowAdmissionPending"], true);
    assert_eq!(receipt["dispatchHistoryUnavailable"], true);
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(params["taskId"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .get("dispatchReservation")
            .is_some()
    );
}
#[tokio::test]
async fn effective_profile_model_cannot_override_ceiling_and_unsupported_adapters_have_no_launch_side_effect()
 {
    let fixture = Fixture::new(Some("cheap"));
    std::fs::write(
        fixture.dir.path().join("config/claude-profiles.json"),
        serde_json::to_vec(
            &json!({"profiles":[{"id":"pinned","name":"Pinned","extraArgs":["--model","opus"]}]}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(json!({"cwd":fixture.project,"profileId":"pinned"}))
            .await
            .is_err()
    );
    assert!(fixture.fake.plans.lock().unwrap().is_empty());
    for params in [
        json!({"cwd":fixture.project,"targetHub":"remote"}),
        json!({"cwd":fixture.project,"launchIntegrationId":"plugin"}),
    ] {
        assert!(fixture.coordinator.spawn_sanitized(params).await.is_err());
    }
    assert!(fixture.coordinator.lifecycle.records().is_empty());
    assert!(!fixture.dir.path().join("trees").exists());
}

#[tokio::test]
async fn acknowledged_tracking_can_recover_without_launching_another_process() {
    let fixture = Fixture::new(None);
    let params = fixture.workflow_params();
    *fixture.fake.end_owner.lock().unwrap() = Some(fixture.owner.clone());
    let receipt = fixture
        .coordinator
        .spawn_sanitized(params.clone())
        .await
        .unwrap();
    assert_eq!(receipt["workflowAdmissionPending"], true);
    fixture.owner.write().unwrap()["status"] = json!("active");
    fixture.coordinator.recover_tracking().await.unwrap();
    assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
    let task = fixture
        .coordinator
        .workflow
        .tasks
        .task(params["taskId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(task["workflow"]["steps"][0]["state"], "dispatched");
    assert!(task.get("dispatchReservation").is_none());
}
#[tokio::test]
async fn resume_keeps_provider_parent_label_and_result_contract_when_fields_are_omitted() {
    let fixture = Fixture::new(None);
    let first=fixture.coordinator.spawn_sanitized(json!({"cwd":fixture.project,"provider":"codex","label":"Keep me","parentSessionId":"manager","resultSchema":{"type":"object"}})).await.unwrap();
    let id = first["sessionId"].as_str().unwrap();
    let generation = fixture.coordinator.lifecycle.records()[id]
        .generation
        .clone();
    fixture.fake.plans.lock().unwrap().clear();
    fixture
        .coordinator
        .lifecycle
        .stopped(id, &generation)
        .await
        .unwrap();
    fixture
        .coordinator
        .spawn_sanitized(json!({"resumeSessionId":id}))
        .await
        .unwrap();
    let plans = fixture.fake.plans.lock().unwrap();
    let plan = &plans[0];
    assert_eq!(plan.provider, "codex");
    assert_eq!(plan.metadata["label"], "Keep me");
    assert_eq!(plan.metadata["parentSessionId"], "manager");
    assert!(
        plan.request["instructions"]
            .as_str()
            .unwrap()
            .contains("wks-result")
    );
}
#[cfg(unix)]
#[tokio::test]
async fn shutdown_interrupts_worktree_setup_and_releases_unexecuted_workflow_claim() {
    let fixture = Fixture::new(None);
    Config::open(fixture.dir.path().join("config/config.yaml")).save(json!({"projects":{fixture.project.clone():{"worktreeSetup":["echo $$ > started; exec sleep 120"]}}}),true).unwrap();
    let params = fixture.workflow_params();
    let id = params["taskId"].as_str().unwrap().to_owned();
    let launched = {
        let coordinator = fixture.coordinator.clone();
        tokio::spawn(async move { coordinator.spawn_sanitized(params).await })
    };
    let marker = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(entries) = std::fs::read_dir(fixture.dir.path().join("trees/project")) {
                if let Some(path) = entries
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path().join("started"))
                    .find(|p| {
                        std::fs::read_to_string(p)
                            .ok()
                            .is_some_and(|v| v.trim().parse::<u32>().is_ok())
                    })
                {
                    break path;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.coordinator.close(),
    )
    .await
    .unwrap();
    assert!(launched.await.unwrap().is_err());
    assert!(fixture.fake.plans.lock().unwrap().is_empty());
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(&id)
            .unwrap()
            .unwrap()
            .get("dispatchReservation")
            .is_none()
    );
    #[cfg(unix)]
    {
        let pid = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn explicit_daemon_rejection_releases_claim_without_claiming_unknown_execution() {
    let fixture = Fixture::new(None);
    let params = fixture.workflow_params();
    fixture
        .fake
        .definitive_rejection
        .store(true, Ordering::SeqCst);
    let error = fixture
        .coordinator
        .spawn_sanitized(params.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("definitive provider refusal"));
    assert!(!error.contains("do not retry blindly"));
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(params["taskId"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .get("dispatchReservation")
            .is_none()
    );
}

struct RemoteExecution {
    coordinator: Arc<SpawnCoordinator>,
    cwd: String,
}
impl workspacer_hub::services::remote_dispatch::Execution for RemoteExecution {
    fn capabilities(
        &self,
    ) -> Operation<'_, workspacer_hub::services::remote_dispatch::Capabilities> {
        use workspacer_hub::services::remote_dispatch::*;
        Box::pin(async move {
            Ok(Capabilities {
                protocol: PROTOCOL,
                exact_model: true,
                executes: true,
                scope: "local".into(),
                providers: vec![Provider {
                    provider: "claude".into(),
                    found: true,
                    authenticated: Some(true),
                    note: String::new(),
                }],
                cwds: vec![Directory {
                    path: self.cwd.clone(),
                    source: "fixture".into(),
                    git: true,
                }],
                unsupported_reason: None,
            })
        })
    }
    fn canonical_directory<'a>(&'a self, cwd: &'a str) -> Operation<'a, String> {
        Box::pin(async move { Ok(std::fs::canonicalize(cwd)?.to_string_lossy().into_owned()) })
    }
    fn allocate<'a>(
        &'a self,
        _repo: &'a str,
        _cwd: &'a str,
        _branch: &'a str,
    ) -> Operation<'a, ()> {
        Box::pin(async { bail!("not used") })
    }
    fn cleanup<'a>(&'a self, _repo: &'a str, _cwd: &'a str, _branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async { bail!("not used") })
    }
    fn spawn<'a>(
        &'a self,
        caller: workspacer_hub::Caller,
        admission: workspacer_hub::services::remote_dispatch::RemoteAdmission,
        params: Value,
    ) -> Operation<'a, Value> {
        Box::pin(async move {
            self.coordinator
                .spawn_remote(caller, admission, params)
                .await
        })
    }
}
#[tokio::test]
async fn consumed_remote_lease_pins_identity_and_never_creates_local_parent_authority() {
    use workspacer_hub::{Caller, Hub, Options, services::remote_dispatch::Receiver};
    let mut fixture = Fixture::new(None);
    // Execution readiness advertises canonical choices, and prepare must echo
    // that exact choice. A raw Windows tempdir path violates the fake's contract.
    fixture.project = workspacer_hub::services::paths::canonicalize(Path::new(&fixture.project))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let receiver = Receiver::open(
        fixture.dir.path().join("remote"),
        hub.handle(),
        Arc::new(RemoteExecution {
            coordinator: fixture.coordinator.clone(),
            cwd: fixture.project.clone(),
        }),
    )
    .unwrap();
    let caller = Caller {
        call_id: 1,
        activity_seq: 0,
        connection_id: 1,
        federated: true,
        authenticated_host: false,
        trusted: false,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: "paired-fingerprint".into(),
    };
    let origin =
        json!({"protocol":2,"dispatchId":"abcdefghijklmnop","ownerKey":"paired-fingerprint"});
    receiver
        .prepare(
            caller.clone(),
            json!({"remoteOrigin":origin,"cwd":fixture.project,"provider":"claude"}),
        )
        .await
        .unwrap();
    let params = json!({"remoteOrigin":origin,"cwd":fixture.project,"provider":"claude","message":"remote work","parentSessionId":"manager","dispatchOwnerSessionId":"manager","trackTask":true,"worktree":true});
    let receipt = receiver
        .spawn(caller.clone(), params.clone())
        .await
        .unwrap();
    assert!(receipt["sessionId"].is_string());
    let plan = fixture.fake.plans.lock().unwrap()[0].clone();
    assert_eq!(plan.session_id, receipt["sessionId"].as_str().unwrap());
    assert!(
        plan.metadata["parentSessionId"]
            .as_str()
            .is_none_or(str::is_empty)
    );
    assert_eq!(plan.metadata["isWakeTarget"], false);
    assert_eq!(
        plan.metadata["remoteOrigin"],
        json!({"protocol":2,"dispatchId":"abcdefghijklmnop"})
    );
    assert!(
        plan.request["instructions"]
            .as_str()
            .unwrap()
            .contains("wks-escalation")
    );
    assert!(
        fixture
            .coordinator
            .workflow
            .tasks
            .list()
            .unwrap()
            .is_empty()
    );
    assert_eq!(plan.request["cwd"], fixture.project);
    assert!(receiver.spawn(caller, params).await.is_err());
    assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
    receiver.close().await;
    fixture.coordinator.close().await;
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn fleet_full_access_follows_recorded_ancestry_and_current_config_for_each_provider() {
    for provider in ["claude", "codex", "copilot"] {
        let fixture = Fixture::new(None);
        fixture.config.save(json!({"agents":{"fleetFullAccess":true},"claude":{"skipPermissionsDefault":false,"defaultPermissionMode":"default"}}),true).unwrap();
        fixture.parents.write().unwrap().extend([
            (
                "cycle-a".into(),
                json!({"sessionId":"cycle-a","parentSessionId":"cycle-b"}),
            ),
            (
                "cycle-b".into(),
                json!({"sessionId":"cycle-b","parentSessionId":"cycle-a"}),
            ),
            (
                "foreign".into(),
                json!({"sessionId":"foreign","hub":"peer","isWakeTarget":true}),
            ),
        ]);
        let mut worker = String::new();
        for role in [
            "manager",
            "worker",
            "grandchild",
            "ordinary",
            "disabled",
            "cycle",
            "unknown",
            "foreign",
        ] {
            fixture
                .config
                .save(json!({"agents":{"fleetFullAccess":role!="disabled"}}), true)
                .unwrap();
            let mut params = json!({"provider":provider,"transport":"stream","cwd":fixture.project,"skipPermissions":false,"trackTask":false});
            match role {
                "manager" | "disabled" => params["manager"] = json!(true),
                "worker" => params["parentSessionId"] = json!("manager"),
                "grandchild" => params["parentSessionId"] = json!(worker),
                "cycle" => params["parentSessionId"] = json!("cycle-a"),
                "unknown" => params["parentSessionId"] = json!("absent"),
                "foreign" => params["parentSessionId"] = json!("foreign"),
                _ => (),
            }
            let result = fixture.coordinator.spawn_sanitized(params).await.unwrap();
            if role == "worker" {
                worker = result["sessionId"].as_str().unwrap().to_owned();
            }
            let full = matches!(role, "manager" | "worker" | "grandchild");
            assert_eq!(result["fullAccess"], full, "{provider}/{role}");
            let plans = fixture.fake.plans.lock().unwrap();
            let plan = plans.last().unwrap();
            assert_eq!(
                plan.full_access, full,
                "{provider}/{role}: actual admitted plan"
            );
            let expected = match (provider, full) {
                ("claude", true) => "bypassPermissions",
                ("claude", false) => "default",
                (_, true) => "yolo",
                (_, false) => "ask",
            };
            assert_eq!(
                plan.metadata["settings"]["permissionMode"], expected,
                "{provider}/{role}"
            );
        }
        fixture.coordinator.close().await;
        fixture.coordinator.lifecycle.close().await;
    }
}
/// `agents.childFullAccess`: the user's explicit choice that NEW child
/// launches of this hub's own sessions bypass provider approvals. Off by
/// default; never for parentless, unknown-parent, foreign-parent or resumed
/// launches; it wins over the parent's request like fleetFullAccess.
#[tokio::test]
async fn child_full_access_applies_only_to_new_children_of_local_sessions() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(None);
        fixture.parents.write().unwrap().insert(
            "foreign".into(),
            json!({"sessionId":"foreign","hub":"peer"}),
        );
        let (mut parent, mut child) = (String::new(), String::new());
        for role in [
            "parent",
            "child",
            "default",
            "orphan",
            "unknown",
            "foreign",
            "resume",
            "grandchild",
        ] {
            let enabled = role != "default";
            fixture
                .config
                .save(
                    json!({"agents":{"childFullAccess":enabled,"fleetFullAccess":false},
                        "claude":{"skipPermissionsDefault":false,"defaultPermissionMode":"default"}}),
                    true,
                )
                .unwrap();
            let mut params = json!({"provider":provider,"transport":"stream","cwd":fixture.project,
                "skipPermissions":false,"permissionMode":"default","trackTask":false});
            match role {
                "child" | "default" => params["parentSessionId"] = json!(parent),
                "grandchild" => params["parentSessionId"] = json!(child),
                "unknown" => params["parentSessionId"] = json!("absent"),
                "foreign" => params["parentSessionId"] = json!("foreign"),
                "resume" => {
                    params["parentSessionId"] = json!(parent);
                    params["resumeSessionId"] = json!("resumed-child");
                }
                _ => (),
            }
            let result = fixture.coordinator.spawn_sanitized(params).await.unwrap();
            match role {
                "parent" => parent = result["sessionId"].as_str().unwrap().to_owned(),
                "child" => child = result["sessionId"].as_str().unwrap().to_owned(),
                _ => (),
            }
            let full = matches!(role, "child" | "grandchild");
            assert_eq!(result["fullAccess"], full, "{provider}/{role}");
            let plans = fixture.fake.plans.lock().unwrap();
            let plan = plans.last().unwrap();
            assert_eq!(plan.full_access, full, "{provider}/{role}: admitted plan");
            let expected = match (provider, full) {
                ("claude", true) => "bypassPermissions",
                ("claude", false) => "default",
                (_, true) => "yolo",
                (_, false) => "ask",
            };
            assert_eq!(
                plan.metadata["settings"]["permissionMode"], expected,
                "{provider}/{role}"
            );
        }
        fixture.coordinator.close().await;
        fixture.coordinator.lifecycle.close().await;
    }
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_project_workflow_spawns_with_canonical_or_legacy_manager_and_task_metadata() {
    use std::os::unix::fs::symlink;
    for manager_alias in [false, true] {
        for legacy_task_alias in [false, true] {
            let mut fixture = Fixture::new(None);
            let canonical =
                workspacer_hub::services::paths::canonicalize(Path::new(&fixture.project))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
            let alias = fixture.dir.path().join("project-alias");
            symlink(&canonical, &alias).unwrap();
            let alias_text = alias.to_string_lossy().into_owned();
            fixture.owner.write().unwrap()["cwd"] = json!(if manager_alias {
                &alias_text
            } else {
                &canonical
            });
            fixture.project = alias_text.clone();
            let mut params = fixture.workflow_params();
            let id = params["taskId"].as_str().unwrap().to_owned();
            let created = fixture
                .coordinator
                .workflow
                .tasks
                .task(&id)
                .unwrap()
                .unwrap();
            assert_eq!(
                created["projectCwd"], canonical,
                "new ingress stores the selected canonical project"
            );
            if legacy_task_alias {
                // Old Go/headless workflow ingress stored an absolute alias
                // verbatim before spawnCore resolved the actual directory.
                fixture
                    .coordinator
                    .workflow
                    .tasks
                    .transaction(|history| {
                        history.task_mut(&id)?["projectCwd"] = json!(alias_text);
                        Ok(())
                    })
                    .unwrap();
                params["expectedTaskRevision"] =
                    json!(workspacer_hub::services::task_store::revision(
                        &fixture
                            .coordinator
                            .workflow
                            .tasks
                            .task(&id)
                            .unwrap()
                            .unwrap()
                    ));
            }
            let denied = fixture.coordinator.workflow.request(
                &json!({"op":"next","taskId":id,"cwd":alias_text}),
                "other-manager",
            );
            assert_eq!(denied["ok"], false);
            assert!(denied.get("task").is_none());
            if !legacy_task_alias {
                let different = fixture.dir.path().join("different-project");
                std::fs::create_dir(&different).unwrap();
                std::fs::remove_file(&alias).unwrap();
                symlink(&different, &alias).unwrap();
                assert!(
                    fixture
                        .coordinator
                        .spawn_sanitized(params.clone())
                        .await
                        .is_err()
                );
                assert!(
                    fixture.fake.plans.lock().unwrap().is_empty(),
                    "repointed selector may not launch"
                );
                std::fs::remove_file(&alias).unwrap();
                symlink(&canonical, &alias).unwrap();
            }
            let receipt = fixture.coordinator.spawn_sanitized(params).await.unwrap();
            assert!(receipt["sessionId"].is_string());
            assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
            let plan = fixture.fake.plans.lock().unwrap()[0].clone();
            assert_eq!(plan.metadata["projectCwd"], canonical);
            assert_eq!(plan.metadata["parentSessionId"], "manager");
            assert!(Path::new(plan.request["cwd"].as_str().unwrap()).is_dir());
            fixture.coordinator.close().await;
        }
    }
}

#[tokio::test]
async fn rendered_receipt_truncates_unicode_codepoints_without_truncating_the_launch_prompt() {
    let fixture = Fixture::new(None);
    let directory = fixture.dir.path().join("config");
    let library = directory.join("library");
    std::fs::create_dir_all(&library).unwrap();
    for repeats in [4000, 4001] {
        let body = "界🦀e\u{301}".repeat(repeats);
        std::fs::write(
            library.join("unicode.md"),
            format!("---\ntitle: Unicode\nkind: dispatch\n---\n{body}"),
        )
        .unwrap();
        let items = workspacer_hub::services::library::Library::new(directory.clone())
            .list(&json!({"cwd":fixture.project,"kind":"dispatch"}))
            .unwrap();
        let template = items
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["title"] == "Unicode")
            .unwrap()["id"]
            .clone();
        let receipt = fixture.coordinator.spawn_sanitized(json!({"cwd":fixture.project,"provider":"codex","model":"gpt-5.4","template":template,"trackTask":false})).await.unwrap();
        let expected: String = body.chars().take(16_000).collect();
        assert_eq!(receipt["renderedMessage"], expected);
        assert_eq!(
            receipt["renderedMessage"].as_str().unwrap().chars().count(),
            16_000
        );
        if repeats == 4001 {
            assert_eq!(receipt["renderedMessageTruncated"], true);
        } else {
            assert!(receipt.get("renderedMessageTruncated").is_none());
        }
        assert_eq!(
            fixture.fake.plans.lock().unwrap().last().unwrap().request["first_message"],
            body
        );
        assert!(
            fixture.fake.messages.lock().unwrap().is_empty(),
            "engine-queued prompt must not be sent twice"
        );
    }
    fixture.coordinator.close().await;
}

#[tokio::test]
async fn forged_task_owner_is_rejected_before_any_engine_or_worktree_admission() {
    let fixture = Fixture::new(None);
    let mut params = fixture.workflow_params();
    fixture.parents.write().unwrap().insert(
        "forged".into(),
        json!({"sessionId":"forged","cwd":fixture.project,"isWakeTarget":true,"status":"active"}),
    );
    params["dispatchOwnerSessionId"] = json!("forged");
    let task_id = params["taskId"].as_str().unwrap();
    let before = fixture
        .coordinator
        .workflow
        .tasks
        .task(task_id)
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(params.clone())
            .await
            .is_err()
    );
    assert!(fixture.fake.plans.lock().unwrap().is_empty());
    assert!(fixture.fake.messages.lock().unwrap().is_empty());
    assert!(fixture.coordinator.lifecycle.records().is_empty());
    assert_eq!(
        fixture
            .coordinator
            .workflow
            .tasks
            .task(task_id)
            .unwrap()
            .unwrap(),
        before
    );
    assert!(!fixture.dir.path().join("trees").exists());
    fixture.coordinator.close().await;
}

#[tokio::test]
async fn acknowledged_launch_falls_back_once_and_preserves_receipt_on_message_uncertainty() {
    for failed_message in [false, true] {
        let fixture = Fixture::new(None);
        fixture
            .fake
            .leave_message_unqueued
            .store(true, Ordering::SeqCst);
        fixture
            .fake
            .fail_message
            .store(failed_message, Ordering::SeqCst);
        let receipt = fixture.coordinator.spawn_sanitized(json!({"cwd":fixture.project,"provider":"codex","model":"gpt-5.4","message":"Exact initial prompt 🦀","trackTask":false})).await.unwrap();
        let id = receipt["sessionId"].as_str().unwrap();
        assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
        assert_eq!(
            *fixture.fake.messages.lock().unwrap(),
            vec![(id.into(), "Exact initial prompt 🦀".into())]
        );
        assert_eq!(receipt["messageQueued"], !failed_message);
        if failed_message {
            assert!(
                receipt["messageError"]
                    .as_str()
                    .unwrap()
                    .contains("do not retry automatically")
            );
        }
        let records = fixture.coordinator.lifecycle.records();
        assert_eq!(
            records[id].receipt.as_ref().unwrap()["messageQueued"],
            !failed_message
        );
        fixture.coordinator.close().await;
        assert_eq!(fixture.fake.messages.lock().unwrap().len(), 1);
    }
    let fixture = Fixture::new(None);
    fixture.fake.fail_spawn.store(true, Ordering::SeqCst);
    assert!(fixture.coordinator.spawn_sanitized(json!({"cwd":fixture.project,"provider":"codex","model":"gpt-5.4","message":"Never replay after unknown spawn","trackTask":false})).await.is_err());
    assert!(fixture.fake.messages.lock().unwrap().is_empty());
    fixture.coordinator.close().await;
}

#[tokio::test]
async fn owned_routing_audit_is_once_across_both_resolutions_and_records_refusals() {
    for (decision, extra, expected) in [
        (
            "owned-clamp",
            json!({"provider":"codex","model":"gpt-5.6-sol","effort":"high","capability":"frontier"}),
            "clamped",
        ),
        (
            "owned-fresh",
            json!({"provider":"codex","model":"gpt-5.6-terra","role":"reviewer","resumeSessionId":"existing-worker"}),
            "refused",
        ),
    ] {
        let fixture = Fixture::new(Some("cheap"));
        let mut params = extra;
        params["cwd"] = json!(fixture.project);
        params["decisionId"] = json!(decision);
        params["message"] = json!("SECRET_OWNED_PROMPT");
        params["env"] = json!({"SECRET":"SECRET_OWNED_ENV"});
        let caller = workspacer_hub::Caller {
            call_id: 1,
            activity_seq: 1,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: "fixture-fingerprint".into(),
            federated: false,
        };
        let result = fixture.coordinator.spawn_for(caller, params).await;
        if expected == "refused" {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("existing-worker"), "{error}");
            assert!(fixture.fake.plans.lock().unwrap().is_empty());
        } else {
            result.unwrap();
            assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
        }
        let raw =
            std::fs::read_to_string(fixture.dir.path().join("config/routing-decisions.jsonl"))
                .unwrap();
        let rows: Vec<Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "preliminary/final routing checks must not duplicate audit rows"
        );
        assert_eq!(rows[0]["decisionId"], decision);
        assert_eq!(rows[0]["spawn"]["outcome"], expected);
        assert_eq!(rows[0]["spawn"]["callerTokenId"], "fixture-fingerprint");
        assert!(!raw.contains("SECRET_OWNED"));
        fixture.coordinator.close().await;
    }
}

#[tokio::test]
async fn reserved_spawn_spellings_do_not_grant_caller_workflow_or_project_authority() {
    let fixture = Fixture::new(None);
    let mut params = fixture.workflow_params();
    params["projectCwd"] = json!("/caller-forged-project");
    params["workflowReservationToken"] = json!("caller-forged-reservation");
    params["op"] = json!("cancel");
    params["intents"] = json!([{"cwd":fixture.project}]);
    fixture.coordinator.spawn_sanitized(params).await.unwrap();
    {
        let plans = fixture.fake.plans.lock().unwrap();
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(
            plan.metadata["projectCwd"],
            json!(
                workspacer_hub::services::paths::canonicalize(Path::new(&fixture.project)).unwrap()
            )
        );
        let token = plan.metadata["workflowReservationToken"].as_str().unwrap();
        assert!(!token.is_empty() && token != "caller-forged-reservation");
        for key in ["projectCwd", "workflowReservationToken", "op", "intents"] {
            assert!(plan.request.get(key).is_none(), "{key}");
        }
    }
    assert!(
        fixture
            .coordinator
            .spawn_sanitized(json!({"cwd":fixture.project,"targetHub":"remote"}))
            .await
            .is_err()
    );
    assert_eq!(fixture.fake.plans.lock().unwrap().len(), 1);
    fixture.coordinator.close().await;
}
