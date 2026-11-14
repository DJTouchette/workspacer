//! Fixture tests: a disposable repository with real linked worktrees. Nothing
//! here reads the user's worktree root or talks to a real daemon.
use super::*;
use serde_json::json;
use std::os::unix::fs::{MetadataExt, symlink};
use std::process::Command;

fn ready() -> CleanupState {
    CleanupState {
        maintenance_supported: true,
        sessions: vec![],
    }
}
fn with_sessions(sessions: Value) -> CleanupState {
    CleanupState {
        maintenance_supported: true,
        sessions: sessions.as_array().unwrap().clone(),
    }
}
fn write(file: impl AsRef<Path>, body: &str) {
    let file = file.as_ref();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}
fn generate(file: impl AsRef<Path>) {
    write(file, "generated output");
}
fn path(value: &Path) -> &str {
    value.to_str().unwrap()
}
fn reasons(report: &CleanupReport) -> String {
    report
        .artifacts
        .iter()
        .filter_map(|item| item.reason.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

struct Fixture {
    _temp: tempfile::TempDir,
    dir: PathBuf,
    repo: PathBuf,
    root: PathBuf,
    cwd: PathBuf,
    git_dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("artifact-cleanup-")
            .tempdir()
            .unwrap();
        let dir = std::fs::canonicalize(temp.path()).unwrap();
        let (repo, root) = (dir.join("source"), dir.join("worktrees"));
        let cwd = root.join("source").join("worker");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(dir.join("empty-config"), "").unwrap();
        let mut fixture = Self {
            _temp: temp,
            dir,
            repo,
            root,
            cwd,
            git_dir: PathBuf::new(),
        };
        let repo = fixture.repo.clone();
        fixture.git(&repo, &["init", "-q", "-b", "main"]);
        fixture.git(&repo, &["config", "user.email", "test@example.com"]);
        fixture.git(&repo, &["config", "user.name", "test"]);
        write(
            repo.join(".gitignore"),
            "node_modules/\nbin/\nobj/\ntarget/\ndist/\nprivate-cache/\n",
        );
        write(repo.join("package.json"), "{}");
        write(repo.join("App.csproj"), "<Project />");
        write(
            repo.join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.1.0\"",
        );
        write(repo.join("source.ts"), "original source");
        fixture.git(&repo, &["add", "."]);
        fixture.git(&repo, &["commit", "-qm", "fixture"]);
        std::fs::create_dir_all(fixture.cwd.parent().unwrap()).unwrap();
        let cwd = fixture.cwd.clone();
        fixture.git(
            &repo,
            &["worktree", "add", "-q", "-b", "wks/worker", path(&cwd)],
        );
        fixture.git_dir = linked_worktree(&cwd).unwrap().unwrap().1;
        fixture.record_allocation(chrono::Utc::now());
        fixture
    }
    /// The fixture's own Git commands ignore the machine's configuration.
    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.dir.join("empty-config"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
    fn marker(&self) -> PathBuf {
        self.git_dir.join(WORKTREE_ALLOCATION)
    }
    /// What worktree allocation writes beside the linked worktree's Git state.
    fn record_allocation(&self, created: chrono::DateTime<chrono::Utc>) {
        let metadata = std::fs::metadata(&self.cwd).unwrap();
        std::fs::write(
            self.marker(),
            json!({
                "version": 1,
                "cwd": self.cwd,
                "root": self.root,
                "dev": metadata.dev(),
                "ino": metadata.ino(),
                "createdAt": created.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .to_string(),
        )
        .unwrap();
    }
    fn lock(&self) -> PathBuf {
        self.git_dir.join(".workspacer-maintenance.lock")
    }
    async fn sweep(&self, state: CleanupState, apply: bool, min_age: Duration) -> CleanupReport {
        cleanup_worktree_artifacts(
            Options {
                root: &self.root,
                apply,
                min_age,
                stop: None,
            },
            || async { Ok(state.clone()) },
        )
        .await
        .unwrap()
    }
    async fn run(&self) -> CleanupReport {
        self.sweep(ready(), true, Duration::ZERO).await
    }
}
const HOUR: Duration = Duration::from_secs(3600);

#[tokio::test]
async fn previews_then_reclaims_ignored_generated_output_while_keeping_dirty_source_untracked_code_and_branches()
 {
    let f = Fixture::new();
    for name in ["node_modules", "bin", "obj", "target", "dist"] {
        generate(f.cwd.join(name).join("large.bin"));
    }
    write(f.cwd.join("source.ts"), "uncommitted edits");
    write(f.cwd.join("new-source.ts"), "valuable untracked code");
    write(
        f.cwd.join("private-cache").join("notes.txt"),
        "valuable ignored notes",
    );
    let preview = f.sweep(ready(), false, Duration::ZERO).await;
    assert_eq!(preview.scanned_worktrees, 1);
    assert_eq!(
        preview
            .artifacts
            .iter()
            .filter(|item| item.action == Action::Planned)
            .count(),
        5
    );
    assert_eq!(preview.removed_bytes, 0);
    assert!(preview.reclaimable_bytes > 0);
    assert!(f.cwd.join("node_modules").exists());
    let result = f.run().await;
    assert_eq!(
        result
            .artifacts
            .iter()
            .filter(|item| item.action == Action::Removed)
            .count(),
        5
    );
    assert!(result.removed_bytes > 0);
    for name in ["node_modules", "bin", "obj", "target", "dist"] {
        assert!(!f.cwd.join(name).exists(), "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(f.cwd.join("source.ts")).unwrap(),
        "uncommitted edits"
    );
    assert!(f.cwd.join("new-source.ts").exists());
    assert!(f.cwd.join("private-cache").join("notes.txt").exists());
    assert_eq!(f.git(&f.cwd, &["branch", "--show-current"]), "wks/worker");
    assert!(f.git(&f.repo, &["worktree", "list"]).contains(path(&f.cwd)));
    assert!(!f.lock().exists());
}

#[tokio::test]
async fn the_report_keeps_the_cleanup_command_json_shape() {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    write(f.cwd.join("build").join("not-ignored.txt"), "kept");
    let report = serde_json::to_value(f.sweep(ready(), false, Duration::ZERO).await).unwrap();
    let mut keys: Vec<&str> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "apply",
            "artifacts",
            "errors",
            "reclaimableBytes",
            "removedBytes",
            "root",
            "scannedWorktrees",
            "skipped"
        ]
    );
    assert_eq!(report["root"], json!(f.root));
    assert_eq!(report["apply"], false);
    let planned = &report["artifacts"][0];
    assert_eq!(planned["worktree"], json!(f.cwd));
    assert_eq!(planned["path"], json!(f.cwd.join("bin")));
    assert_eq!(planned["action"], "planned", "{report}");
    assert!(planned["bytes"].is_u64());
    assert!(planned.get("reason").is_none(), "{report}");
    assert_eq!(report["artifacts"][1]["action"], "skipped");
    assert_eq!(report["artifacts"][1]["reason"], "Not Git-ignored");
}

#[tokio::test]
async fn protects_tracked_files_nonignored_directories_and_nested_repositories() {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("important.cs"));
    f.git(&f.cwd, &["add", "-f", "bin/important.cs"]);
    generate(f.cwd.join("build").join("not-ignored.txt"));
    generate(
        f.cwd
            .join("node_modules")
            .join("local-package")
            .join(".git")
            .join("config"),
    );
    let result = f.run().await;
    assert_eq!(result.removed_bytes, 0);
    assert_eq!(result.artifacts.len(), 3);
    assert!(
        result
            .artifacts
            .iter()
            .all(|item| item.action == Action::Skipped)
    );
    assert!(f.cwd.join("bin").join("important.cs").exists());
    assert!(f.cwd.join("build").join("not-ignored.txt").exists());
    assert!(f.cwd.join("node_modules").join("local-package").exists());
    assert!(reasons(&result).contains("tracked files"));
    assert!(reasons(&result).contains("Not Git-ignored"));
    assert!(reasons(&result).contains("nested Git"));
}

#[tokio::test]
async fn never_follows_dependency_or_nested_artifact_symlinks() {
    let f = Fixture::new();
    write(
        f.repo.join("node_modules").join("shared.txt"),
        "keep shared deps",
    );
    symlink(f.repo.join("node_modules"), f.cwd.join("node_modules")).unwrap();
    generate(f.cwd.join("bin").join("output.dll"));
    symlink(
        f.repo.join("node_modules"),
        f.cwd.join("bin").join("linked-deps"),
    )
    .unwrap();
    f.run().await;
    assert!(!f.cwd.join("bin").exists());
    assert!(
        std::fs::symlink_metadata(f.cwd.join("node_modules"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("node_modules").join("shared.txt")).unwrap(),
        "keep shared deps"
    );
}

#[tokio::test]
async fn protects_dependencies_referenced_by_another_worktree_including_one_added_during_the_sweep()
{
    let f = Fixture::new();
    generate(f.cwd.join("node_modules").join("shared.txt"));
    let other = f.root.join("source").join("child");
    let result = cleanup_worktree_artifacts(
        Options {
            root: &f.root,
            apply: true,
            min_age: Duration::ZERO,
            stop: None,
        },
        || async {
            f.git(
                &f.repo,
                &["worktree", "add", "-q", "-b", "wks/child", path(&other)],
            );
            symlink(f.cwd.join("node_modules"), other.join("node_modules")).unwrap();
            Ok(ready())
        },
    )
    .await
    .unwrap();
    assert!(
        result.artifacts[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("Referenced")
    );
    assert!(f.cwd.join("node_modules").join("shared.txt").exists());
}

#[tokio::test]
async fn finds_nested_and_hidden_shared_dependency_links_in_worktrees_under_a_different_root() {
    let f = Fixture::new();
    generate(f.cwd.join("node_modules").join("pkg").join("index.js"));
    let other = f.dir.join("different-root").join("child");
    std::fs::create_dir(other.parent().unwrap()).unwrap();
    f.git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wks/cross-root",
            path(&other),
        ],
    );
    std::fs::create_dir(other.join("node_modules")).unwrap();
    symlink(
        f.cwd.join("node_modules").join("pkg"),
        other.join("node_modules").join("pkg"),
    )
    .unwrap();
    let nested = f
        .sweep(
            with_sessions(json!([{"mode": "input", "cwd": other}])),
            true,
            Duration::ZERO,
        )
        .await;
    assert!(reasons(&nested).contains("Referenced"));
    std::fs::remove_file(other.join("node_modules").join("pkg")).unwrap();
    std::fs::create_dir(other.join(".dependencies")).unwrap();
    symlink(
        f.cwd.join("node_modules"),
        other.join(".dependencies").join("deps"),
    )
    .unwrap();
    assert!(reasons(&f.run().await).contains("Referenced"));
    assert!(
        f.cwd
            .join("node_modules")
            .join("pkg")
            .join("index.js")
            .exists()
    );
}

#[tokio::test]
async fn honors_git_worktree_locks_and_never_cleans_a_primary_checkout_placed_under_the_managed_root()
 {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    f.git(&f.repo, &["worktree", "lock", path(&f.cwd)]);
    assert!(
        f.run().await.skipped[0]
            .reason
            .contains("worktree is locked")
    );
    assert!(f.cwd.join("bin").exists());
    f.git(&f.repo, &["worktree", "unlock", path(&f.cwd)]);
    let primary = f.root.join("source").join("primary");
    std::fs::create_dir(&primary).unwrap();
    f.git(&primary, &["init", "-q"]);
    write(primary.join("package.json"), "{}");
    generate(primary.join("node_modules").join("keep.txt"));
    // A primary whose Git directory lives elsewhere has a `gitdir:` file too.
    let separate = f.root.join("source").join("separate");
    std::fs::create_dir(&separate).unwrap();
    let elsewhere = f.dir.join("separate.git");
    f.git(
        &separate,
        &[
            "init",
            "-q",
            "-b",
            "wks/separate",
            "--separate-git-dir",
            path(&elsewhere),
        ],
    );
    write(separate.join("package.json"), "{}");
    generate(separate.join("node_modules").join("keep.txt"));
    let report = f.run().await;
    assert_eq!(report.scanned_worktrees, 1);
    assert!(primary.join("node_modules").join("keep.txt").exists());
    assert!(separate.join("node_modules").join("keep.txt").exists());
    assert!(!f.cwd.join("bin").exists());
}

#[tokio::test]
async fn does_not_clean_a_live_session_even_when_its_cwd_is_nested() {
    for mode in ["input", "responding", "unknown", "approval", "question"] {
        let f = Fixture::new();
        generate(f.cwd.join("bin").join("output.dll"));
        std::fs::create_dir(f.cwd.join("src")).unwrap();
        let report = f
            .sweep(
                with_sessions(json!([{"mode": mode, "cwd": f.cwd.join("src")}])),
                true,
                Duration::ZERO,
            )
            .await;
        assert!(report.skipped[0].reason.contains("live session"), "{mode}");
        assert!(report.artifacts.is_empty(), "{mode}");
        assert!(f.cwd.join("bin").exists(), "{mode}");
    }
}

#[tokio::test]
async fn protects_original_and_live_directories_unknown_live_cwd_recent_stops_and_recent_artifacts()
{
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    f.record_allocation(chrono::Utc::now() - chrono::Duration::hours(2));
    let now = chrono::Utc::now().to_rfc3339();
    for sessions in [
        json!([{"mode": "input", "cwd": f.repo, "liveCwd": f.cwd}]),
        json!([{"mode": "input", "cwd": f.repo, "status_line": {"workspace": {"current_dir": f.cwd}}}]),
        json!([{"mode": "input", "cwd": null}]),
        json!([{"mode": "input", "cwd": "relative/path"}]),
        json!([{"mode": "stopped", "cwd": f.cwd, "updated_at": now}]),
        json!([{"mode": "stopped", "cwd": f.cwd, "updated_at": "not a time"}]),
        json!([{"mode": "stopped", "cwd": f.cwd}]),
    ] {
        let report = f.sweep(with_sessions(sessions.clone()), true, HOUR).await;
        assert_eq!(report.skipped.len(), 1, "{sessions}");
        assert!(report.artifacts.is_empty(), "{sessions}");
    }
    // A session that stopped long ago, or lives elsewhere, protects nothing;
    // the artifact's own fresh write still does.
    let old = (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
    for state in [
        ready(),
        with_sessions(json!([
            {"mode": "stopped", "cwd": f.cwd, "updated_at": old},
            {"mode": "input", "cwd": f.repo}
        ])),
    ] {
        let recent = f.sweep(state, true, HOUR).await;
        assert!(recent.skipped.is_empty());
        assert!(
            recent.artifacts[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("recently written")
        );
    }
    assert!(f.cwd.join("bin").exists());
    // A fresh allocation is protected on its own.
    f.record_allocation(chrono::Utc::now());
    let fresh = f.sweep(ready(), true, HOUR).await;
    assert!(fresh.skipped[0].reason.contains("New worktree"));
}

#[tokio::test]
async fn requires_compatible_daemon_state_and_releases_the_lease_after_errors() {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    let unsupported = CleanupState {
        maintenance_supported: false,
        sessions: vec![],
    };
    assert!(
        f.sweep(unsupported.clone(), true, Duration::ZERO)
            .await
            .skipped[0]
            .reason
            .contains("lacks worktree-maintenance")
    );
    // A dry run may inspect without that support.
    let preview = f.sweep(unsupported, false, Duration::ZERO).await;
    assert_eq!(preview.artifacts[0].action, Action::Planned);
    let failed = cleanup_worktree_artifacts(
        Options {
            root: &f.root,
            apply: true,
            min_age: Duration::ZERO,
            stop: None,
        },
        || async {
            assert!(f.lock().exists());
            bail!("daemon unreachable")
        },
    )
    .await
    .unwrap();
    assert!(failed.skipped[0].reason.contains("daemon unreachable"));
    assert!(failed.artifacts.is_empty());
    assert!(!f.lock().exists());
    assert!(f.cwd.join("bin").exists());
}

#[tokio::test]
async fn recognizes_legacy_wks_branches_and_refuses_changed_allocation_identity_or_occupied_leases()
{
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    let original = std::fs::read_to_string(f.marker()).unwrap();
    let allocation: Value = serde_json::from_str(&original).unwrap();
    for (field, value) in [
        ("ino", json!(allocation["ino"].as_u64().unwrap() + 1)),
        ("dev", json!(allocation["dev"].as_u64().unwrap() + 1)),
        ("version", json!(2)),
        ("cwd", json!(f.repo)),
        ("root", json!(f.dir)),
        ("createdAt", json!("yesterday")),
    ] {
        let mut changed = allocation.clone();
        changed[field] = value;
        std::fs::write(f.marker(), changed.to_string()).unwrap();
        let report = f.run().await;
        assert!(
            report.skipped[0].reason.contains("identity changed"),
            "{field}"
        );
        assert_eq!(report.scanned_worktrees, 0, "{field}");
    }
    std::fs::write(f.marker(), "{").unwrap();
    assert_eq!(f.run().await.skipped.len(), 1);
    std::fs::remove_file(f.marker()).unwrap();
    {
        let _lease = WorktreeMaintenance::acquire_git_dir(&f.git_dir).unwrap();
        assert!(f.run().await.skipped[0].reason.contains("maintenance"));
        assert!(f.cwd.join("bin").exists());
    }
    // A lock nobody in this process owns is never stolen, whatever its age.
    std::fs::write(f.lock(), r#"{"pid":4294967295,"token":"dead-or-unknown"}"#).unwrap();
    assert!(f.run().await.skipped[0].reason.contains("maintenance"));
    assert!(f.cwd.join("bin").exists());
    std::fs::remove_file(f.lock()).unwrap();
    assert_eq!(f.run().await.artifacts[0].action, Action::Removed);
    // Without the record, only a wks/ branch marks a managed worktree.
    generate(f.cwd.join("bin").join("output.dll"));
    f.git(&f.cwd, &["checkout", "-q", "-b", "feature"]);
    let report = f.run().await;
    assert_eq!(report.scanned_worktrees, 0);
    assert!(report.is_quiet());
    assert!(f.cwd.join("bin").exists());
}

#[tokio::test]
async fn negotiates_daemon_protection_without_changing_the_health_payload_contract() {
    use axum::{Json, Router, routing::get};
    async fn serve(marker: Option<&'static str>, sessions: Value) -> String {
        let app = Router::new()
            .route(
                "/health",
                get(move || async move {
                    let mut response = axum::response::IntoResponse::into_response("ok");
                    if let Some(marker) = marker {
                        response
                            .headers_mut()
                            .insert("X-Workspacer-Maintenance", marker.parse().unwrap());
                    }
                    response
                }),
            )
            .route("/sessions", get(move || async move { Json(sessions) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        url
    }
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let stopped = json!([{"mode": "stopped", "cwd": "/project"}]);
    let state = load_worktree_cleanup_state(&client, &serve(Some("1"), stopped.clone()).await)
        .await
        .unwrap();
    assert!(state.maintenance_supported);
    assert_eq!(state.sessions, stopped.as_array().unwrap().clone());
    for marker in [None, Some("0"), Some("true")] {
        let url = serve(marker, stopped.clone()).await;
        let state = load_worktree_cleanup_state(&client, &url).await.unwrap();
        assert!(!state.maintenance_supported, "{marker:?}");
    }
    for invalid in [
        json!({"sessions": []}),
        json!([{"cwd": "/project"}]),
        json!([null]),
        json!([{"mode": 1}]),
    ] {
        let url = serve(Some("1"), invalid.clone()).await;
        let error = load_worktree_cleanup_state(&client, &url)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("Invalid daemon session state"),
            "{invalid}"
        );
    }
    // An unreachable daemon is an error, never an empty fleet.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert!(load_worktree_cleanup_state(&client, &closed).await.is_err());
}

#[tokio::test]
async fn a_stop_request_removes_nothing_and_releases_the_lease() {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    // Raised while the pass already holds the lease and is reading state.
    let stop = AtomicBool::new(false);
    let report = cleanup_worktree_artifacts(
        Options {
            root: &f.root,
            apply: true,
            min_age: Duration::ZERO,
            stop: Some(&stop),
        },
        || async {
            stop.store(true, Ordering::Release);
            Ok(ready())
        },
    )
    .await
    .unwrap();
    assert!(report.skipped[0].reason.contains("stopped"));
    assert_eq!(report.removed_bytes, 0);
    assert!(f.cwd.join("bin").exists());
    assert!(!f.lock().exists());
    // Already raised: the worktree is not even opened.
    let report = cleanup_worktree_artifacts(
        Options {
            root: &f.root,
            apply: true,
            min_age: Duration::ZERO,
            stop: Some(&stop),
        },
        || async { panic!("state must not be loaded after a stop") },
    )
    .await
    .unwrap();
    assert!(report.skipped[0].reason.contains("stopped"));
    assert!(f.cwd.join("bin").exists());
}

#[tokio::test]
async fn names_alone_are_not_artifacts_and_pattern_characters_in_paths_are_literal() {
    let f = Fixture::new();
    // Recognized names without the project metadata that produces them.
    generate(f.cwd.join("docs").join("target").join("page.html"));
    generate(f.cwd.join("docs").join("node_modules").join("x.js"));
    generate(f.cwd.join("docs").join("bin").join("tool"));
    // Hidden directories are not searched.
    write(f.cwd.join(".cache").join("package.json"), "{}");
    generate(f.cwd.join(".cache").join("node_modules").join("x.js"));
    // A parent directory spelled like a pathspec pattern.
    let odd = f.cwd.join("pk[g]*");
    write(odd.join("Cargo.toml"), "[package]");
    generate(odd.join("target").join("debug").join("out"));
    write(odd.join("go.mod"), "module fixture");
    generate(odd.join("bin").join("kept"));
    f.git(&f.cwd, &["add", "-f", "pk[g]*/bin/kept"]);
    let report = f.run().await;
    assert_eq!(report.artifacts.len(), 2, "{report:?}");
    assert_eq!(report.artifacts[0].path, path(&odd.join("bin")));
    assert!(reasons(&report).contains("tracked files"));
    assert_eq!(report.artifacts[1].path, path(&odd.join("target")));
    assert_eq!(report.artifacts[1].action, Action::Removed);
    assert!(odd.join("bin").join("kept").exists());
    assert!(!odd.join("target").exists());
    assert!(f.cwd.join("docs").join("target").join("page.html").exists());
    assert!(f.cwd.join("docs").join("node_modules").exists());
    assert!(f.cwd.join(".cache").join("node_modules").exists());
}

#[tokio::test]
async fn a_replaced_worktree_directory_or_symlinked_group_is_never_cleaned() {
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    // Same path, different directory: the recorded identity no longer matches.
    let moved = f.root.join("source").join("moved-aside");
    std::fs::rename(&f.cwd, &moved).unwrap();
    std::fs::create_dir(&f.cwd).unwrap();
    std::fs::copy(moved.join(".git"), f.cwd.join(".git")).unwrap();
    write(f.cwd.join("App.csproj"), "<Project />");
    generate(f.cwd.join("bin").join("output.dll"));
    let report = f.run().await;
    assert_eq!(report.removed_bytes, 0);
    assert!(report.artifacts.is_empty());
    assert!(f.cwd.join("bin").exists());
    assert!(moved.join("bin").exists());
    // A repository group or worktree reached through a symlink is not ours.
    symlink(f.root.join("source"), f.root.join("alias")).unwrap();
    symlink(&moved, f.root.join("source").join("alias-worker")).unwrap();
    let report = f.run().await;
    assert_eq!(report.removed_bytes, 0);
    assert!(moved.join("bin").exists());
    // A missing root is an empty report, not an error.
    let missing = cleanup_worktree_artifacts(
        Options {
            root: &f.dir.join("absent"),
            apply: true,
            min_age: Duration::ZERO,
            stop: None,
        },
        || async { Ok(ready()) },
    )
    .await
    .unwrap();
    assert!(missing.is_quiet());
    assert_eq!(missing.scanned_worktrees, 0);
}

#[tokio::test]
async fn an_owned_backend_cleans_through_its_own_engine_and_refuses_after_a_stop() {
    use super::super::artifact_cleanup_scheduler::{InFlight, Policy, Request, owned_pass};
    use std::sync::Arc;
    let _engine_guard = crate::backend::ENGINE_TEST_LOCK.lock().await;
    let f = Fixture::new();
    generate(f.cwd.join("bin").join("output.dll"));
    let config = f.dir.join("config");
    write(
        config.join("config.yaml"),
        &json!({"agents": {"worktreeRoot": f.root, "artifactCleanup": {"minAgeHours": 0}}})
            .to_string(),
    );
    let mut options = crate::Options::default();
    options.config_dir = Some(config);
    options.home_dir = Some(f.dir.join("home"));
    let backend = crate::backend::Backend::start(
        claudemon::daemon::ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: f.dir.join("state.db"),
        },
        claudemon::daemon::embedded::Options {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    assert!(backend.chores_started());
    let in_flight: Arc<AtomicBool> = Default::default();
    let pass = owned_pass(backend.engine_client());
    let request = |stop: bool| Request {
        policy: Policy {
            root: f.root.clone(),
            min_age: Duration::ZERO,
        },
        stop: Arc::new(AtomicBool::new(stop)),
        in_flight: InFlight::take(&in_flight).expect("no pass is running"),
    };
    // After shutdown began, the worktree is not opened and nothing is removed.
    let refused = pass(request(true)).await.unwrap();
    assert!(refused.skipped[0].reason.contains("stopped"), "{refused:?}");
    assert!(f.cwd.join("bin").exists());
    // The real engine: its health marker and its (empty) session inventory.
    let report = pass(request(false)).await.unwrap();
    assert_eq!(report.artifacts[0].action, Action::Removed, "{report:?}");
    assert!(report.apply);
    assert!(!f.cwd.join("bin").exists());
    assert!(!f.lock().exists());
    assert!(!in_flight.load(Ordering::Acquire));
    backend.shutdown().await.unwrap();
}
