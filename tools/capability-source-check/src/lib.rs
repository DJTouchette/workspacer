//! Source-only caller binding analysis. No provider processes or runtime APIs.
//! Unresolved input flows are evidence gaps, never silently counted as inert.
mod index;
pub mod policy;
pub mod reference;
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
pub fn scan(sources: BTreeMap<String, String>) -> Result<Report, String> {
    let index = Index::parse(sources)?;
    Ok(trace::scan(&index))
}
