//! Source-only caller binding analysis. No provider processes or runtime APIs.
//! Unresolved input flows are evidence gaps, never silently counted as inert.
mod index;
pub mod policy;
pub mod reference;
pub mod spawn_support;
mod trace;
pub use index::{Index, read_sources};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bound {
    pub fields: BTreeSet<String>,
    pub opaque: BTreeSet<String>,
    pub opaque_paths: BTreeSet<String>,
    pub key_inspections: BTreeSet<String>,
    pub opaque_transforms: BTreeSet<String>,
    pub unresolved: BTreeSet<String>,
    pub sources: BTreeSet<String>,
}
impl Bound {
    pub(crate) fn merge(&mut self, other: Self) {
        self.fields.extend(other.fields);
        self.opaque.extend(other.opaque);
        self.opaque_paths.extend(other.opaque_paths);
        self.key_inspections.extend(other.key_inspections);
        self.opaque_transforms.extend(other.opaque_transforms);
        self.unresolved.extend(other.unresolved);
        self.sources.extend(other.sources);
    }
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Report {
    pub methods: BTreeMap<String, Bound>,
    pub unresolved_registrations: BTreeSet<String>,
    pub source_files: usize,
}
/// The trace recurses once per call and expression level, so its depth grows
/// with the hub's call chains; it runs on its own thread with a stack sized
/// for that, not the caller's (2 MiB for a test thread). Syn trees are not
/// `Send`, so the index is built on that thread too.
const SCAN_STACK: usize = 256 << 20;
pub fn scan(sources: BTreeMap<String, String>) -> Result<Report, String> {
    std::thread::Builder::new()
        .name("capability-scan".into())
        .stack_size(SCAN_STACK)
        .spawn(move || Ok(trace::scan(&Index::parse(sources)?)))
        .map_err(|e| format!("spawn scan thread: {e}"))?
        .join()
        .map_err(|_| "scan thread panicked".to_string())?
}
