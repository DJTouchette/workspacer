//! Discovery guards retained from the Go contracts_test.go suite. A mention is
//! counted only inside a test source; per-block file/needle guards live in the
//! independent vocabulary suite. Neither is a claim of behavioral completeness.
#[path = "support/repo.rs"]
mod repo;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    repo::root()
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
        let doc: Value = serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
        let architectural = if name == HTTP_SOURCE_REGISTRY {
            // This is source/authority evidence, not a second behavior replay.
            // Always validate it, even if this ownership test mentions its name.
            verify_architectural_registry(&root, &name, &doc);
            true
        } else {
            assert_ne!(
                doc["ownership"]["kind"], "architectural-source-registry",
                "unreviewed architectural-registry exception {name}"
            );
            false
        };
        if !architectural
            && baselines.get(&name).is_none()
            && (loaders.len() < 2 || languages.len() < 2)
        {
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
// Exactly one named architectural registry: its TS test compares real Rust
// production declarations rather than replaying golden behavior in two stacks.
// This classification does not waive vocabulary, discovery or mutation guards.
const HTTP_SOURCE_REGISTRY: &str = "http-route-registry.json";
const HTTP_GUARD: &str = "apps/desktop/src/main/services/httpRouteRegistry.test.ts";
const HTTP_CHECKS: &[&str] = &[
    "http_registry_matches_actual_bindings",
    "discovers new route-bearing production modules instead of trusting the router list",
    "holds credential primitives to operator versus actual host authority",
    "rejects mutations of routes, classifiers, guards, operation closure and live confinement layers",
    "keeps the served fixture and all four caller ownership guards linked",
    "source parser ignores comments and quoted decoys but retains post-test production",
];
const HTTP_SOURCES: &[&str] = &[
    "services/hub-rs/src/server.rs",
    "services/hub-rs/src/server/web.rs",
    "services/hub-rs/src/plugins/http.rs",
    "services/hub-rs/src/mcp.rs",
    "services/hub-rs/src/mcp/legacy_sse.rs",
    "services/claudemon/src/daemon/api.rs",
    "services/claudemon/src/daemon/hook.rs",
];
fn strings(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}
fn verify_architectural_registry(root: &Path, name: &str, doc: &Value) {
    assert_eq!(
        name, HTTP_SOURCE_REGISTRY,
        "unreviewed architectural registry"
    );
    let ownership = &doc["ownership"];
    assert_eq!(ownership["kind"], "architectural-source-registry");
    assert_eq!(ownership["sourceLanguage"], "rust");
    assert_eq!(ownership["guardLanguage"], "typescript");
    let expected: BTreeSet<_> = HTTP_SOURCES.iter().copied().collect();
    assert_eq!(
        strings(&ownership["sourceFiles"]),
        expected,
        "architectural registry must keep every reviewed production router"
    );
    assert_eq!(
        doc["routerOwners"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        expected,
        "declared source evidence and actual router ownership drifted"
    );
    assert_eq!(
        strings(&ownership["sourceRoots"]),
        BTreeSet::from(["services/hub-rs/src", "services/claudemon/src/daemon"])
    );
    for file in expected {
        let path = local_path(root, file);
        assert_eq!(path.extension().and_then(|v| v.to_str()), Some("rs"));
        let source = fs::read_to_string(path).unwrap();
        assert!(
            source.contains("Router")
                && (source.contains(".route(") || source.contains(".nest_service(")),
            "missing real Rust route declaration source {file}"
        );
    }
    assert_eq!(
        ownership["guard"],
        format!("{HTTP_GUARD}::http_registry_matches_actual_bindings")
    );
    assert_eq!(
        strings(&ownership["requiredChecks"]),
        HTTP_CHECKS.iter().copied().collect(),
        "architectural registry lost a required forcing function"
    );
    let guard = fs::read_to_string(local_path(root, HTTP_GUARD)).unwrap();
    assert!(
        guard.contains(name) && guard.contains("from 'vitest'") && !guard.contains("it.skip("),
        "guard must be an active Vitest source reader, not a documentation reference"
    );
    for check in HTTP_CHECKS {
        assert!(
            guard.contains(&format!("'{check}'")),
            "missing runnable guard {check}"
        );
    }
    for mechanism in [
        "check(registry, source)",
        "discover(files, registry.routerOwners)",
        "primitiveErrors(source)",
        "rustHttpSource",
    ] {
        assert!(
            guard.contains(mechanism),
            "guard lost actual-source mechanism {mechanism}"
        );
    }
    assert_eq!(
        ownership["runner"],
        "npm --prefix apps/desktop run test:main -- src/main/services/httpRouteRegistry.test.ts"
    );
    let package: Value =
        serde_json::from_slice(&fs::read(root.join("apps/desktop/package.json")).unwrap()).unwrap();
    assert_eq!(
        package["scripts"]["test:main"], "vitest run",
        "recorded runner is no longer valid"
    );
    let config = fs::read_to_string(root.join("apps/desktop/vitest.config.ts")).unwrap();
    assert!(
        config.contains("src/main/**/*.test.ts") && !config.contains("exclude:"),
        "normal test runner no longer includes the architectural guard"
    );
    let block = &doc["vocabulary"]["blocks"]["routes"];
    assert!(
        strings(&block["loaders"])
            .contains(format!("{HTTP_GUARD}::http_registry_matches_actual_bindings").as_str()),
        "architectural classification must retain the concrete vocabulary loader"
    );
    assert!(
        doc["routes"].as_array().unwrap().len() >= 70,
        "route population collapsed"
    );
}
#[test]
fn architectural_source_registry_requires_real_sources_and_runnable_forcing_functions() {
    let root = root();
    let doc: Value = serde_json::from_slice(
        &fs::read(root.join("contracts").join(HTTP_SOURCE_REGISTRY)).unwrap(),
    )
    .unwrap();
    verify_architectural_registry(&root, HTTP_SOURCE_REGISTRY, &doc);
    for (field, value) in [
        ("kind", Value::Null),
        ("sourceLanguage", Value::String("typescript".into())),
        ("sourceFiles", serde_json::json!([])),
        ("sourceRoots", serde_json::json!([])),
        (
            "guard",
            Value::String("apps/desktop/src/main/services/placeholder.test.ts::reads_json".into()),
        ),
        (
            "requiredChecks",
            serde_json::json!(["http_registry_matches_actual_bindings"]),
        ),
        ("runner", Value::String("echo skipped".into())),
    ] {
        let mut mutant = doc.clone();
        mutant["ownership"][field] = value;
        assert!(
            std::panic::catch_unwind(|| verify_architectural_registry(
                &root,
                HTTP_SOURCE_REGISTRY,
                &mutant
            ))
            .is_err(),
            "architectural ownership mutation escaped: {field}"
        );
    }
    let mut mutant = doc.clone();
    mutant["routerOwners"]
        .as_object_mut()
        .unwrap()
        .remove(HTTP_SOURCES[0]);
    assert!(
        std::panic::catch_unwind(|| verify_architectural_registry(
            &root,
            HTTP_SOURCE_REGISTRY,
            &mutant
        ))
        .is_err()
    );
    let mut mutant = doc.clone();
    mutant["vocabulary"]["blocks"]["routes"]["loaders"] = serde_json::json!([]);
    assert!(
        std::panic::catch_unwind(|| verify_architectural_registry(
            &root,
            HTTP_SOURCE_REGISTRY,
            &mutant
        ))
        .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| verify_architectural_registry(&root, "unreviewed.json", &doc))
            .is_err()
    );
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
        &repo::read(
            root,
            Path::new("contracts/reference-baselines/manifest.json"),
        )
        .unwrap(),
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

#[test]
fn captured_hash_inputs_keep_exact_bytes_under_windows_checkout_filters() {
    use std::process::Command;
    let root = root();
    let fixture = "contracts/fleet-quiescence-cases.json";
    let source = "services/hub/internal/quiescence/quiescence.go";
    for path in [fixture, source] {
        if !root.join(path).exists() {
            continue;
        } // Retired Go sources may be removed.
        let attrs = Command::new("git")
            .current_dir(&root)
            .args(["check-attr", "text", "eol", "--", path])
            .output()
            .unwrap();
        assert!(attrs.status.success());
        let attrs = String::from_utf8(attrs.stdout).unwrap();
        assert!(
            attrs.contains(": text: set") && attrs.contains(": eol: lf"),
            "{path}: {attrs}"
        );
        let filtered = Command::new("git")
            .current_dir(&root)
            .args([
                "-c",
                "core.autocrlf=true",
                "cat-file",
                "--filters",
                &format!("HEAD:{path}"),
            ])
            .output()
            .unwrap();
        assert!(
            filtered.status.success(),
            "{}",
            String::from_utf8_lossy(&filtered.stderr)
        );
        // This is the actual checkout conversion, not a test-local LF normalizer.
        assert!(
            !filtered.stdout.windows(2).any(|bytes| bytes == b"\r\n"),
            "CRLF conversion changed {path}"
        );
        let blob = Command::new("git")
            .current_dir(&root)
            .args(["cat-file", "blob", &format!("HEAD:{path}")])
            .output()
            .unwrap();
        assert!(blob.status.success());
        assert_eq!(
            filtered.stdout, blob.stdout,
            "checkout conversion changed {path} bytes"
        );
    }
}
