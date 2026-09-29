//! Resolve links before parent components; opening a cleaned spelling first
//! would change the selected object for inputs such as `link/../file`.
use anyhow::{Result, bail};
use std::{
    collections::VecDeque,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

#[derive(Clone)]
enum Part {
    Root(PathBuf),
    Parent,
    Name(OsString),
}
fn parts(path: &Path) -> VecDeque<Part> {
    let mut result = VecDeque::new();
    let mut root = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Prefix(p) => root.push(p.as_os_str()),
            Component::RootDir => {
                root.push(std::path::MAIN_SEPARATOR_STR);
                result.push_back(Part::Root(root.clone()));
            }
            Component::ParentDir => result.push_back(Part::Parent),
            Component::Normal(p) => result.push_back(Part::Name(p.into())),
            Component::CurDir => (),
        }
    }
    result
}
pub fn canonicalize(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("path is empty or not absolute");
    }
    let mut queue = parts(path);
    let mut resolved = PathBuf::new();
    let mut hops = 0;
    while let Some(part) = queue.pop_front() {
        match part {
            Part::Root(root) => resolved = root,
            Part::Parent => {
                resolved.pop();
            }
            Part::Name(name) => {
                #[cfg(windows)]
                let name = {
                    let text = name.to_string_lossy();
                    let trimmed = text.trim_end_matches([' ', '.']);
                    if trimmed.is_empty() {
                        match text.trim_end_matches(' ') {
                            "." => continue,
                            ".." => {
                                resolved.pop();
                                continue;
                            }
                            _ => bail!("path component names nothing"),
                        }
                    }
                    OsString::from(trimmed)
                };
                let next = resolved.join(&name);
                match std::fs::symlink_metadata(&next) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        hops += 1;
                        if hops > 40 {
                            bail!("too many symbolic links");
                        }
                        let link = std::fs::read_link(&next)?;
                        let mut target = parts(&link);
                        target.append(&mut queue);
                        queue = target;
                    }
                    Ok(_) => resolved = next,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Windows conflates a missing name with traversal through
                        // a regular file. Require every existing parent to walk.
                        if let Ok(parent) = std::fs::metadata(&resolved)
                            && !parent.is_dir()
                        {
                            bail!("path parent is not a directory");
                        }
                        resolved = next;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    #[cfg(windows)]
    {
        // Git and native APIs can spell the same directory with a DOS short
        // name, a different case, or a verbatim prefix. Resolve the existing
        // ancestor through the OS after the link-before-parent walk above.
        // Missing tails remain supported for writes and selected new entries.
        let mut ancestor = resolved.as_path();
        let mut missing = Vec::new();
        loop {
            match std::fs::canonicalize(ancestor) {
                Ok(mut canonical) => {
                    for name in missing.into_iter().rev() {
                        canonical.push(name);
                    }
                    return Ok(canonical);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let Some(name) = ancestor.file_name() else {
                        return Err(error.into());
                    };
                    missing.push(name.to_os_string());
                    let Some(parent) = ancestor.parent() else {
                        return Err(error.into());
                    };
                    ancestor = parent;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    #[cfg(not(windows))]
    Ok(resolved)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn git_style_and_verbatim_paths_share_one_selected_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("MixedCaseDirectory");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("file.txt");
        std::fs::write(&file, "fixture").unwrap();
        let canonical = std::fs::canonicalize(&root).unwrap();
        let spelling = canonical.to_string_lossy();
        let plain = spelling.strip_prefix(r"\\?\").unwrap_or(&spelling);
        let git_style = PathBuf::from(plain.replace('\\', "/").to_lowercase());
        assert_eq!(canonicalize(&git_style).unwrap(), canonical);
        let selected = canonicalize(&file).unwrap();
        assert!(contained(&selected, &canonicalize(&git_style).unwrap()));
        assert_eq!(
            selected.strip_prefix(&canonical).unwrap(),
            Path::new("file.txt")
        );
        assert_eq!(
            canonicalize(&git_style.join("new/file.txt")).unwrap(),
            canonical.join("new/file.txt")
        );
        assert!(!contained(
            &canonicalize(&directory.path().join("outside.txt")).unwrap(),
            &canonical
        ));
    }
}
pub fn contained(target: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    #[cfg(not(windows))]
    {
        target.starts_with(root)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CompareStringOrdinal(
                a: *const u16,
                na: i32,
                b: *const u16,
                nb: i32,
                ignore_case: i32,
            ) -> i32;
        }
        let target: Vec<_> = target.as_os_str().encode_wide().collect();
        let root: Vec<_> = root.as_os_str().encode_wide().collect();
        if target.len() < root.len() {
            return false;
        }
        let equal = unsafe {
            CompareStringOrdinal(
                target.as_ptr(),
                root.len() as i32,
                root.as_ptr(),
                root.len() as i32,
                1,
            )
        } == 2;
        equal
            && (target.len() == root.len()
                || root.last() == Some(&(b'\\' as u16))
                || target.get(root.len()) == Some(&(b'\\' as u16)))
    }
}
pub fn selected_path(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || Path::new(name).is_absolute()
    {
        bail!("name must be a basename");
    }
    let root = canonicalize(root)?;
    let target = canonicalize(&root.join(name))?;
    if !contained(&target, &root) {
        bail!("path escapes selected object");
    }
    Ok(target)
}

/// Git for Windows does not accept Rust's verbatim Win32 prefix as an argv
/// worktree path. This is only an external-command spelling; authorization and
/// stored directory identity must continue to use the canonical PathBuf.
pub(crate) fn git_argument(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Git path is not Unicode"))?;
    #[cfg(windows)]
    {
        if matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Verbatim(_) | std::path::Prefix::DeviceNS(_)))
        {
            bail!("Git cannot address this Windows device namespace");
        }
        let value = if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            value.strip_prefix(r"\\?\").unwrap_or(value).to_owned()
        };
        Ok(value.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    Ok(value.to_owned())
}

#[cfg(all(test, windows))]
mod git_argument_tests {
    use super::*;
    #[test]
    fn converts_only_windows_filesystem_spellings_for_git() {
        assert_eq!(
            git_argument(Path::new(r"\\?\C:\work\my repo\child")).unwrap(),
            "C:/work/my repo/child"
        );
        assert_eq!(
            git_argument(Path::new(r"\\?\UNC\server\share\child")).unwrap(),
            "//server/share/child"
        );
        assert_eq!(
            git_argument(Path::new(r"C:\work\child")).unwrap(),
            "C:/work/child"
        );
        assert!(git_argument(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume1")).is_err());
        assert!(git_argument(Path::new(r"\\.\pipe\fixture")).is_err());
    }
}
