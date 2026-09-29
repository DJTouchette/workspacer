//! Install the public command from the running Rust artifact; never resolve an
//! existing `workspacer` through PATH (it may still be the retired Go launcher).
use anyhow::Result;
use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn run(directory: Option<&Path>, out: &mut dyn Write) -> Result<i32> {
    let source = std::env::current_exe()?.canonicalize()?;
    let directory = directory
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .unwrap_or_default();
            let local = std::env::var_os("LOCALAPPDATA")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from);
            pick_directory(cfg!(windows), &home, local.as_deref(), |path| {
                tempfile::NamedTempFile::new_in(path).is_ok()
            })
        });
    install_to(
        &source,
        &directory,
        &std::env::var_os("PATH").unwrap_or_default(),
        out,
    )?;
    Ok(0)
}

fn pick_directory(
    windows: bool,
    home: &Path,
    local: Option<&Path>,
    writable: impl FnOnce(&Path) -> bool,
) -> PathBuf {
    if windows {
        return local
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("workspacer/bin");
    }
    let system = Path::new("/usr/local/bin");
    if writable(system) {
        system.to_path_buf()
    } else {
        home.join(".local/bin")
    }
}

fn install_to(source: &Path, directory: &Path, path: &OsStr, out: &mut dyn Write) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let destination = directory.join(if cfg!(windows) {
        "workspacer.exe"
    } else {
        "workspacer"
    });
    install_binary(source, &destination, platform_link)?;
    writeln!(
        out,
        "installed {} -> {}",
        destination.display(),
        source.display()
    )?;
    if !on_path(directory, path) {
        writeln!(
            out,
            "\n{} is not on your PATH. Add it:",
            directory.display()
        )?;
        if cfg!(windows) {
            writeln!(
                out,
                "  Settings > System > About > Advanced system settings > Environment Variables"
            )?;
        } else {
            writeln!(
                out,
                "  Add {} to PATH in ~/.profile or your shell's rc file.",
                directory.display()
            )?;
        }
    }
    Ok(())
}

fn on_path(directory: &Path, path: &OsStr) -> bool {
    !directory.as_os_str().is_empty() && std::env::split_paths(path).any(|entry| entry == directory)
}

fn platform_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination)
    }
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

fn install_binary(
    source: &Path,
    destination: &Path,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let source = source.canonicalize()?;
    if destination.canonicalize().ok().as_ref() == Some(&source) {
        return Ok(());
    }
    // Stage first: a failed copy must not destroy a previous working command.
    let staging = tempfile::Builder::new()
        .prefix(".workspacer-install-")
        .tempdir_in(destination.parent().unwrap())?;
    let staged = staging.path().join("command");
    if link(&source, &staged).is_err() {
        std::fs::copy(&source, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    #[cfg(windows)]
    {
        // Windows can rename a running executable away even when replacement
        // fails. Keep the old copy if Windows cannot delete it until exit.
        if std::fs::symlink_metadata(destination).is_ok() {
            let backup = destination.with_extension("exe.old");
            let _ = std::fs::remove_file(&backup);
            std::fs::rename(destination, &backup)?;
            if let Err(error) = std::fs::rename(&staged, destination) {
                let _ = std::fs::rename(&backup, destination);
                return Err(error.into());
            }
            let _ = std::fs::remove_file(backup);
            return Ok(());
        }
    }
    std::fs::rename(staged, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destination_selection_is_pure_and_matches_platform_defaults() {
        let home = Path::new("home");
        assert_eq!(
            pick_directory(false, home, None, |_| true),
            Path::new("/usr/local/bin")
        );
        assert_eq!(
            pick_directory(false, home, None, |_| false),
            home.join(".local/bin")
        );
        assert_eq!(
            pick_directory(true, home, None, |_| panic!(
                "Windows must not probe system bin"
            )),
            home.join("AppData/Local/workspacer/bin")
        );
        assert_eq!(
            pick_directory(true, home, Some(Path::new("local")), |_| panic!()),
            Path::new("local/workspacer/bin")
        );
    }
    #[test]
    fn public_alias_replaces_old_launcher_and_reports_path_without_mutating_it() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("workspacer-rust");
        std::fs::write(&source, b"rust artifact").unwrap();
        let directory = root.path().join("bin");
        std::fs::create_dir(&directory).unwrap();
        let alias = directory.join(if cfg!(windows) {
            "workspacer.exe"
        } else {
            "workspacer"
        });
        std::fs::write(&alias, b"old Go launcher").unwrap();
        let mut out = Vec::new();
        install_to(&source, &directory, OsStr::new(""), &mut out).unwrap();
        assert_eq!(std::fs::read(&alias).unwrap(), b"rust artifact");
        assert!(String::from_utf8(out).unwrap().contains("not on your PATH"));
        let path = std::env::join_paths([&directory]).unwrap();
        let mut out = Vec::new();
        install_to(&source, &directory, &path, &mut out).unwrap();
        assert!(!String::from_utf8(out).unwrap().contains("not on your PATH"));
        assert!(!directory.join("workspacer-rust").exists());
        assert!(!on_path(Path::new(""), OsStr::new("")));
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(&alias).unwrap(),
            source.canonicalize().unwrap()
        );
    }
    #[test]
    fn failed_staging_preserves_the_previous_command() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source-directory");
        std::fs::create_dir(&source).unwrap();
        let destination = root.path().join("command");
        std::fs::write(&destination, b"previous command").unwrap();
        assert!(
            install_binary(&source, &destination, |_, _| Err(
                std::io::ErrorKind::Unsupported.into()
            ))
            .is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous command");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn failed_link_falls_back_to_executable_copy_and_self_install_is_noop() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("command");
        std::fs::write(&source, b"rust").unwrap();
        install_binary(&source, &destination, |_, _| {
            Err(std::io::ErrorKind::PermissionDenied.into())
        })
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"rust");
        assert!(
            !std::fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&destination)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
        install_binary(&source, &source, |_, _| panic!("must not replace itself")).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"rust");
    }
}
