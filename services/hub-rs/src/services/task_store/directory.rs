//! Durable identity of an already canonicalized directory; paths alone never
//! authorize opening a historical worktree after deletion or replacement.
use anyhow::{Result, bail};
use std::path::Path;
#[cfg(unix)]
pub(super) fn identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path)?;
    if !meta.is_dir() {
        bail!("Recorded worktree is not a directory");
    }
    Ok((meta.dev(), meta.ino()))
}
#[cfg(windows)]
pub(super) fn identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
        },
    };
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if name[..name.len() - 1].contains(&0) {
        bail!("Directory path contains NUL");
    }
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!("Recorded worktree is not a directory");
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
#[cfg(not(any(unix, windows)))]
pub(super) fn identity(_path: &Path) -> Result<(u64, u64)> {
    bail!("directory identity unavailable on this platform")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_distinguishes_replaced_directory() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("worktree");
        std::fs::create_dir(&path).unwrap();
        let before = identity(&path).unwrap();
        std::fs::rename(&path, parent.path().join("old")).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_ne!(identity(&path).unwrap(), before);
    }
}
