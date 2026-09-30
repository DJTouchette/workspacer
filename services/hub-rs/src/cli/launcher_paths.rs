//! Launcher choices stay explicit; inferred session storage must never land in CWD.
use anyhow::{Result, ensure};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(super) fn database(
    explicit: Option<&Path>,
    xdg: Option<&OsStr>,
    fallback: impl FnOnce() -> PathBuf,
) -> Result<PathBuf> {
    if let Some(path) = explicit.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.into());
    }
    if let Some(xdg) = xdg {
        let path = Path::new(xdg).join("claudemon/state.db");
        ensure!(
            path.is_absolute(),
            "XDG_DATA_HOME must be absolute; pass --claudemon-db-path instead of placing session state relative to the working directory"
        );
        return Ok(path);
    }
    let path = fallback();
    ensure!(
        path.is_absolute(),
        "cannot derive an absolute session database path; set a home directory or pass --claudemon-db-path"
    );
    Ok(path)
}

pub(super) fn webapp(directory: &Path) -> Option<PathBuf> {
    [directory.join("web"), directory.join("../web")]
        .into_iter()
        .find(|path| path.join("index.html").is_file())
}

pub(super) fn select_webapp(
    explicit: Option<&Path>,
    environment: Option<std::ffi::OsString>,
    discover: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = explicit {
        // Empty is an explicit disable, not an invitation to rediscover assets.
        return (!path.as_os_str().is_empty()).then(|| path.to_path_buf());
    }
    environment
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(discover)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_empty_webapp_suppresses_environment_and_discovery() {
        assert_eq!(
            select_webapp(Some(Path::new("")), Some("ambient".into()), || panic!(
                "explicit disable reached discovery"
            )),
            None
        );
        assert_eq!(
            select_webapp(
                Some(Path::new("chosen")),
                Some("ambient".into()),
                || panic!()
            ),
            Some(PathBuf::from("chosen"))
        );
        assert_eq!(
            select_webapp(None, Some("ambient".into()), || panic!()),
            Some(PathBuf::from("ambient"))
        );
        for environment in [None, Some("".into())] {
            assert_eq!(
                select_webapp(None, environment, || Some(PathBuf::from("bundled"))),
                Some(PathBuf::from("bundled"))
            );
        }
    }
    #[test]
    fn explicit_database_wins_but_inferred_relative_or_empty_environment_is_refused() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            database(None, Some(root.path().as_os_str()), || panic!()).unwrap(),
            root.path().join("claudemon/state.db")
        );
        for bad in ["", "relative/data"] {
            assert!(
                database(None, Some(OsStr::new(bad)), || panic!())
                    .unwrap_err()
                    .to_string()
                    .contains("XDG_DATA_HOME")
            );
        }
        assert_eq!(
            database(
                Some(Path::new("chosen.db")),
                Some(OsStr::new("")),
                || panic!()
            )
            .unwrap(),
            Path::new("chosen.db")
        );
        assert!(
            database(Some(Path::new("")), None, || PathBuf::from(
                ".claudemon/state.db"
            ))
            .is_err()
        );
        assert_eq!(
            database(None, None, || root.path().join(".claudemon/state.db")).unwrap(),
            root.path().join(".claudemon/state.db")
        );
    }
    #[test]
    fn shipped_web_assets_require_an_index_and_prefer_the_executable_sibling() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(bin.join("web")).unwrap();
        std::fs::create_dir(root.path().join("web")).unwrap();
        assert!(webapp(&bin).is_none());
        std::fs::write(root.path().join("web/index.html"), "parent").unwrap();
        assert_eq!(webapp(&bin).unwrap(), bin.join("../web"));
        std::fs::create_dir(bin.join("web/index.html")).unwrap();
        assert_eq!(webapp(&bin).unwrap(), bin.join("../web"));
        std::fs::remove_dir(bin.join("web/index.html")).unwrap();
        std::fs::write(bin.join("web/index.html"), "sibling").unwrap();
        assert_eq!(webapp(&bin).unwrap(), bin.join("web"));
    }
}
