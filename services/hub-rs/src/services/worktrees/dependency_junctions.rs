//! Detach recorded dependency junctions without traversing their targets.
use anyhow::{Result, bail};
use std::{
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

pub(super) fn remove_recorded(cwd: &Path, source: &Path, links: &[PathBuf]) -> Result<()> {
    let mut verified = Vec::new();
    for relative in links {
        if relative
            .file_name()
            .is_none_or(|name| name != "node_modules")
            || relative.to_string_lossy().contains(':')
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            bail!("invalid recorded dependency link path");
        }
        let mut parent = cwd.to_owned();
        for part in relative.parent().unwrap().components() {
            parent.push(part);
            let metadata = std::fs::symlink_metadata(&parent)?;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                bail!("recorded dependency link has a changed parent");
            }
        }
        let link = cwd.join(relative);
        match std::fs::symlink_metadata(&link) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        if !junction::exists(&link)? {
            bail!("recorded dependency link is no longer a junction");
        }
        let target = junction::get_target(&link)?;
        if std::fs::canonicalize(target)? != std::fs::canonicalize(source.join(relative))? {
            bail!("recorded dependency junction target changed");
        }
        verified.push(link);
    }
    for link in verified {
        // Removes only the reparse point and its empty directory, not contents
        // reached through the junction. Paths already ordinary directories were refused above.
        junction::delete(&link)?;
        // junction::delete clears the reparse data but retains the backing
        // directory. Remove that now-empty directory without recursive cleanup.
        std::fs::remove_dir(link)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_junction_is_detached_without_touching_source_or_unrecorded_links() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let cwd = dir.path().join("tree");
        let relative = PathBuf::from("apps/deep/node_modules");
        std::fs::create_dir_all(source.join(&relative)).unwrap();
        std::fs::create_dir_all(cwd.join("apps/deep")).unwrap();
        std::fs::write(source.join(&relative).join("keep.js"), "source content").unwrap();
        junction::create(source.join(&relative), cwd.join(&relative)).unwrap();
        junction::create(source.join(&relative), cwd.join("unrecorded")).unwrap();
        remove_recorded(&cwd, &source, &[relative.clone()]).unwrap();
        assert!(!cwd.join(relative).exists());
        assert!(junction::exists(cwd.join("unrecorded")).unwrap());
        assert_eq!(
            std::fs::read(source.join("apps/deep/node_modules/keep.js")).unwrap(),
            b"source content"
        );
    }

    #[test]
    fn changed_target_real_directory_and_reparse_parent_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let cwd = dir.path().join("tree");
        let other = dir.path().join("other");
        std::fs::create_dir_all(source.join("node_modules")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("keep"), "preserve").unwrap();
        junction::create(&other, cwd.join("node_modules")).unwrap();
        assert!(remove_recorded(&cwd, &source, &["node_modules".into()]).is_err());
        assert!(junction::exists(cwd.join("node_modules")).unwrap());
        junction::delete(cwd.join("node_modules")).unwrap();
        assert!(cwd.join("node_modules").is_dir());
        assert!(remove_recorded(&cwd, &source, &["node_modules".into()]).is_err());
        assert!(cwd.join("node_modules").is_dir());
        junction::create(&other, cwd.join("apps")).unwrap();
        assert!(remove_recorded(&cwd, &source, &["apps/node_modules".into()]).is_err());
        assert!(junction::exists(cwd.join("apps")).unwrap());
        assert_eq!(std::fs::read(other.join("keep")).unwrap(), b"preserve");
        assert!(remove_recorded(&cwd, &source, &["../node_modules".into()]).is_err());
    }
}
