//! Persisted task projects may predate the canonical spelling used by launch
//! admission (including alias-spelled legacy headless projects). Compare directory
//! identity without rewriting history or
//! treating a case-folded/nonexistent spelling as an authorization proof.
pub(crate) fn same_cwd(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    use std::path::Path;
    let (left, right) = (Path::new(left), Path::new(right));
    if !left.is_absolute() || !right.is_absolute() {
        return false;
    }
    // Use the same link-before-parent resolver as spawn/confinement. A
    // lexical prefix strip or lowercase comparison could conflate objects.
    let (Ok(left), Ok(right)) = (
        crate::services::paths::canonicalize(left),
        crate::services::paths::canonicalize(right),
    ) else {
        return false;
    };
    left.is_dir() && right.is_dir() && left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_persisted_spelling_survives_offline_history_but_other_projects_do_not_match() {
        let directory = tempfile::tempdir().unwrap();
        let left = directory.path().join("offline-project");
        let right = directory.path().join("offline-other");
        assert!(same_cwd(left.to_str().unwrap(), left.to_str().unwrap()));
        assert!(!same_cwd(left.to_str().unwrap(), right.to_str().unwrap()));
        assert!(!same_cwd("", ""));
    }
    #[cfg(windows)]
    #[test]
    fn dos_git_and_verbatim_spellings_match_only_the_same_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("MixedCaseProject");
        let other = root.path().join("OtherProject");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&other).unwrap();
        let canonical = crate::services::paths::canonicalize(&project).unwrap();
        let canonical = canonical.to_str().unwrap();
        let plain = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
        let git = plain.replace('\\', "/").to_lowercase();
        assert!(same_cwd(project.to_str().unwrap(), canonical));
        assert!(same_cwd(&git, canonical));
        assert!(!same_cwd(other.to_str().unwrap(), canonical));
        assert!(!same_cwd("MixedCaseProject", canonical));
        // Missing tails are valid write targets elsewhere, but cannot prove
        // that two different project spellings name the same live directory.
        assert!(!same_cwd(
            &format!("{git}/missing"),
            &format!("{canonical}\\missing")
        ));
        let file = project.join("file.txt");
        std::fs::write(&file, b"fixture").unwrap();
        assert!(!same_cwd(
            file.to_str().unwrap(),
            &file.to_string_lossy().replace('\\', "/")
        ));
    }
}
