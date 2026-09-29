use serde_json::{Value, json};
use std::path::Path;
use workspacer_hub::services::{files, paths};

#[test]
fn shared_active_path_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        #[cfg(windows)]
        if case["needsSymlinks"] == true {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let sandbox = std::fs::canonicalize(dir.path()).unwrap();
        let root = sandbox.join("root");
        let outside = sandbox.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let expand = |raw: &str| {
            raw.replace("${SANDBOX}", sandbox.to_str().unwrap())
                .replace("${ROOT}", root.to_str().unwrap())
                .replace("${OUTSIDE}", outside.to_str().unwrap())
        };
        if let Some(dirs) = case["tree"]["dirs"].as_array() {
            for sub in dirs {
                std::fs::create_dir_all(sandbox.join(sub.as_str().unwrap())).unwrap();
            }
        }
        #[cfg(unix)]
        if let Some(links) = case["tree"]["symlinks"].as_object() {
            for (name, target) in links {
                let link = sandbox.join(name);
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                std::os::unix::fs::symlink(sandbox.join(target.as_str().unwrap()), link).unwrap();
            }
        }
        let target = expand(case["target"].as_str().unwrap());
        let result = paths::canonicalize(Path::new(&target));
        let result = result.and_then(|canonical| {
            if case["group"] == "selected-object"
                && !case["roots"].as_array().unwrap().iter().any(|root| {
                    paths::canonicalize(Path::new(&expand(root.as_str().unwrap())))
                        .is_ok_and(|root| paths::contained(&canonical, &root))
                })
            {
                anyhow::bail!("outside selected object");
            }
            Ok(canonical)
        });
        if case["expect"] == "deny" {
            assert!(result.is_err(), "{}", case["name"]);
        } else {
            assert_eq!(
                result.unwrap(),
                Path::new(&expand(case["resolvesTo"].as_str().unwrap())),
                "{}",
                case["name"]
            );
        }
    }
}

#[test]
fn file_roundtrip_is_lossless_and_binary_and_relative_paths_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/file.txt");
    files::call(
        "fs.write",
        json!({"path":path,"contents":"hello\né\n"}),
        dir.path(),
    )
    .unwrap();
    let result = files::call("fs.read", json!({"path":path}), dir.path()).unwrap();
    assert_eq!(result["contents"], "hello\né\n");
    assert_eq!(result["size"], 9);
    std::fs::write(&path, b"binary\0bytes").unwrap();
    assert!(
        files::call("fs.read", json!({"path":path}), dir.path())
            .unwrap_err()
            .to_string()
            .contains("binary")
    );
    for path in ["relative", "~/file", ""] {
        assert!(files::call("fs.read", json!({"path":path}), dir.path()).is_err());
    }
}

#[test]
fn file_tree_uses_git_ignore_rules_and_bytewise_directory_first_order() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
    for name in ["z.txt", "a.txt", "ignored.log", "line\nbreak.log"] {
        std::fs::write(dir.path().join(name), "test").unwrap();
    }
    std::fs::create_dir(dir.path().join("B-dir")).unwrap();
    let result = files::call("fs.listEntries", json!({"path":dir.path()}), dir.path()).unwrap();
    let names: Vec<_> = result["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["B-dir", ".gitignore", "a.txt", "z.txt"]);
}

#[test]
#[cfg(unix)]
fn selected_store_entry_cannot_redirect_through_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("sessions");
    std::fs::create_dir(&store).unwrap();
    let other = dir.path().join("outside.yaml");
    std::fs::write(&other, "private").unwrap();
    std::os::unix::fs::symlink(&other, store.join("escape.yaml")).unwrap();
    assert!(paths::selected_path(&store, "escape.yaml").is_err());
    for name in ["..", "../outside.yaml", "sub/file.yaml", ""] {
        assert!(paths::selected_path(&store, name).is_err());
    }
    assert_eq!(
        paths::selected_path(&store, "new.yaml").unwrap(),
        store.join("new.yaml")
    );
}
