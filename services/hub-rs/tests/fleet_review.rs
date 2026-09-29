use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
use workspacer_hub::services::{
    config::Config,
    fleet_review::ReviewStore,
    task_store::TaskStore,
    wakes::{Capture, Delivery, Inventory, Wakes},
    worktrees::Worktrees,
};
fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn commit(cwd: &Path) {
    git(cwd, &["add", "."]);
    git(cwd, &["commit", "-qm", "fixture"]);
}
fn request(id: &str, file: Option<&str>) -> Value {
    let mut r = json!({"ownerSessionId":"manager","workerSessionId":"worker","evidenceId":id});
    if let Some(file) = file {
        r["file"] = json!(file);
    }
    r
}
struct Fixture {
    root: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    config: Arc<Config>,
    review: Arc<ReviewStore>,
    trees: Arc<Worktrees>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let home = root.path().join("home");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&home).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "fixture@example.test"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        std::fs::write(repo.join("first.txt"), "base\n").unwrap();
        std::fs::write(repo.join("delete.txt"), "remove me\n").unwrap();
        commit(&repo);
        let directory = root.path().join("config");
        let config = Arc::new(Config::open(directory.join("config.yaml")));
        config
            .save(
                json!({"agents":{"worktreeRoot":root.path().join("trees")}}),
                true,
            )
            .unwrap();
        let review = ReviewStore::new(directory, home.clone());
        let trees = Worktrees::new(home.clone(), config.clone(), None);
        trees.set_review_store(review.clone());
        Self {
            root,
            repo,
            home,
            config,
            review,
            trees,
        }
    }
    async fn allocate(&self) -> (PathBuf, Value) {
        let created = self
            .trees
            .create(json!({"repoCwd":self.repo,"name":"worker"}))
            .await
            .unwrap();
        assert_eq!(created["ok"], true, "{created}");
        let allocation = created["reviewAllocation"].clone();
        self.review
            .register("manager", "worker", allocation.clone())
            .unwrap();
        (PathBuf::from(created["path"].as_str().unwrap()), allocation)
    }
    fn persisted(&self) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.root.path().join("config/fleet-review.json")).unwrap(),
        )
        .unwrap()
    }
}
#[tokio::test]
async fn production_teardown_preserves_full_range_for_finish_wake_history_and_restart() {
    use claudemon::daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    };
    let mut f = Fixture::new();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: f.root.path().join("engine.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    f.trees = Worktrees::new(f.home.clone(), f.config.clone(), Some(engine.client()));
    f.trees.set_review_store(f.review.clone());
    let (cwd, allocation) = f.allocate().await;
    std::fs::write(cwd.join("first.txt"), "first change\n").unwrap();
    commit(&cwd);
    std::fs::write(cwd.join("second.txt"), "second change\n").unwrap();
    commit(&cwd);
    let head = git(&cwd, &["rev-parse", "HEAD"]);
    let parent = json!({"sessionId":"manager","cwd":f.repo,"isWakeTarget":true,"status":"active","ambientState":"idle","lastActivity":0});
    let rows = Arc::new(Mutex::new(BTreeMap::from([
        ("manager".to_owned(), parent.clone()),
        (
            "worker".to_owned(),
            json!({"sessionId":"worker","parentSessionId":"manager","cwd":"/worker-supplied/wrong","status":"active","ambientState":"streaming","lastActivity":1}),
        ),
    ])));
    let lookup_rows = rows.clone();
    let lookup = Arc::new(move |id: &str| lookup_rows.lock().unwrap().get(id).cloned());
    let list_rows = rows.clone();
    let list: Inventory = Arc::new(move || list_rows.lock().unwrap().values().cloned().collect());
    let capture: Capture = Arc::new(|_| {
        Box::pin(async {
            Ok(
                json!({"items":[{"kind":"user_message","text":"task"},{"kind":"assistant_text","text":"commit: fabricated"}]}),
            )
        })
    });
    let sent = Arc::new(Mutex::new(Vec::new()));
    let output = sent.clone();
    let delivery: Delivery = Arc::new(move |_, text, _| {
        let output = output.clone();
        Box::pin(async move {
            output.lock().unwrap().push(text);
            Ok(())
        })
    });
    let history = Arc::new(TaskStore::open(f.root.path().join("tasks.json")).unwrap());
    let accepted = history
        .accept(
            json!({"owner":parent,"sessionId":"worker","projectCwd":f.repo,"executionCwd":cwd}),
            || Ok(()),
        )
        .unwrap()
        .unwrap();
    let wakes = Wakes::new(lookup, list, capture, delivery, Some(history.clone()), None);
    wakes.set_review_store(f.review.clone());
    wakes.prime(&rows.lock().unwrap().values().cloned().collect::<Vec<_>>());
    let row = {
        let mut rows = rows.lock().unwrap();
        let worker = rows.get_mut("worker").unwrap();
        worker["ambientState"] = json!("idle");
        worker["lastActivity"] = json!(100);
        worker.clone()
    };
    wakes.observe(&row, 100);
    assert_eq!(f.trees.remove(&cwd).await.unwrap()["ok"], true);
    assert!(!cwd.exists());
    wakes.tick(1600).await.unwrap();
    let task = history
        .task(accepted["taskId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    let id = task["attempts"][0]["reviewEvidenceId"].as_str().unwrap();
    assert!(sent.lock().unwrap()[0].contains(id));
    let before = f.review.read(&request(id, Some("first.txt")));
    assert_eq!(before["evidence"]["headCommit"], head);
    assert_eq!(before["evidence"]["baseCommit"], allocation["baseCommit"]);
    assert!(
        before["evidence"]["files"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+first change")
    );
    git(
        &f.repo,
        &["branch", "-D", allocation["branch"].as_str().unwrap()],
    );
    std::fs::write(f.repo.join("first.txt"), "unrelated manager HEAD\n").unwrap();
    commit(&f.repo);
    let reopened = ReviewStore::new(f.root.path().join("config"), f.home.clone());
    assert_eq!(reopened.read(&request(id, Some("first.txt"))), before);
    assert!(
        reopened.read(&request(id, None))["evidence"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.get("diff").is_none())
    );
    assert_eq!(
        reopened.forget(&request(id, None)).unwrap(),
        json!({"ok":true})
    );
    assert!(
        f.review
            .capture("manager", "worker", "session-ended")
            .await
            .unwrap()
            .is_none()
    );
    engine.shutdown().await.unwrap();
}
#[tokio::test]
async fn adoption_readers_and_exact_selectors_preserve_immutable_records() {
    let f = Fixture::new();
    let (cwd, _) = f.allocate().await;
    std::fs::write(cwd.join("first.txt"), "changed\n").unwrap();
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    let before = f.review.read(&request(&id, Some("first.txt")));
    for patch in [
        json!({"ownerSessionId":"wrong"}),
        json!({"workerSessionId":"wrong"}),
        json!({"evidenceId":"wrong"}),
        json!({"file":"../first.txt"}),
        json!({"file":"/etc/passwd"}),
        json!({"file":":(glob)*"}),
        json!({"revision":"HEAD"}),
        json!({"cwd":f.repo}),
        json!({"command":"status"}),
    ] {
        let mut r = request(&id, None);
        r.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(f.review.read(&r)["ok"], false);
    }
    assert_eq!(
        f.review.forget(&request(&id, Some("missing"))).unwrap()["ok"],
        false
    );
    assert_eq!(f.review.read(&request(&id, Some("first.txt"))), before);
    f.review.adopt_owner("manager", "successor").unwrap();
    f.review.adopt_owner("successor", "next").unwrap();
    for owner in ["manager", "successor", "next"] {
        let mut r = request(&id, Some("first.txt"));
        r["ownerSessionId"] = json!(owner);
        assert_eq!(f.review.read(&r), before);
    }
    assert!(
        f.review
            .capture("manager", "worker", "turn-ended")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.review
            .capture("next", "worker", "turn-ended")
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test]
async fn generations_revocation_and_concurrent_capture_do_not_resurrect_old_data() {
    let f = Fixture::new();
    let (cwd, allocation) = f.allocate().await;
    std::fs::write(cwd.join("first.txt"), "change\n").unwrap();
    commit(&cwd);
    let (a, b) = tokio::join!(
        f.review.capture("manager", "worker", "turn-ended"),
        f.review.capture("manager", "worker", "session-ended")
    );
    let id = a.unwrap().unwrap();
    assert_eq!(Some(id.clone()), b.unwrap());
    let pending = f.review.capture("manager", "worker", "session-ended");
    tokio::pin!(pending);
    assert!(futures_util::poll!(&mut pending).is_pending());
    f.review.forget(&request(&id, None)).unwrap();
    assert!(pending.await.unwrap().is_none());
    assert!(f.persisted()["records"].as_array().unwrap().is_empty());
    f.review
        .register("manager", "worker", allocation.clone())
        .unwrap();
    let old = f.review.capture("manager", "worker", "turn-ended");
    tokio::pin!(old);
    assert!(futures_util::poll!(&mut old).is_pending());
    f.review.register("manager", "worker", allocation).unwrap();
    let (old, new) = tokio::join!(old, f.review.capture("manager", "worker", "turn-ended"));
    assert!(old.unwrap().is_none());
    assert!(new.unwrap().is_some());
}
#[tokio::test]
async fn dirty_missing_changed_branch_and_restricted_paths_are_honest_without_partial_diffs() {
    let f = Fixture::new();
    let (cwd, allocation) = f.allocate().await;
    std::fs::write(cwd.join("untracked.txt"), "uncommitted").unwrap();
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    let dirty = f.review.read(&request(&id, None));
    assert_eq!(dirty["evidence"]["availability"], "dirty");
    assert_eq!(dirty["evidence"]["files"], json!([]));
    std::fs::remove_file(cwd.join("untracked.txt")).unwrap();
    std::fs::write(cwd.join(".env"), "TOKEN=must-not-be-retained").unwrap();
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.review.read(&request(&id, None))["evidence"]["availability"],
        "unavailable"
    );
    assert!(!f.persisted().to_string().contains("must-not-be-retained"));
    git(
        &cwd,
        &[
            "reset",
            "--hard",
            allocation["baseCommit"].as_str().unwrap(),
        ],
    );
    git(&cwd, &["checkout", "-qb", "changed-branch"]);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.review.read(&request(&id, None))["evidence"]["availability"],
        "unavailable"
    );
    let mut missing = allocation;
    missing["allocatedCwd"] = json!(f.root.path().join("missing"));
    f.review
        .register("manager", "missing-worker", missing)
        .unwrap();
    let id = f
        .review
        .capture("manager", "missing-worker", "session-ended")
        .await
        .unwrap()
        .unwrap();
    let mut r = request(&id, None);
    r["workerSessionId"] = json!("missing-worker");
    assert_eq!(f.review.read(&r)["evidence"]["availability"], "unavailable");
    assert!(
        f.review
            .capture("manager", "in-place-worker", "turn-ended")
            .await
            .unwrap()
            .is_none()
    );
}
#[cfg(unix)]
#[tokio::test]
async fn literal_rename_binary_mode_and_symlink_text_are_captured_without_following_targets() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let (cwd, _) = f.allocate().await;
    git(&cwd, &["mv", "first.txt", "renamed name.txt"]);
    git(&cwd, &["rm", "delete.txt"]);
    std::fs::write(cwd.join("tab\tnewline\n.txt"), "odd name\n").unwrap();
    std::fs::write(cwd.join("binary.dat"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(cwd.join("run.sh"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(cwd.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let outside = f.root.path().join("outside-secret");
    std::fs::write(&outside, "secret-target-body").unwrap();
    std::os::unix::fs::symlink(&outside, cwd.join("link")).unwrap();
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.review.read(&request(&id, None))["evidence"]["availability"],
        "captured"
    );
    for file in [
        "renamed name.txt",
        "delete.txt",
        "tab\tnewline\n.txt",
        "binary.dat",
        "run.sh",
        "link",
    ] {
        assert!(
            f.review.read(&request(&id, Some(file)))["evidence"]["files"][0]["diff"].is_string()
        );
    }
    assert_eq!(
        f.review.read(&request(&id, Some("renamed name.txt")))["evidence"]["files"][0]["oldPath"],
        "first.txt"
    );
    assert!(!f.persisted().to_string().contains("secret-target-body"));
    assert_eq!(
        std::fs::metadata(cwd.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        std::fs::metadata(f.root.path().join("config/fleet-review.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn oversize_file_diff_and_record_retention_never_publish_partial_ranges() {
    let f = Fixture::new();
    let (cwd, allocation) = f.allocate().await;
    std::fs::write(cwd.join("large.txt"), "a".repeat(1024 * 1024 + 1)).unwrap();
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    let large = f.review.read(&request(&id, None));
    assert_eq!(large["evidence"]["availability"], "oversized");
    assert_eq!(large["evidence"]["files"], json!([]));
    let mut state = f.persisted();
    state["records"] = json!(
        (0..64)
            .map(|i| {
                let mut row = state["records"][0].clone();
                row["id"] = json!(format!("old-{i}"));
                row
            })
            .collect::<Vec<_>>()
    );
    std::fs::write(
        f.root.path().join("config/fleet-review.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    f.review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap();
    assert_eq!(f.persisted()["records"].as_array().unwrap().len(), 64);
    assert_eq!(f.review.read(&request("old-0", None))["ok"], false);
    assert_eq!(f.review.read(&request("old-1", None))["ok"], true);
    git(
        &cwd,
        &[
            "reset",
            "--hard",
            allocation["baseCommit"].as_str().unwrap(),
        ],
    );
    for n in 0..201 {
        std::fs::write(cwd.join(format!("file-{n}")), "a\n").unwrap();
    }
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    let many = f.review.read(&request(&id, None));
    assert_eq!(many["evidence"]["availability"], "oversized");
    assert_eq!(many["evidence"]["files"], json!([]));
}
#[tokio::test]
async fn adoption_after_removal_reuses_authorized_immutable_evidence() {
    let f = Fixture::new();
    let (cwd, _) = f.allocate().await;
    std::fs::write(cwd.join("first.txt"), "before removal\n").unwrap();
    commit(&cwd);
    f.review.before_remove(&cwd).await.unwrap();
    let id = f.persisted()["records"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    git(&f.repo, &["worktree", "remove", cwd.to_str().unwrap()]);
    f.review.adopt_owner("manager", "successor").unwrap();
    assert_eq!(
        f.review
            .capture("successor", "worker", "session-ended")
            .await
            .unwrap(),
        Some(id.clone())
    );
    let mut r = request(&id, Some("first.txt"));
    r["ownerSessionId"] = json!("successor");
    assert_eq!(f.review.read(&r)["evidence"]["availability"], "captured");
}
#[cfg(unix)]
#[tokio::test]
async fn allocation_base_precedes_setup_commits_and_survives_root_alias() {
    let f = Fixture::new();
    let original = git(&f.repo, &["rev-parse", "HEAD"]);
    let physical = f.root.path().join("physical");
    let alias = f.root.path().join("alias");
    std::fs::create_dir(&physical).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    f.config.save(json!({"agents":{"worktreeRoot":alias},"projects":{f.repo.to_str().unwrap():{"worktreeSetup":["printf 'setup\\n' > setup.txt; git -c core.hooksPath=/dev/null add setup.txt; git -c core.hooksPath=/dev/null -c commit.gpgsign=false commit -qm setup"]}}}),true).unwrap();
    let (cwd, allocation) = f.allocate().await;
    assert_eq!(allocation["baseCommit"], original);
    assert_ne!(git(&cwd, &["rev-parse", "HEAD"]), original);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    assert!(
        f.review.read(&request(&id, Some("setup.txt")))["evidence"]["files"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+setup")
    );
}
#[tokio::test]
async fn public_review_requests_require_actual_host_and_never_accept_raw_git_selectors() {
    use workspacer_hub::{
        Hub, Options,
        auth::{self, Scope},
        client::Client,
    };
    let f = Fixture::new();
    let (cwd, _) = f.allocate().await;
    std::fs::write(cwd.join("first.txt"), "host-only\n").unwrap();
    commit(&cwd);
    let id = f
        .review
        .capture("manager", "worker", "turn-ended")
        .await
        .unwrap()
        .unwrap();
    let tokens = f.root.path().join("tokens.json");
    let token = auth::mint(&tokens, Scope::Operator, "not-host").unwrap();
    let mut options = Options::default();
    options.config_dir = Some(f.root.path().join("config"));
    options.home_dir = Some(f.home.clone());
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let owner = Client::connect(&hub.handle()).await.unwrap();
    let scoped = Client::from_connection(
        hub.handle()
            .connect_authenticated(token.token, false)
            .await
            .unwrap(),
    );
    let params = json!({"request":request(&id,Some("first.txt"))});
    assert_eq!(
        owner
            .call("desktop.fleetReviewRead", params.clone())
            .await
            .unwrap()["ok"],
        true
    );
    assert!(
        scoped
            .call("desktop.fleetReviewRead", params.clone())
            .await
            .is_err()
    );
    assert!(
        scoped
            .call("desktop.fleetReviewForget", params)
            .await
            .is_err()
    );
    assert_eq!(owner.call("desktop.fleetReviewRead",json!({"request":{"ownerSessionId":"manager","workerSessionId":"worker","evidenceId":id,"revision":"HEAD"}})).await.unwrap()["ok"],false);
    owner.close();
    scoped.close();
    hub.shutdown().unwrap();
}
