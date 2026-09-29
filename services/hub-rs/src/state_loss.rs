//! Distinguish a missing create-once file from a genuinely empty first run.
//! Empty bootstrap directories are not evidence; zero-byte files and unknown
//! or unreadable child directories are. This is a diagnostic, not history.
use std::{ffi::OsStr, path::Path};

pub(crate) fn suspected(directory: &Path, missing: &OsStr) -> bool {
    suspected_ignoring(directory, missing, &[])
}

/// Exclusions are explicit host-created bookkeeping files, never a blanket
/// dotfile/lock suffix exception that could hide real persisted state.
pub(crate) fn suspected_ignoring(directory: &Path, missing: &OsStr, ignored: &[&OsStr]) -> bool {
    let Ok(entries) = std::fs::read_dir(directory)
        .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
    else {
        return false;
    };
    for entry in entries {
        let name = entry.file_name();
        if name == missing || ignored.iter().any(|ignored| name == *ignored) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && std::fs::read_dir(entry.path()).is_ok_and(|mut children| children.next().is_none())
        {
            continue;
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_suspected_corpus_distinguishes_empty_bootstrap_from_real_state() {
        let cases: &[(&str, Option<&[&str]>, &str, bool)] = &[
            ("directory absent", None, "remote-token", false),
            ("directory empty", Some(&[]), "remote-token", false),
            (
                "own file is not evidence",
                Some(&["remote-token"]),
                "remote-token",
                false,
            ),
            ("another file", Some(&["config.yaml"]), "remote-token", true),
            (
                "empty child directory",
                Some(&["sessions/"]),
                "config.yaml",
                false,
            ),
            (
                "bootstrap directories",
                Some(&["plugins/", "library/", "layouts/", "sessions/", "logs/"]),
                "config.yaml",
                false,
            ),
            (
                "nonempty child directory",
                Some(&["sessions/live.json"]),
                "config.yaml",
                true,
            ),
            (
                "file beside empty directories",
                Some(&["plugins/", "tokens.json"]),
                "config.yaml",
                true,
            ),
            (
                "zero byte file",
                Some(&["tokens.json:"]),
                "config.yaml",
                true,
            ),
            (
                "own file beside other state",
                Some(&["remote-token", "tokens.json"]),
                "remote-token",
                true,
            ),
        ];
        let root = std::env::temp_dir().join(format!(
            "wks-state-loss-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        for (index, (name, seed, missing, expected)) in cases.iter().enumerate() {
            let directory = root.join(index.to_string());
            if let Some(entries) = seed {
                std::fs::create_dir_all(&directory).unwrap();
                for entry in *entries {
                    let file = directory.join(entry.trim_end_matches(':'));
                    if entry.ends_with('/') {
                        std::fs::create_dir_all(file).unwrap();
                    } else {
                        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                        std::fs::write(
                            file,
                            if entry.ends_with(':') {
                                &b""[..]
                            } else {
                                &b"x"[..]
                            },
                        )
                        .unwrap();
                    }
                }
            }
            assert_eq!(
                suspected(&directory, OsStr::new(missing)),
                *expected,
                "{name}"
            );
        }
        let directory = root.join("lock-only");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(".remote-token.lock"), b"").unwrap();
        assert!(suspected(&directory, OsStr::new("remote-token")));
        assert!(!suspected_ignoring(
            &directory,
            OsStr::new("remote-token"),
            &[OsStr::new(".remote-token.lock")]
        ));
        std::fs::write(directory.join("some-other.lock"), b"").unwrap();
        assert!(suspected_ignoring(
            &directory,
            OsStr::new("remote-token"),
            &[OsStr::new(".remote-token.lock")]
        ));
    }
}
