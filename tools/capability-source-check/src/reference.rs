//! Frozen legacy scanner output: individual binding closure and byte provenance.
//! Capturing is optional Go tooling; checking is pure Rust and source-only.
use crate::Report;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub scanner_sha256: String,
    pub sources: BTreeMap<String, String>,
    pub vocabulary_sha256: String,
    pub dangerous_bindings: usize,
    pub methods: BTreeMap<String, Binding>,
}
#[derive(Deserialize)]
pub struct Binding {
    pub dangerous: Vec<String>,
}
impl Reference {
    pub fn check(&self, root: &Path, report: &Report) -> Vec<String> {
        let mut errors = Vec::new();
        let count: usize = self.methods.values().map(|b| b.dangerous.len()).sum();
        if count != 84 || self.dangerous_bindings != count || self.sources.is_empty() {
            errors.push("original Go binding/provenance population changed".into());
        }
        for (path, expected) in self.sources.iter().map(|(a, b)| (a.as_str(), b)).chain([
            (
                "services/hub/cmd/brain/capspec_params_test.go",
                &self.scanner_sha256,
            ),
            (
                "apps/desktop/tests/fixtures/capability-parameter-vocabulary.json",
                &self.vocabulary_sha256,
            ),
        ]) {
            match std::fs::read(root.join(path)) {
                Ok(bytes) if format!("{:x}", Sha256::digest(&bytes)) == *expected => (),
                Ok(_) => errors.push(format!("Go reference provenance changed: {path}")),
                Err(error) => errors.push(format!("Go reference unavailable: {path}: {error}")),
            }
        }
        for (method, binding) in &self.methods {
            for field in &binding.dangerous {
                if !report.methods.get(method).is_some_and(|b| {
                    b.fields.iter().any(|p| {
                        p.rsplit('.')
                            .next()
                            .is_some_and(|leaf| leaf.eq_ignore_ascii_case(field))
                    })
                }) {
                    errors.push(format!(
                        "missing original Go caller binding {method}.{field}"
                    ));
                }
            }
        }
        errors
    }
}
