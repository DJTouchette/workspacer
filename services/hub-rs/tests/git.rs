use serde_json::json;
use workspacer_hub::services::git::{call, format_action_error, numstat, porcelain};

fn isolated(test: &str) -> bool {
    if std::env::var_os("WKS_GIT_FIXTURE_CHILD").is_some() {
        return false;
    }
    let root = tempfile::tempdir().unwrap();
    let empty = root.path().join("empty-config");
    std::fs::write(&empty, "").unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env("WKS_GIT_FIXTURE_CHILD", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_SYSTEM", &empty)
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    for key in [
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_EXTERNAL_DIFF",
        "GIT_DIFF_OPTS",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_EXEC_PATH",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_AUTHOR_DATE",
        "GIT_COMMITTER_DATE",
    ] {
        command.env_remove(key);
    }
    assert!(
        command.status().unwrap().success(),
        "isolated git test {test}"
    );
    true
}
fn fixture_git(cwd: &std::path::Path, args: &[&str]) -> String {
    let mut command = std::process::Command::new("git");
    command.current_dir(cwd);
    for pair in workspacer_hub::services::files::GIT_NO_EXEC {
        command.args(["-c", pair]);
    }
    let output = command.args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn setup() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let template = tempfile::tempdir().unwrap();
    fixture_git(
        directory.path(),
        &[
            "init",
            "--quiet",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );
    let hooks = directory.path().join(".git/fixture-hooks");
    std::fs::create_dir(&hooks).unwrap();
    let attributes = directory.path().join(".git/fixture-attributes");
    std::fs::write(&attributes, "").unwrap();
    for args in [
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "core.hooksPath", hooks.to_str().unwrap()],
        vec![
            "config",
            "core.attributesFile",
            attributes.to_str().unwrap(),
        ],
        vec!["config", "protocol.allow", "never"],
        vec!["config", "protocol.file.allow", "always"],
    ] {
        fixture_git(directory.path(), &args);
    }
    directory
}

#[test]
fn parsers_keep_rename_identity_binary_counts_and_embedded_newlines() {
    assert_eq!(
        porcelain("R  new name\0old name\0?? line\nbreak.txt\0"),
        vec![
            json!({"path":"new name","orig_path":"old name","staged":"R","unstaged":" "}),
            json!({"path":"line\nbreak.txt","staged":"?","unstaged":"?"})
        ]
    );
    assert_eq!(
        numstat("1\t2\tsrc/{old => new}/file\n-\t-\timage.png\n"),
        vec![
            json!({"path":"src/new/file","added":1,"deleted":2}),
            json!({"path":"image.png","added":null,"deleted":null})
        ]
    );
    assert!(
        format_action_error("fatal: no upstream branch", "fallback")
            .starts_with("No upstream branch is configured.")
    );
}

#[tokio::test]
async fn review_reads_and_mutations_use_the_selected_repository_and_cwd() {
    if isolated("review_reads_and_mutations_use_the_selected_repository_and_cwd") {
        return;
    }
    let directory = setup();
    let root = directory.path();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(root.join("root.txt"), "root\n").unwrap();
    std::fs::write(sub.join("child.txt"), "child\n").unwrap();
    let diff = call(
        "git.diff",
        json!({"cwd":sub,"path":"root.txt","untracked":true}),
    )
    .await
    .unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("+root"));
    call("git.stage", json!({"cwd":sub})).await.unwrap();
    let status = call("git.status", json!({"cwd":root})).await.unwrap();
    let files = status["files"].as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "root.txt" && f["staged"] == "?")
    );
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "sub/child.txt" && f["staged"] == "A")
    );
    call("git.commit", json!({"cwd":root,"message":"Fixture commit"}))
        .await
        .unwrap();
    let log = call("git.log", json!({"cwd":sub})).await.unwrap();
    assert_eq!(log["commits"][0]["subject"], "Fixture commit");
    let hash = log["commits"][0]["hash"].clone();
    assert!(
        call(
            "git.commitDiff",
            json!({"cwd":sub,"hash":hash,"path":"sub/child.txt"})
        )
        .await
        .unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+child")
    );
    assert_eq!(
        call("git.commitNumstat", json!({"cwd":sub,"hash":hash}))
            .await
            .unwrap()["files"][0]["added"],
        1
    );
    assert!(
        call(
            "git.diff",
            json!({"cwd":sub,"path":"../outside","untracked":true})
        )
        .await
        .is_err()
    );
    assert!(
        call(
            "git.commitDiff",
            json!({"cwd":root,"hash":"--output=outside"})
        )
        .await
        .is_err()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn diff_refuses_symlink_escape_and_disables_external_diff_program() {
    if isolated("diff_refuses_symlink_escape_and_disables_external_diff_program") {
        return;
    }
    let directory = setup();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
    assert!(
        call(
            "git.diff",
            json!({"cwd":directory.path(),"path":"escape/private","untracked":true})
        )
        .await
        .is_err()
    );
    std::fs::write(directory.path().join("file"), "old\n").unwrap();
    call("git.stage", json!({"cwd":directory.path()}))
        .await
        .unwrap();
    call(
        "git.commit",
        json!({"cwd":directory.path(),"message":"old"}),
    )
    .await
    .unwrap();
    assert!(
        std::process::Command::new("git")
            .current_dir(directory.path())
            .args(["config", "diff.external", "this-program-must-not-exist"])
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(directory.path().join("file"), "new\n").unwrap();
    let diff = call("git.diff", json!({"cwd":directory.path(),"path":"file"}))
        .await
        .unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("+new"));
}

#[test]
fn all_legacy_porcelain_numstat_and_action_error_vectors_are_retained() {
    assert_eq!(
        porcelain(" M src/a.ts\0?? new.txt\0"),
        vec![
            json!({"path":"src/a.ts","staged":" ","unstaged":"M"}),
            json!({"path":"new.txt","staged":"?","unstaged":"?"})
        ]
    );
    assert_eq!(
        porcelain("R  new/path.ts\0old/path.ts\0 M other.ts\0")[1]["path"],
        "other.ts"
    );
    assert_eq!(porcelain(" M \0 M a\0").len(), 1);
    assert_eq!(
        porcelain("UU both.ts\0")[0],
        json!({"path":"both.ts","staged":"U","unstaged":"U"})
    );
    assert!(porcelain("é file\0").is_empty());
    for (input, expected) in [
        ("src/a.ts", "src/a.ts"),
        ("old.ts => new.ts", "new.ts"),
        ("src/{old => new}/a.ts", "src/new/a.ts"),
        ("src/{ => sub}/a.ts", "src/sub/a.ts"),
        ("src/{old => }/a.ts", "src/a.ts"),
    ] {
        assert_eq!(
            numstat(&format!("3\t1\t{input}\n"))[0],
            json!({"path":expected,"added":3,"deleted":1})
        );
    }
    for (raw, prefix) in [
        (
            "fatal: you have unmerged paths",
            "Merge conflicts need resolution",
        ),
        ("nothing to commit", "Nothing is staged"),
        ("no upstream branch", "No upstream branch"),
        ("non-fast-forward", "Push was rejected"),
        ("fetch first", "Push was rejected"),
        ("native detail", "native detail"),
    ] {
        let message = format_action_error(raw, "fallback");
        assert!(message.starts_with(prefix));
        assert!(message.contains(raw));
    }
    assert_eq!(format_action_error("   ", "fallback"), "fallback");
}

#[tokio::test]
async fn malformed_requests_never_turn_into_mutations_and_empty_repo_is_distinct() {
    if isolated("malformed_requests_never_turn_into_mutations_and_empty_repo_is_distinct") {
        return;
    }
    let directory = setup();
    let root = directory.path();
    std::fs::write(root.join("keep.txt"), "not staged\n").unwrap();
    for (method, patch) in [
        ("git.stage", json!({"path":17})),
        ("git.stage", json!({"Path":"keep.txt"})),
        ("git.diff", json!({"ſtaged":true})),
        ("git.diff", json!({"untracKed":true})),
        ("git.unstage", json!({"path":false})),
        ("git.diff", json!({"staged":"false"})),
        ("git.diff", json!({"untracked":[]})),
        ("git.numstat", json!({"staged":0})),
        ("git.log", json!({"limit":1.5})),
        ("git.commit", json!({"message":17})),
    ] {
        let mut params = json!({"cwd":root});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(call(method, params).await.is_err(), "{method}");
    }
    assert_eq!(
        call("git.status", json!({"cwd":root})).await.unwrap()["files"][0]["staged"],
        "?"
    );
    assert_eq!(
        call("git.log", json!({"cwd":root})).await.unwrap(),
        json!({"commits":[]})
    );
    assert!(
        call(
            "git.numstat",
            json!({"cwd":root,"path":"../../outside","untracked":true})
        )
        .await
        .unwrap()["files"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let outside = tempfile::tempdir().unwrap();
    let error = call("git.status", json!({"cwd":outside.path()}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not inside a git work tree"), "{error}");
}

#[tokio::test]
async fn stage_delete_unstage_commit_and_local_push_preserve_selected_cwd() {
    if isolated("stage_delete_unstage_commit_and_local_push_preserve_selected_cwd") {
        return;
    }
    let directory = setup();
    let root = directory.path();
    let front = root.join("frontend");
    let back = root.join("backend");
    std::fs::create_dir(&front).unwrap();
    std::fs::create_dir(&back).unwrap();
    std::fs::write(front.join("tracked.ts"), "old\n").unwrap();
    std::fs::write(back.join("tracked.ts"), "other\n").unwrap();
    call("git.stage", json!({"cwd":root})).await.unwrap();
    call("git.commit", json!({"cwd":root,"message":"initial"}))
        .await
        .unwrap();
    std::fs::remove_file(front.join("tracked.ts")).unwrap();
    std::fs::write(front.join("untracked.txt"), "brand new line\n").unwrap();
    std::fs::write(back.join("tracked.ts"), "leave unstaged\n").unwrap();
    call("git.stage", json!({"cwd":front})).await.unwrap();
    let staged = fixture_git(
        root,
        &["diff", "--no-ext-diff", "--cached", "--name-status"],
    );
    assert!(
        staged.contains("D\tfrontend/tracked.ts")
            && staged.contains("A\tfrontend/untracked.txt")
            && !staged.contains("backend/"),
        "{staged}"
    );
    call(
        "git.unstage",
        json!({"cwd":front,"path":"frontend/untracked.txt"}),
    )
    .await
    .unwrap();
    assert!(
        !fixture_git(root, &["diff", "--no-ext-diff", "--cached", "--name-only"])
            .contains("untracked.txt")
    );
    call(
        "git.stage",
        json!({"cwd":front,"path":"frontend/untracked.txt"}),
    )
    .await
    .unwrap();
    call("git.commit", json!({"cwd":front,"message":"review action"}))
        .await
        .unwrap();
    let hash = fixture_git(root, &["rev-parse", "HEAD"]);
    assert!(
        call(
            "git.commitDiff",
            json!({"cwd":front,"hash":hash,"path":"frontend/untracked.txt"})
        )
        .await
        .unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+brand new line")
    );
    assert_eq!(
        call("git.commitNumstat", json!({"cwd":front,"hash":hash}))
            .await
            .unwrap()["files"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let remote = tempfile::tempdir().unwrap();
    let template = tempfile::tempdir().unwrap();
    fixture_git(
        root,
        &[
            "init",
            "--bare",
            "--quiet",
            "--template",
            template.path().to_str().unwrap(),
            remote.path().to_str().unwrap(),
        ],
    );
    fixture_git(
        root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    let branch = fixture_git(root, &["branch", "--show-current"]);
    fixture_git(
        root,
        &["config", &format!("branch.{branch}.remote"), "origin"],
    );
    fixture_git(
        root,
        &[
            "config",
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        ],
    );
    call("git.push", json!({"cwd":front})).await.unwrap();
    assert_eq!(
        fixture_git(
            remote.path(),
            &["rev-parse", &format!("refs/heads/{branch}")]
        ),
        hash
    );
}

#[tokio::test]
#[cfg(any(unix, windows))]
async fn canonical_symlink_cwd_is_used_instead_of_reopening_the_requested_spelling() {
    if isolated("canonical_symlink_cwd_is_used_instead_of_reopening_the_requested_spelling") {
        return;
    }
    let directory = setup();
    let root = directory.path();
    let cwd = root.join("frontend");
    std::fs::create_dir_all(cwd.join("sub")).unwrap();
    std::fs::create_dir(cwd.join("real")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(cwd.join("real"), cwd.join("sub/link")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(cwd.join("real"), cwd.join("sub/link"))
        .expect("Windows contract CI must provide symlink privilege");
    std::fs::write(cwd.join("tracked.ts"), "old\n").unwrap();
    call("git.stage", json!({"cwd":root})).await.unwrap();
    call("git.commit", json!({"cwd":root,"message":"initial"}))
        .await
        .unwrap();
    std::fs::write(cwd.join("tracked.ts"), "old\nnew\n").unwrap();
    std::fs::write(cwd.join("untracked.txt"), "untracked\n").unwrap();
    let requested = format!("{}/../nope/../", cwd.join("sub/link").display());
    let status = call("git.status", json!({"cwd":requested})).await.unwrap();
    assert!(status["branch"].is_string());
    assert!(
        status["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "frontend/untracked.txt" && f["staged"] == "?")
    );
    let log = call("git.log", json!({"cwd":requested,"limit":5}))
        .await
        .unwrap();
    assert_eq!(log["commits"][0]["subject"], "initial");
    assert!(log["commits"][0]["authoredAt"].as_i64().unwrap() > 0);
    let stat = call("git.numstat", json!({"cwd":requested})).await.unwrap();
    assert_eq!(
        stat["files"][0],
        json!({"path":"frontend/tracked.ts","added":1,"deleted":0})
    );
    let diff = call(
        "git.diff",
        json!({"cwd":requested,"path":"frontend/tracked.ts"}),
    )
    .await
    .unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("+new"));
}

#[test]
#[cfg(unix)]
fn file_listing_uses_the_fixed_execution_prefix_on_its_actual_git_child() {
    if isolated("file_listing_uses_the_fixed_execution_prefix_on_its_actual_git_child") {
        return;
    }
    if let Some(repo) = std::env::var_os("WKS_GIT_PREFIX_REPO") {
        let root = std::path::PathBuf::from(repo);
        let result =
            workspacer_hub::services::files::call("fs.listEntries", json!({"path":root}), &root)
                .unwrap();
        assert!(
            result["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "keep.txt")
        );
        assert!(
            !result["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "ignored.log")
        );
        let argv =
            std::fs::read_to_string(std::env::var_os("WKS_GIT_PREFIX_ARGS").unwrap()).unwrap();
        let argv: Vec<_> = argv.lines().collect();
        let expected: Vec<_> = workspacer_hub::services::files::GIT_NO_EXEC
            .iter()
            .flat_map(|pair| ["-c", *pair])
            .collect();
        assert_eq!(&argv[..expected.len()], expected);
        assert_eq!(
            &argv[expected.len()..],
            [
                "-c",
                "core.quotePath=false",
                "check-ignore",
                "-z",
                "--stdin"
            ]
        );
        return;
    }
    let directory = setup();
    let witness = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("keep.txt"), "kept").unwrap();
    std::fs::write(directory.path().join("ignored.log"), "ignored").unwrap();
    std::fs::write(directory.path().join(".gitignore"), "*.log\n").unwrap();
    let original_path = std::env::var_os("PATH").unwrap();
    let real = std::env::split_paths(&original_path)
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let wrapper = witness.path().join("git");
    // Authored fixture wrapper only records argv, then execs the real binary.
    std::fs::write(&wrapper,"#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$WKS_GIT_PREFIX_ARGS\"\nexec \"$WKS_GIT_PREFIX_REAL\" \"$@\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(witness.path().to_path_buf()).chain(std::env::split_paths(&original_path)),
    )
    .unwrap();
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "file_listing_uses_the_fixed_execution_prefix_on_its_actual_git_child",
                "--nocapture"
            ])
            .env("PATH", path)
            .env("WKS_GIT_PREFIX_REPO", directory.path())
            .env("WKS_GIT_PREFIX_ARGS", witness.path().join("args"))
            .env("WKS_GIT_PREFIX_REAL", real)
            .status()
            .unwrap()
            .success()
    );
}
