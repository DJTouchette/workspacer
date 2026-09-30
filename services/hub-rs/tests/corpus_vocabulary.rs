//! Independent Rust twin of contractsVocabulary.test.ts. Runtime reads deliberately
//! discover new/deleted corpus files on every cargo test invocation.
#[path = "support/repo.rs"]
mod repo;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const CHECKS: &[&str] = &[
    "blocks-declared",
    "blocks-exist",
    "required-fields",
    "verdict-vocabulary",
    "verdict-reason-required",
    "verdict-reason-declared",
    "verdict-reason-forbidden",
    "reason-vocabulary-used",
    "unique-case-names",
    "token-references",
    "unknown-fields",
    "optional-used",
    "block-loaders",
];
const EXEMPT: &[&str] = &[
    "session-schema.json",
    "config-lock.json",
    "job-preset-power-down.json",
    "backend-capabilities.json",
];
fn root() -> PathBuf {
    repo::root()
}
fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().expect("string array"))
                .collect()
        })
        .unwrap_or_default()
}
fn object(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value.as_object().into_iter().flatten()
}
fn present(value: Option<&Value>) -> bool {
    value.is_some_and(|v| !v.is_null() && v != "")
}
fn fixtures() -> BTreeMap<String, Value> {
    fs::read_dir(root().join("contracts"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| {
            (
                e.file_name().into_string().unwrap(),
                serde_json::from_slice(
                    &repo::read(&root(), &Path::new("contracts").join(e.file_name())).unwrap(),
                )
                .unwrap(),
            )
        })
        .collect()
}
fn blocks<'a>(v: &'a Value, at: &str, out: &mut BTreeMap<String, &'a Vec<Value>>) {
    if at == "vocabulary" {
        return;
    }
    match v {
        Value::Array(rows) if !rows.is_empty() && rows.iter().all(Value::is_object) => {
            out.insert(at.into(), rows);
        }
        Value::Object(map) => {
            for (key, child) in map {
                blocks(
                    child,
                    &if at.is_empty() {
                        key.clone()
                    } else {
                        format!("{at}.{key}")
                    },
                    out,
                );
            }
        }
        _ => (),
    }
}
fn visit_strings(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => {
            for v in a {
                visit_strings(v, f)
            }
        }
        Value::Object(m) => {
            for (k, v) in m {
                f(k);
                visit_strings(v, f);
            }
        }
        _ => (),
    }
}
fn validate(doc: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    let mut add = |id: &str, detail: String| errors.push(format!("[{id}] {detail}"));
    let specs: BTreeMap<_, _> = object(&doc["vocabulary"]["blocks"])
        .filter(|(k, _)| k.as_str() != "_comment")
        .collect();
    if specs.is_empty() {
        add(
            "blocks-declared",
            "missing or empty vocabulary.blocks".into(),
        );
        return errors;
    } // [blocks-declared]
    let mut found = BTreeMap::new();
    blocks(doc, "", &mut found);
    for name in found.keys() {
        if !specs.contains_key(name) {
            add("blocks-declared", name.clone());
        }
    }
    for name in specs.keys() {
        if !found.contains_key(name.as_str()) {
            add("blocks-exist", name.to_string());
        }
    } // [blocks-exist]
    for (name, rows) in found {
        let Some(spec) = specs.get(&name) else {
            continue;
        };
        let required = strings(&spec["required"]);
        let optional = strings(&spec["optional"]);
        let allowed: BTreeSet<_> = required.iter().chain(&optional).copied().collect();
        let mut used = BTreeSet::new();
        let mut nested_used: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut names = BTreeSet::new();
        for row in rows {
            if let Some(n) = row["name"].as_str().filter(|n| !n.is_empty()) {
                if !names.insert(n) {
                    add("unique-case-names", format!("{name}: {n}"));
                }
            } // [unique-case-names]
            for key in &required {
                if row.get(*key).is_none() {
                    add("required-fields", format!("{name}: {key}"));
                }
            } // [required-fields]
            for (key, value) in object(row) {
                if !allowed.contains(key.as_str()) {
                    add("unknown-fields", format!("{name}: {key}"));
                    continue;
                } // [unknown-fields]
                used.insert(key.as_str());
                if spec["nested"].get(key).is_some() {
                    let keys = strings(&spec["nested"][key]);
                    for (child, _) in object(value) {
                        if !keys.contains(&child.as_str()) {
                            add("unknown-fields", format!("{name}: {key}.{child}"));
                        } else {
                            nested_used.entry(key).or_default().insert(child);
                        }
                    }
                }
            }
            if let Some(field) = spec["verdictField"].as_str().filter(|s| !s.is_empty()) {
                let verdict = row[field].as_str().unwrap_or("");
                if let Some(def) = spec["verdicts"].get(verdict) {
                    for key in strings(&def["requires"]) {
                        if !present(row.get(key)) {
                            add("verdict-reason-required", format!("{name}: {key}"));
                            continue;
                        } // [verdict-reason-required]
                        if let Some(reason) = def["reasons"].as_str().filter(|s| !s.is_empty()) {
                            let value = row[key]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| row[key].to_string());
                            if doc["vocabulary"][reason].get(&value).is_none() {
                                add("verdict-reason-declared", format!("{name}: {value}"));
                            } // [verdict-reason-declared]
                        }
                    }
                    for key in strings(&def["forbids"]) {
                        if present(row.get(key)) {
                            add("verdict-reason-forbidden", format!("{name}: {key}"));
                        }
                    } // [verdict-reason-forbidden]
                } else {
                    add("verdict-vocabulary", format!("{name}: {verdict}"));
                } // [verdict-vocabulary]
            }
        }
        for key in optional {
            if !used.contains(key) {
                add("optional-used", format!("{name}: {key}"));
            }
        } // [optional-used]
        for (key, keys) in object(&spec["nested"]) {
            for child in strings(keys) {
                if !nested_used
                    .get(key.as_str())
                    .is_some_and(|used| used.contains(child))
                {
                    add("optional-used", format!("{name}: {key}.{child}"));
                }
            }
        }
        let loaders = strings(&spec["loaders"]);
        if loaders.is_empty() || loaders.iter().any(|l| !l.contains("::")) {
            add("block-loaders", name.clone());
        } // [block-loaders]
        for (verdict, def) in object(&spec["verdicts"]) {
            if let Some(reason) = def["reasons"].as_str().filter(|s| !s.is_empty()) {
                let field = spec["verdictField"].as_str().unwrap_or("");
                for (reason, _) in object(&doc["vocabulary"][reason]) {
                    if !rows.iter().filter(|r| r[field] == *verdict).any(|r| {
                        strings(&def["requires"])
                            .iter()
                            .any(|key| r[*key] == *reason)
                    }) {
                        add("reason-vocabulary-used", format!("{name}: {reason}")); // [reason-vocabulary-used]
                    }
                }
            }
        }
    }
    visit_strings(doc, &mut |s| {
        let mut rest = s;
        while let Some(start) = rest.find("${") {
            rest = &rest[start + 2..];
            let Some(end) = rest.find('}') else {
                add("token-references", format!("unterminated: {s}"));
                break;
            }; // [token-references]
            if doc["vocabulary"]["tokens"].get(&rest[..end]).is_none() {
                add("token-references", rest[..end].into());
            }
            rest = &rest[end + 1..];
        }
    });
    errors.sort();
    errors
}
#[test]
fn every_corpus_block_is_declared_and_closed() {
    let all = fixtures();
    let mut checked = 0;
    for (name, doc) in all {
        if EXEMPT.contains(&name.as_str()) {
            assert!(
                doc.get("vocabulary").is_none(),
                "remove {name} from explicit exemption"
            );
            continue;
        }
        checked += 1;
        assert_eq!(validate(&doc), Vec::<String>::new(), "{name}");
    }
    assert!(checked >= 10, "fixture discovery or corpus was reduced");
}
#[test]
fn vocabulary_mutation_battery() {
    let base = fixtures().remove("path-containment-cases.json").unwrap();
    assert!(validate(&base).is_empty());
    let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        (
            "verdict-reason-required",
            Box::new(|d| {
                let r = d["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["expect"] == "deny")
                    .unwrap();
                r.as_object_mut().unwrap().remove("deniedBy");
            }),
        ),
        (
            "verdict-reason-required",
            Box::new(|d| {
                let r = d["sessionFilenames"]["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["expect"] == "refuse")
                    .unwrap();
                r.as_object_mut().unwrap().remove("refusedBy");
            }),
        ),
        (
            "verdict-reason-declared",
            Box::new(|d| {
                d["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["expect"] == "deny")
                    .unwrap()["deniedBy"] = json!("outside-rooots")
            }),
        ),
        (
            "verdict-vocabulary",
            Box::new(|d| d["cases"][0]["expect"] = json!("maybe")),
        ),
        (
            "verdict-reason-forbidden",
            Box::new(|d| {
                d["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["expect"] == "allow")
                    .unwrap()["deniedBy"] = json!("secret")
            }),
        ),
        (
            "required-fields",
            Box::new(|d| {
                d["spawnCwds"]["cases"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("why");
            }),
        ),
        (
            "blocks-declared",
            Box::new(|d| d["newIdeas"] = json!([{"name":"x"}])),
        ),
        (
            "blocks-exist",
            Box::new(|d| {
                d["casesRenamed"] = d["cases"].clone();
                d.as_object_mut().unwrap().remove("cases");
            }),
        ),
        (
            "reason-vocabulary-used",
            Box::new(|d| {
                for r in d["sessionFilenames"]["cases"].as_array_mut().unwrap() {
                    if r["refusedBy"] == "escapes-sessions-dir" {
                        r["refusedBy"] = json!("not-a-basename");
                    }
                }
            }),
        ),
        (
            "unique-case-names",
            Box::new(|d| d["cases"][1]["name"] = d["cases"][0]["name"].clone()),
        ),
        (
            "token-references",
            Box::new(|d| d["cases"][0]["target"] = json!("${ROOOT}/x")),
        ),
        (
            "token-references",
            Box::new(|d| d["cases"][0]["target"] = json!("${ROOT/x")),
        ),
        (
            "unknown-fields",
            Box::new(|d| {
                let r = d["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r.get("needsSymlinks").is_some())
                    .unwrap();
                r["needsSymLinks"] = r.as_object_mut().unwrap().remove("needsSymlinks").unwrap();
            }),
        ),
        (
            "unknown-fields",
            Box::new(|d| {
                let r = d["cases"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["tree"].get("symlinks").is_some())
                    .unwrap();
                r["tree"]["symLinks"] = r["tree"]
                    .as_object_mut()
                    .unwrap()
                    .remove("symlinks")
                    .unwrap();
            }),
        ),
        (
            "optional-used",
            Box::new(|d| {
                d["vocabulary"]["blocks"]["cases"]["optional"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("configDirVla"))
            }),
        ),
        (
            "block-loaders",
            Box::new(|d| {
                d["vocabulary"]["blocks"]["sessionFilenames.cases"]
                    .as_object_mut()
                    .unwrap()
                    .remove("loaders");
            }),
        ),
    ];
    let mut exercised = BTreeSet::new();
    for (id, mutate) in cases {
        let mut doc = base.clone();
        mutate(&mut doc);
        let errors = validate(&doc);
        assert!(
            errors.iter().any(|e| e.contains(&format!("[{id}]"))),
            "{id}: {errors:?}"
        );
        exercised.insert(id);
    }
    assert_eq!(exercised, CHECKS.iter().copied().collect());
}
#[test]
fn independent_typescript_twin_and_active_path_loaders_exist() {
    let source = fs::read_to_string(
        root().join("apps/desktop/src/main/services/contractsVocabulary.test.ts"),
    )
    .unwrap();
    for id in CHECKS {
        assert!(
            source.contains(&format!("[{id}]")),
            "TypeScript twin lost {id}"
        );
    }
    for (file, needle) in [
        (
            "services/hub-rs/tests/files.rs",
            "fn shared_active_path_contract(",
        ),
        (
            "apps/desktop/src/main/lib/pathConfinement.test.ts",
            "describe('active path contract'",
        ),
    ] {
        assert!(
            fs::read_to_string(root().join(file))
                .unwrap()
                .contains(needle),
            "missing active loader {file}::{needle}"
        );
    }
}
#[test]
fn declared_block_loaders_resolve_to_real_test_sources() {
    let mut checked = 0;
    for (name, doc) in fixtures() {
        for (block, spec) in object(&doc["vocabulary"]["blocks"]) {
            if block == "_comment" {
                continue;
            }
            for entry in strings(&spec["loaders"]) {
                let (file, needle) = entry.split_once("::").expect("loader file::needle");
                assert!(!needle.is_empty());
                let path = Path::new(file);
                assert!(
                    !path.is_absolute()
                        && !path
                            .components()
                            .any(|p| p == std::path::Component::ParentDir)
                );
                let source = fs::read_to_string(root().join(path))
                    .unwrap_or_else(|e| panic!("{name}/{block}: {entry}: {e}"));
                assert!(
                    source.contains(needle),
                    "{name}/{block}: missing loader {entry}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 25, "block-loader corpus reduced to {checked}");
}

#[test]
fn retired_anonymous_parameter_harness_is_explicitly_archived() {
    let active = fixtures().remove("path-containment-cases.json").unwrap();
    assert!(active.get("paramShapes").is_none());
    assert!(active["vocabulary"]["blocks"].get("paramShapes").is_none());
    let archived: Value = serde_json::from_slice(
        &fs::read(root().join("contracts/retired/path-parameter-shapes.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(archived["paramShapes"].as_array().unwrap().len(), 17);
    assert_eq!(validate(&archived), Vec::<String>::new());
    let rationale = fs::read_to_string(root().join("contracts/retired/README.md")).unwrap();
    for evidence in [
        "conn.authorize",
        "paramString",
        "handleBus",
        "No TypeScript",
        "active",
    ] {
        assert!(
            rationale.contains(evidence),
            "retirement rationale lost {evidence}"
        );
    }
}

#[test]
fn retired_go_block_loaders_have_sealed_provenance_and_live_replacements() {
    use sha2::{Digest, Sha256};
    let bytes = repo::read(
        &root(),
        Path::new("contracts/retired/block-loader-provenance.json"),
    )
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "174f82db9374ba473137afd695563f06c281b362cc9b6e630dfbe06a5e29e016"
    );
    let archived: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(archived["version"], 1);
    assert_eq!(
        archived["referenceCommit"],
        "0c077b82e62965ce51f85b2d2a4c0f1aec872fea"
    );
    let rows = archived["loaders"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    let active = fixtures();
    let mut seen = BTreeSet::new();
    for row in rows {
        let fixture = row["fixture"].as_str().unwrap();
        let block = row["block"].as_str().unwrap();
        let loader = row["loader"].as_str().unwrap();
        assert!(seen.insert((fixture, block, loader)));
        let (file, needle) = loader.split_once("::").unwrap();
        assert!(file.starts_with("services/hub/") && !needle.is_empty());
        let expected = row["sourceSha256"].as_str().unwrap();
        assert_eq!(expected.len(), 64);
        assert!(expected.bytes().all(|c| c.is_ascii_hexdigit()));
        // If a reviewed original is deliberately restored, changed bytes do
        // not inherit its archived provenance. Absence never excuses a LIVE loader.
        match fs::read(root().join(file)) {
            Ok(original) => assert_eq!(format!("{:x}", Sha256::digest(original)), expected),
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        }
        let doc = &active[fixture];
        let mut cases = doc;
        for key in block.split('.') {
            cases = &cases[key];
        }
        assert!(cases.as_array().unwrap().len() >= row["caseCount"].as_u64().unwrap() as usize);
        let declared = strings(&doc["vocabulary"]["blocks"][block]["loaders"]);
        assert!(!declared.contains(&loader));
        let retained = strings(&row["retainedLoaders"]);
        assert!(retained.len() >= 2);
        for entry in retained {
            assert!(declared.contains(&entry), "lost retained loader {entry}");
            let (path, needle) = entry.split_once("::").unwrap();
            assert!(!path.starts_with("services/hub/"));
            assert!(
                fs::read_to_string(root().join(path))
                    .unwrap()
                    .contains(needle)
            );
        }
    }
}
