//! Where the app keeps its own data, and the one-time move from the name
//! preview builds used.
use std::path::{Path, PathBuf};

/// The app's data folder, under the platform's local data directory.
pub const DATA_FOLDER: &str = "Workspacer Native";
/// What preview builds called it.
pub const PREVIEW_DATA_FOLDER: &str = "Workspacer Native Rust Preview";

/// The data folder under `base`, moving a preview build's folder to the
/// current name when that is safe. The preview folder stays in use when it
/// cannot be moved (another copy of the app holds it open, or the new name
/// is already taken by something that is not empty); nothing is merged or
/// deleted.
pub fn adopt_preview_directory(base: &Path) -> PathBuf {
    let (current, preview) = (base.join(DATA_FOLDER), base.join(PREVIEW_DATA_FOLDER));
    if !preview.is_dir() {
        return current;
    }
    let holds_data =
        |dir: &Path| dir.join("state.db").exists() || dir.join("config/config.yaml").exists();
    if current.exists() {
        if holds_data(&current) || !holds_data(&preview) {
            return current;
        }
        // Only an empty folder is replaced: remove_dir refuses anything else.
        if std::fs::remove_dir(&current).is_err() {
            return preview;
        }
    }
    match std::fs::rename(&preview, &current) {
        Ok(()) => current,
        Err(error) => {
            eprintln!(
                "Could not rename {} to {}; using it as is: {error}",
                preview.display(),
                current.display()
            );
            preview
        }
    }
}

/// Written into the data folder by Settings → About to ask the next launch
/// to import the Electron app's data over what is there (after a backup).
pub const IMPORT_REQUEST_FILE: &str = "import-electron.requested";

/// The data folder under the platform's local data directory, without
/// creating or moving anything.
pub fn located(base: &Path) -> PathBuf {
    let (current, preview) = (base.join(DATA_FOLDER), base.join(PREVIEW_DATA_FOLDER));
    if !current.exists() && preview.is_dir() {
        preview
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wks-data-dir-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_fresh_install_uses_the_current_name_and_creates_nothing() {
        let base = base("fresh");
        assert_eq!(adopt_preview_directory(&base), base.join(DATA_FOLDER));
        assert!(!base.join(DATA_FOLDER).exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_preview_folder_is_renamed_with_its_contents() {
        let base = base("rename");
        let preview = base.join(PREVIEW_DATA_FOLDER);
        std::fs::create_dir_all(preview.join("config")).unwrap();
        std::fs::write(preview.join("state.db"), b"sessions").unwrap();
        std::fs::write(preview.join("config/config.yaml"), b"a: 1").unwrap();
        assert_eq!(located(&base), preview);
        let current = adopt_preview_directory(&base);
        assert_eq!(current, base.join(DATA_FOLDER));
        assert!(!preview.exists());
        assert_eq!(
            std::fs::read(current.join("state.db")).unwrap(),
            b"sessions"
        );
        assert_eq!(located(&base), current);
        // A second launch finds nothing left to move.
        assert_eq!(adopt_preview_directory(&base), current);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn an_empty_current_folder_gives_way_but_one_holding_files_does_not() {
        let base = base("occupied");
        let (current, preview) = (base.join(DATA_FOLDER), base.join(PREVIEW_DATA_FOLDER));
        std::fs::create_dir_all(&preview).unwrap();
        std::fs::write(preview.join("state.db"), b"sessions").unwrap();
        std::fs::create_dir_all(&current).unwrap();
        assert_eq!(adopt_preview_directory(&base), current);
        assert_eq!(
            std::fs::read(current.join("state.db")).unwrap(),
            b"sessions"
        );

        // Logs written under the new name before the data could move: the
        // preview folder stays in use and neither folder loses anything.
        std::fs::rename(&current, &preview).unwrap();
        std::fs::create_dir_all(current.join("logs")).unwrap();
        std::fs::write(current.join("logs/wks-native.log"), b"log").unwrap();
        assert_eq!(adopt_preview_directory(&base), preview);
        assert!(current.join("logs/wks-native.log").exists());
        assert!(preview.join("state.db").exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn data_under_both_names_is_never_merged() {
        let base = base("both");
        let (current, preview) = (base.join(DATA_FOLDER), base.join(PREVIEW_DATA_FOLDER));
        for (dir, text) in [(&current, "new"), (&preview, "old")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("state.db"), text).unwrap();
        }
        assert_eq!(adopt_preview_directory(&base), current);
        assert_eq!(std::fs::read(current.join("state.db")).unwrap(), b"new");
        assert_eq!(std::fs::read(preview.join("state.db")).unwrap(), b"old");
        std::fs::remove_dir_all(base).unwrap();
    }
}
