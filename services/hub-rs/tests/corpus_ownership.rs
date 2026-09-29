//! Discovery guards retained from the Go contracts_test.go suite. A mention is
//! counted only inside a test source; per-block file/needle guards live in the
//! independent vocabulary suite. Neither is a claim of behavioral completeness.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "release",
    "build",
    ".git",
    ".workspacer",
    ".claude",
];
struct Source {
    path: PathBuf,
    language: &'static str,
    body: String,
    test: bool,
}
fn scan(root: &Path) -> Vec<Source> {
    fn walk(dir: &Path, files: &mut Vec<Source>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let ty = entry.file_type().unwrap();
            let path = entry.path();
            if ty.is_dir() {
                if !SKIP.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    walk(&path, files)
                };
                continue;
            }
            if !ty.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let language = match path.extension().and_then(|x| x.to_str()) {
                Some("ts" | "tsx" | "mjs" | "cjs") => "typescript",
                Some("rs") => "rust",
                Some("go") => "go",
                _ => continue,
            };
            if name.ends_with(".d.ts") {
                continue;
            }
            let body = fs::read_to_string(&path).unwrap();
            let test = match language {
                "go" => name.ends_with("_test.go"),
                "rust" => ["#[cfg(test)]", "#[test]", "#[tokio::test"]
                    .iter()
                    .any(|x| body.contains(x)),
                _ => {
                    name.contains(".test.")
                        || name.contains(".spec.")
                        || (name.starts_with("test-") && body.contains("node:test"))
                }
            };
            files.push(Source {
                path,
                language,
                body,
                test,
            });
        }
    }
    let mut files = vec![];
    walk(root, &mut files);
    files
}
#[test]
fn source_discovery_excludes_runtime_caches_and_comments_are_not_loaders() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "app/fixture.test.ts",
        "svc/fixture_test.go",
        "rust/test.rs",
        "app/implementation.ts",
        ".workspacer/cache/stale_test.go",
        ".claude/worktrees/stale.test.ts",
        "target/stale.rs",
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#[test] fixture.json").unwrap();
    }
    let sources = scan(dir.path());
    assert_eq!(sources.len(), 4);
    assert_eq!(sources.iter().filter(|s| s.test).count(), 3);
}
#[test]
fn every_active_fixture_has_test_loaders_in_distinct_languages() {
    let root = root();
    let sources = scan(&root);
    for language in ["typescript", "rust"] {
        assert!(
            sources.iter().any(|s| s.language == language),
            "source discovery lost {language}"
        );
    }
    let mut failures = vec![];
    let mut fixtures = BTreeSet::new();
    let readme = fs::read_to_string(root.join("contracts/README.md")).unwrap();
    let baselines = captured_baselines(&root);
    for entry in fs::read_dir(root.join("contracts")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if !name.ends_with(".json") {
            continue;
        }
        fixtures.insert(name.clone());
        let loaders: Vec<_> = sources
            .iter()
            .filter(|s| s.test && s.body.contains(&name))
            .collect();
        let languages: BTreeSet<_> = loaders.iter().map(|s| s.language).collect();
        if baselines.get(&name).is_none() && (loaders.len() < 2 || languages.len() < 2) {
            failures.push(format!(
                "{name}: only {:?} in {languages:?}",
                loaders
                    .iter()
                    .map(|s| s.path.strip_prefix(&root).unwrap())
                    .collect::<Vec<_>>()
            ));
        }
        if !readme.contains(&name) {
            failures.push(format!("{name}: missing owner documentation"));
        }
    }
    assert!(!fixtures.is_empty(), "fixture directory empty");
    for token in readme.split(|c: char| c.is_whitespace() || "`|(),\"'".contains(c)) {
        let token = token.trim_end_matches('.');
        if token.ends_with(".json") && !token.contains(['/', '\\']) && !fixtures.contains(token) {
            failures.push(format!("README advertises missing {token}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
fn windows_guards(workflow: &Value) -> (bool, bool) {
    let mut rust = false;
    let mut ts = false;
    for job in workflow["jobs"].as_object().unwrap().values() {
        let runner = job["runs-on"].as_str().unwrap_or("");
        let windows = runner.contains("windows")
            || (runner.contains("matrix.os")
                && job["strategy"]["matrix"]["os"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|x| x.as_str().is_some_and(|s| s.contains("windows")))
                }));
        if !windows {
            continue;
        }
        for step in job["steps"].as_array().into_iter().flatten() {
            let run = step["run"].as_str().unwrap_or("");
            let cwd = step["working-directory"]
                .as_str()
                .or(job["defaults"]["run"]["working-directory"].as_str())
                .unwrap_or("");
            rust |= run.contains("cargo test")
                && (cwd == "services/hub-rs" || run.contains("services/hub-rs/Cargo.toml"));
            ts |= run.contains("vitest")
                && (run.contains("pathConfinement") || !run.contains(".test.ts"));
        }
    }
    (rust, ts)
}
#[test]
fn windows_ci_runs_both_live_path_implementations() {
    let workflow: Value =
        serde_yaml::from_str(&fs::read_to_string(root().join(".github/workflows/ci.yml")).unwrap())
            .unwrap();
    assert_eq!(windows_guards(&workflow), (true, true));
    let mut mutant = workflow.clone();
    for job in mutant["jobs"].as_object_mut().unwrap().values_mut() {
        job["runs-on"] = Value::String("ubuntu-latest".into());
    }
    assert_eq!(
        windows_guards(&mutant),
        (false, false),
        "OS mutation did not disable checks"
    );
    let mut mutant = workflow;
    for job in mutant["jobs"].as_object_mut().unwrap().values_mut() {
        for step in job["steps"].as_array_mut().unwrap() {
            step.as_object_mut().unwrap().remove("run");
        }
    }
    assert_eq!(
        windows_guards(&mutant),
        (false, false),
        "deleted test commands still counted"
    );
}

const CAPTURED: &[&str] = &[
    "hub-bus-cases.json",
    "hub-job-cases.json",
    "hub-snapshot-cases.json",
    "routing-policy-cases.json",
    "usage-pacing-cases.json",
    "fleet-quiescence-cases.json",
];
fn hex(value: &Value, length: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == length && s.bytes().all(|c| c.is_ascii_hexdigit()))
}
fn local_path(root: &Path, relative: &str) -> PathBuf {
    let path = Path::new(relative);
    assert!(
        !path.is_absolute()
            && !path
                .components()
                .any(|p| p == std::path::Component::ParentDir),
        "invalid baseline path {relative}"
    );
    root.join(path)
}
fn verify_baseline(root: &Path, name: &str, entry: &Value) {
    assert!(
        CAPTURED.contains(&name),
        "unreviewed baseline exception {name}"
    );
    assert!(
        hex(&entry["referenceCommit"], 40),
        "missing captured Git provenance"
    );
    assert!(!entry["captureKind"].as_str().unwrap().is_empty());
    let raw = fs::read(root.join("contracts").join(name)).unwrap();
    assert_eq!(
        entry["fixtureSha256"],
        format!("{:x}", Sha256::digest(&raw)),
        "captured {name} changed; explicit reference refresh required"
    );
    let doc: Value = serde_json::from_slice(&raw).unwrap();
    let sources = entry["referenceSources"].as_object().unwrap();
    assert!(!sources.is_empty());
    for (path, digest) in sources {
        assert!(hex(digest, 64), "bad reference digest {path}");
        let path = local_path(root, path);
        if path.exists() {
            assert_eq!(
                digest,
                &Value::String(format!("{:x}", Sha256::digest(fs::read(path).unwrap()))),
                "retained reference source drifted"
            );
        }
    }
    let specs = doc["vocabulary"]["blocks"].as_object().unwrap();
    let floors = entry["caseFloors"].as_object().unwrap();
    assert_eq!(
        floors.keys().collect::<BTreeSet<_>>(),
        specs
            .keys()
            .filter(|k| k.as_str() != "_comment")
            .collect::<BTreeSet<_>>()
    );
    let mut actual = BTreeSet::new();
    for (block, minimum) in floors {
        let rows = block
            .split('.')
            .fold(&doc, |v, k| &v[k])
            .as_array()
            .unwrap();
        let minimum = minimum.as_u64().unwrap();
        assert!(
            minimum > 0 && rows.len() as u64 >= minimum,
            "{name}/{block}: case floor reduced"
        );
        for loader in specs[block]["loaders"].as_array().unwrap() {
            let loader = loader.as_str().unwrap();
            // A captured baseline cannot replace an independent active TS owner.
            assert!(
                loader.starts_with("services/hub-rs/"),
                "{name}/{block}: independent owner must remain active: {loader}"
            );
            let (file, needle) = loader.split_once("::").unwrap();
            let source = fs::read_to_string(local_path(root, file)).unwrap();
            assert!(
                !needle.is_empty() && source.contains(needle) && source.contains(name),
                "missing actual fixture replay {loader}"
            );
            assert!(
                source.contains("#[test]") || source.contains("#[tokio::test"),
                "replay is not a test"
            );
            actual.insert(loader);
        }
    }
    let recorded: BTreeSet<_> = entry["rustLoaders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(!recorded.is_empty());
    assert_eq!(actual, recorded, "captured Rust loader set drifted");
}
fn captured_baselines(root: &Path) -> Value {
    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("contracts/reference-baselines/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["version"], 1);
    let entries = manifest["fixtures"].as_object().unwrap();
    assert_eq!(
        entries.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        CAPTURED.iter().copied().collect()
    );
    for (name, entry) in entries {
        verify_baseline(root, name, entry);
    }
    manifest["fixtures"].clone()
}
#[test]
fn captured_reference_guard_rejects_mutations() {
    let root = root();
    let manifest = captured_baselines(&root);
    let name = CAPTURED[0];
    let base = manifest[name].clone();
    for (field, value) in [
        ("fixtureSha256", Value::String("0".repeat(64))),
        ("referenceSources", serde_json::json!({})),
        ("rustLoaders", serde_json::json!([])),
        ("caseFloors", serde_json::json!({})),
        ("referenceCommit", Value::Null),
    ] {
        let mut mutant = base.clone();
        mutant[field] = value;
        assert!(
            std::panic::catch_unwind(|| verify_baseline(&root, name, &mutant)).is_err(),
            "baseline mutation escaped: {field}"
        );
    }
}
