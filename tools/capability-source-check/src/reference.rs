//! Frozen legacy scanner output: individual binding closure and byte provenance.
//! Capturing is optional Go tooling; checking is pure Rust and source-only.
use crate::Report;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, process::Command};
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub scanner_sha256: String,
    pub sources: BTreeMap<String, String>,
    pub vocabulary_sha256: String,
    pub dangerous_bindings: usize,
    pub methods: BTreeMap<String, Binding>,
}
#[derive(Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub dangerous: Vec<String>,
}
const MANIFEST: &str = "tools/capability-source-check/go-reference-provenance.json";
const SEAL: &str = "0b06d5507b2ab23d2bd63400e0921fbcaa76407029987607acb7bbe1137d4525";
const VOCABULARY: &str = "apps/desktop/tests/fixtures/capability-parameter-vocabulary.json";
const SCANNER: &str = "services/hub/cmd/brain/capspec_params_test.go";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Provenance {
    version: u32,
    mode: String,
    reference_commit: String,
    reference_tree: String,
    capture_path: String,
    capture_sha256: String,
    dangerous_bindings: usize,
    review: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Reference {
    fn provenance(&self, root: &Path) -> Result<Provenance, String> {
        let bytes = std::fs::read(root.join(MANIFEST))
            .map_err(|e| format!("captured provenance unavailable: {e}"))?;
        if digest(&bytes) != SEAL {
            return Err("captured provenance seal changed".into());
        }
        let manifest: Provenance = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid captured provenance: {e}"))?;
        if manifest.version != 1
            || manifest.mode != "captured-provenance"
            || manifest.dangerous_bindings != 84
            || manifest.review.trim().is_empty()
        {
            return Err("invalid captured provenance mode/population".into());
        }
        let capture = std::fs::read(root.join(&manifest.capture_path))
            .map_err(|e| format!("sealed Go capture unavailable: {e}"))?;
        if digest(&capture) != manifest.capture_sha256 {
            return Err("sealed Go capture digest changed".into());
        }
        let original: Self = serde_json::from_slice(&capture)
            .map_err(|e| format!("invalid sealed Go capture: {e}"))?;
        if *self != original {
            return Err("parsed Go reference differs from sealed capture".into());
        }
        Ok(manifest)
    }
    /// Explicit oracle verification requires the pinned historical checkout,
    /// never merely the presence of an arbitrary old directory.
    pub fn verify_historical(&self, root: &Path, checkout: &Path) -> Vec<String> {
        let mut errors = Vec::new();
        let manifest = match self.provenance(root) {
            Ok(m) => m,
            Err(e) => return vec![e],
        };
        for (revision, expected) in [
            ("HEAD", manifest.reference_commit),
            ("HEAD^{tree}", manifest.reference_tree),
        ] {
            match Command::new("git")
                .arg("-C")
                .arg(checkout)
                .args(["rev-parse", "--verify", revision])
                .output()
            {
                Ok(output)
                    if output.status.success()
                        && String::from_utf8_lossy(&output.stdout).trim() == expected =>
                {
                    ()
                }
                _ => errors.push(format!(
                    "historical checkout {revision} does not match pinned provenance"
                )),
            }
        }
        errors.extend(self.source_bytes(checkout, false));
        match std::fs::read(checkout.join(VOCABULARY)) {
            Ok(bytes) if digest(&bytes) == self.vocabulary_sha256 => (),
            Ok(_) => errors.push(format!(
                "historical vocabulary provenance changed: {VOCABULARY}"
            )),
            Err(error) => errors.push(format!(
                "historical vocabulary unavailable: {VOCABULARY}: {error}"
            )),
        }
        errors
    }
    fn source_bytes(&self, root: &Path, allow_captured_absence: bool) -> Vec<String> {
        let mut errors = Vec::new();
        for (path, expected) in self
            .sources
            .iter()
            .map(|(p, h)| (p.as_str(), h))
            .chain([(SCANNER, &self.scanner_sha256)])
        {
            match std::fs::read(root.join(path)) {
                Ok(bytes) if digest(&bytes) == *expected => (),
                Ok(_) => errors.push(format!("Go reference provenance changed: {path}")),
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && allow_captured_absence =>
                {
                    ()
                }
                Err(error) => errors.push(format!("Go reference unavailable: {path}: {error}")),
            }
        }
        errors
    }

    pub fn check(&self, root: &Path, report: &Report) -> Vec<String> {
        let mut errors = Vec::new();
        let count: usize = self.methods.values().map(|b| b.dangerous.len()).sum();
        if count != 84 || self.dangerous_bindings != count || self.sources.is_empty() {
            errors.push("original Go binding/provenance population changed".into());
        }
        let sealed = match self.provenance(root) {
            Ok(_) => true,
            Err(error) => {
                errors.push(error);
                false
            }
        };
        errors.extend(self.source_bytes(root, sealed));
        // This current independent vocabulary is never an optional historical path.
        match std::fs::read(root.join(VOCABULARY)) {
            Ok(bytes) if digest(&bytes) == self.vocabulary_sha256 => (),
            Ok(_) => errors.push(format!("Go reference provenance changed: {VOCABULARY}")),
            Err(error) => errors.push(format!(
                "live vocabulary unavailable: {VOCABULARY}: {error}"
            )),
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
