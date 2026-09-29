use std::path::PathBuf;
/// Host configuration calls this with its chosen standard config path. The
/// runtime itself never consults ambient user directories for hub state.
pub(crate) fn historical_directory(
    selected: &std::path::Path,
    standard: &std::path::Path,
) -> Option<PathBuf> {
    let normalized = |path: &std::path::Path| {
        super::paths::canonicalize(path)
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let selected_normalized = normalized(selected);
    let standard_normalized = normalized(standard);
    if !super::paths::contained(&selected_normalized, &standard_normalized)
        || !super::paths::contained(&standard_normalized, &selected_normalized)
    {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/workspacer-hub"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        standard
            .parent()
            .map(|parent| parent.join("workspacer-hub"))
    }
}
