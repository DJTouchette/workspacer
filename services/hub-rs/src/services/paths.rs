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
    Ok(resolved)
}
pub fn contained(target: &Path, root: &Path) -> bool {
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
