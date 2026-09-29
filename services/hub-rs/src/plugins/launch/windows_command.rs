//! Resolve a Windows npm shim before constructing the shell-free Codex probe.
use std::{collections::BTreeMap, path::Path};
pub(super) fn resolve(binary: &str, cwd: &Path, env: &BTreeMap<String, String>) -> String {
    let path = Path::new(binary);
    if path.components().count() != 1 {
        return if path.is_relative() {
            cwd.join(path).to_string_lossy().into_owned()
        } else {
            binary.into()
        };
    }
    let search = env
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| std::ffi::OsString::from(value))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let names = if path.extension().is_some() {
        vec![binary.to_owned()]
    } else {
        vec![format!("{binary}.exe"), format!("{binary}.cmd")]
    };
    std::env::split_paths(&search)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| binary.into())
}
