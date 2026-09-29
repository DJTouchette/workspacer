use serde_json::json;
use std::{path::Path, process::Command, sync::Arc, time::Duration};
use workspacer_hub::services::{
    config::Config,
    worktrees::{Worktrees, info, resolve_setup, run_setup, slug},
};
fn git(cwd: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn repository(path: &Path) {
    std::fs::create_dir_all(path.join("apps/deep/source")).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["config", "user.email", "fixture@example.test"]);
    git(path, &["config", "user.name", "Fixture"]);
    std::fs::write(path.join("apps/deep/source/index.txt"), "hello").unwrap();
    std::fs::write(path.join(".gitignore"), "node_modules\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-qm", "initial"]);
}
#[test]
fn naming_and_setup_script_resolution_match_legacy_contract() {
    assert_eq!(slug("..Hello, WORLD.."), "hello-world");
    assert_eq!(slug("❤️"), "agent");
    let cfg = json!({"projects":{"/repo/sub":{"worktreeSetup":["  script: install  ","script: missing","echo end"]}},"scripts":{"/repo/sub":[{"name":"install","command":"echo install"}]}});
    let setup = resolve_setup(&cfg, &[Path::new("/repo/sub"), Path::new("/repo")]);
    assert_eq!(setup[0].command.as_deref(), Some("echo install"));
    assert!(setup[1].error.as_ref().unwrap().contains("missing"));
    assert_eq!(setup[2].command.as_deref(), Some("echo end"));
}
#[tokio::test]
async fn setup_timeout_kills_the_process_group_and_skips_successors() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let (slow, skipped, settle) = ("sleep 0.2; touch escaped", "touch skipped", 300);
    #[cfg(windows)]
    let (slow, skipped, settle) = (
        "ping -n 2 127.0.0.1 >nul & type nul > escaped",
        "type nul > skipped",
        1300,
    );
    let commands = resolve_setup(
        &json!({"projects":{dir.path().to_str().unwrap():{"worktreeSetup":[slow,skipped]}}}),
        &[dir.path()],
    );
    let report = run_setup(&commands, dir.path(), dir.path(), Duration::from_millis(30))
        .await
        .unwrap();
    assert!(
        report["failed"]["error"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    assert_eq!(report["skipped"], json!([skipped]));
    assert!(!dir.path().join("skipped").exists());
    tokio::time::sleep(Duration::from_millis(settle)).await;
    assert!(!dir.path().join("escaped").exists());
}
#[tokio::test]
async fn actual_allocation_reservation_setup_dependency_links_and_conservative_cleanup() {
    use claudemon::daemon::{
        ServeConfig, WorktreeAdmission, WorktreeMaintenance,
        embedded::{Command as EngineCommand, EmbeddedDaemon, Options as EngineOptions},
    };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("repo");
    repository(&project);
    let modules = project.join("apps/deep/source/node_modules");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(modules.join("dependency.js"), "fixture").unwrap();
    let cfg = Arc::new(Config::open(dir.path().join("config.yaml")));
    let root = dir.path().join("trees");
    #[cfg(unix)]
    let (check, fail, skipped) = (
        "test -d \"$SOURCE/apps/deep/source/node_modules\"",
        "exit 7",
        "touch should-not-run",
    );
    #[cfg(windows)]
    let (check, fail, skipped) = (
        "if not exist \"$SOURCE/apps/deep/source/node_modules\" exit /B 1",
        "exit /B 7",
        "type nul > should-not-run",
    );
    cfg.save(json!({"agents":{"worktreeRoot":root},"projects":{project.to_str().unwrap():{"worktreeSetup":[check,fail,skipped]}}}),true).unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("state.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    let service = Worktrees::new(dir.path().join("home"), cfg, Some(engine.client()));
    assert_eq!(info(&project).await["gitStatus"], "repo");
    assert_eq!(
        info(&dir.path().join("missing")).await["directory"],
        "invalid"
    );
    let created = service
        .create_reserved(json!({"repoCwd":project,"name":"Review Me"}))
        .await
        .unwrap();
    assert_eq!(created.result["ok"], true);
    assert_eq!(created.result["branch"], "wks/review-me");
    assert_eq!(created.result["setup"]["ran"], json!([check]));
    assert_eq!(created.result["setup"]["failed"]["command"], fail);
    let cwd = std::path::PathBuf::from(created.result["path"].as_str().unwrap());
    assert!(
        cwd.join("apps/deep/source/node_modules/dependency.js")
            .is_file()
    );
    assert!(!cwd.join("should-not-run").exists());
    let own = std::path::PathBuf::from(git(&cwd, &["rev-parse", "--absolute-git-dir"]));
    let allocation: serde_json::Value =
        serde_json::from_slice(&std::fs::read(own.join("workspacer-allocation.json")).unwrap())
            .unwrap();
    assert_eq!(allocation["cwd"], json!(cwd));
    assert!(WorktreeMaintenance::acquire_git_dir(&own).is_err());
    assert_eq!(service.remove(&cwd).await.unwrap()["skipped"], true);
    let shared = WorktreeAdmission::acquire(cwd.to_str().unwrap()).unwrap();
    drop(created);
    assert!(WorktreeMaintenance::acquire_git_dir(&own).is_err());
    drop(shared);
    let maintenance = WorktreeMaintenance::acquire_git_dir(&own).unwrap();
    assert!(WorktreeAdmission::acquire(cwd.to_str().unwrap()).is_err());
    drop(maintenance);
    // A runtime in a nested cwd protects the entire linked checkout.
    #[cfg(unix)]
    let argv = json!(["/bin/sh", "-c", "exec sleep 120"]);
    #[cfg(windows)]
    let argv = json!(["cmd.exe", "/D", "/C", "ping -n 120 127.0.0.1 >nul"]);
    engine.client().request(EngineCommand::Request{method:"POST".into(),path:"/sessions/spawn".into(),payload:Some(json!({"session_id":"fixture-worker","cwd":cwd.join("apps/deep/source"),"argv":argv}))}).await.unwrap();
    assert_eq!(
        service.remove(&cwd).await.unwrap()["error"],
        "Worktree still has a live agent"
    );
    engine
        .client()
        .request(EngineCommand::Request {
            method: "POST".into(),
            path: "/sessions/fixture-worker/signal".into(),
            payload: Some(json!({"signal":"SIGTERM"})),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let rows = engine
                .client()
                .request(EngineCommand::Sessions)
                .await
                .unwrap();
            if rows
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["mode"] == "stopped")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    std::fs::write(cwd.join("uncommitted.txt"), "preserve").unwrap();
    assert_eq!(service.remove(&cwd).await.unwrap()["skipped"], true);
    assert!(cwd.exists());
    std::fs::remove_file(cwd.join("uncommitted.txt")).unwrap();
    assert_eq!(service.remove(&cwd).await.unwrap()["ok"], true);
    assert!(!cwd.exists());
    assert!(
        !git(
            &project,
            &["rev-parse", "--verify", "refs/heads/wks/review-me"]
        )
        .is_empty()
    );
    assert_eq!(service.remove(&project).await.unwrap()["skipped"], true);
    engine.shutdown().await.unwrap();
}

#[cfg(unix)]
#[test]
fn setup_configuration_keeps_existing_alias_keys_after_canonical_admission() {
    let dir = tempfile::tempdir().unwrap();
    let actual = dir.path().join("actual");
    let other = dir.path().join("other");
    let alias = dir.path().join("alias");
    std::fs::create_dir(&actual).unwrap();
    std::fs::create_dir(&other).unwrap();
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let config = json!({
        "projects":{alias.to_str().unwrap():{"worktreeSetup":["script:deps"]}},
        "scripts":{alias.to_str().unwrap():[{"name":"deps","command":"echo existing-project"}]}
    });
    let canonical = std::fs::canonicalize(&actual).unwrap();
    let commands = resolve_setup(&config, &[&canonical]);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].command.as_deref(),
        Some("echo existing-project")
    );
    assert!(resolve_setup(&config, &[&other]).is_empty());
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&other, &alias).unwrap();
    assert!(
        resolve_setup(&config, &[&canonical]).is_empty(),
        "a retargeted alias is not the selected project"
    );
}
