//! Capture identity while observing the file, including Windows replacement
//! identity. No delayed path-based lookup and no file contents are retained.
use anyhow::Result;
use std::path::Path;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub identity: (u64, u64),
    pub size: u64,
    pub modified: u128,
    pub mode: u32,
}
#[cfg(unix)]
pub fn sample(path: &Path) -> Result<Sample> {
    use std::os::unix::fs::MetadataExt;
    let value = std::fs::metadata(path)?;
    Ok(Sample {
        identity: (value.dev(), value.ino()),
        size: value.len(),
        modified: ((value.mtime() as i128) * 1_000_000_000 + value.mtime_nsec() as i128) as u128,
        mode: value.mode(),
    })
}
#[cfg(windows)]
pub fn sample(path: &Path) -> Result<Sample> {
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetFileInformationByHandle, OPEN_EXISTING,
        },
    };
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    anyhow::ensure!(
        !name[..name.len() - 1].contains(&0),
        "file path contains NUL"
    );
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
    Ok(Sample {
        identity: (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ),
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        modified: ((info.ftLastWriteTime.dwHighDateTime as u128) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u128,
        mode: info.dwFileAttributes,
    })
}
#[cfg(not(any(unix, windows)))]
pub fn sample(_path: &Path) -> Result<Sample> {
    anyhow::bail!("file identity unavailable on this platform")
}
