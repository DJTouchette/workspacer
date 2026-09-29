use serde_json::json;
use workspacer_hub::services::git::{call, format_action_error, numstat, porcelain};

fn setup() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(directory.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
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
