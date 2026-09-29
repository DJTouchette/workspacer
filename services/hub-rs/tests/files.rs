use serde_json::{Value, json};
use std::path::Path;
use workspacer_hub::services::{files, paths};
#[path = "support/sweepguard.rs"]
mod sweepguard;

#[test]
fn shared_active_path_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    let mut tally = sweepguard::Tally::default();
    for case in fixture["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = std::fs::canonicalize(dir.path()).unwrap();
        let root = sandbox.join("root");
        let outside = sandbox.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let expand = |raw: &str| {
            let expanded = raw
                .replace("${SANDBOX}", sandbox.to_str().unwrap())
                .replace("${ROOT}", root.to_str().unwrap())
                .replace("${OUTSIDE}", outside.to_str().unwrap());
            #[cfg(windows)]
            {
                expanded.replace('/', "\\")
            }
            #[cfg(not(windows))]
            {
                expanded
            }
        };
        if let Some(dirs) = case["tree"]["dirs"].as_array() {
            for sub in dirs {
                std::fs::create_dir_all(sandbox.join(sub.as_str().unwrap())).unwrap();
            }
        }
        if let Some(links) = case["tree"]["symlinks"].as_object() {
            for (name, target) in links {
                let link = sandbox.join(name);
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(sandbox.join(target.as_str().unwrap()), link).unwrap();
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(sandbox.join(target.as_str().unwrap()), link).expect("Windows contract CI must enable Developer Mode rather than skip symlink cases");
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
        tally.ran(case["expect"].as_str().unwrap());
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
    tally
        .require_corpus("active path containment", 8, 3, 5)
        .unwrap();
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
    for name in ["z.txt", "a.txt", "ignored.log", "é.log"] {
        std::fs::write(dir.path().join(name), "test").unwrap();
    }
    #[cfg(unix)]
    std::fs::write(dir.path().join("line\nbreak.log"), "test").unwrap();
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
        std::fs::canonicalize(&store).unwrap().join("new.yaml")
    );
}

#[test]
#[cfg(any(unix, windows))]
fn canonical_walk_accepts_exact_fixture_link_budget_and_refuses_one_more() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    let limit = fixture["maxLinkHops"].as_u64().unwrap() as usize;
    assert_eq!(limit, 40, "review any change to the shared link budget");
    let directory = tempfile::tempdir().unwrap();
    // Resolve ambient /var or runner junction aliases first: this test counts
    // exactly the links it creates, not a platform's temporary-root spelling.
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let target = root.join("target");
    std::fs::create_dir(&target).unwrap();
    assert!(paths::contained(&target.join("child"), &target));
    assert!(!paths::contained(
        &root.join("target-sibling").join("child"),
        &target
    ));
    let ordinary_file = root.join("ordinary-file");
    std::fs::write(&ordinary_file, b"fixture").unwrap();
    assert!(paths::canonicalize(&ordinary_file.join("child")).is_err());
    for index in (0..=limit).rev() {
        let link = root.join(format!("link-{index}"));
        let next = if index == limit {
            target.clone()
        } else {
            root.join(format!("link-{}", index + 1))
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(&next, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&next, &link).expect(
            "Windows contract CI must enable Developer Mode rather than skip the hop boundary",
        );
    }
    assert_eq!(paths::canonicalize(&root.join("link-1")).unwrap(), target);
    let error = paths::canonicalize(&root.join("link-0")).unwrap_err();
    assert!(
        error.to_string().contains("too many symbolic links"),
        "{error}"
    );
}
