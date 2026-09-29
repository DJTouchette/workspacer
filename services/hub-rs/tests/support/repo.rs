//! Strict repository discovery for cross-stack guards. Extracted/vendored test
//! sources are an error too: these guards never turn missing inputs into skips.
use std::path::{Path, PathBuf};

pub fn checked_root(candidate: &Path) -> Result<PathBuf, String> {
    let root = candidate.canonicalize().map_err(|error| {
        format!(
            "repository root {} is unavailable: {error}",
            candidate.display()
        )
    })?;
    for marker in ["services/hub-rs/Cargo.toml", "Makefile"] {
        if !root.join(marker).is_file() {
            return Err(format!(
                "repository {} is missing root marker {marker}; cross-repo guards cannot skip a moved or extracted checkout",
                root.display()
            ));
        }
    }
    Ok(root)
}

pub fn root() -> PathBuf {
    checked_root(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

pub fn read(root: &Path, relative: &Path) -> Result<Vec<u8>, String> {
    let root = checked_root(root)?;
    let path = root.join(relative);
    std::fs::read(&path).map_err(|error| {
        format!("required cross-repo input {} is unreadable: {error}; a moved fixture cannot be skipped", path.display())
    })
}
