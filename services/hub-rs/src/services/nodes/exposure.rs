//! Three-valued inspection: inability to read/classify an ACL is never a claim
//! that a credential file is private. Windows mode bits are not ACL evidence.
use std::path::Path;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    Unknown,
    OwnerOnly,
    Loose,
}
impl Exposure {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown; owner-only access could not be established",
            Self::OwnerOnly => "owner-only",
            Self::Loose => "loose; principals beyond the owner can read it",
        }
    }
}
#[cfg(windows)]
#[path = "exposure_windows.rs"]
mod windows;
#[cfg(windows)]
pub fn file(path: &Path) -> Exposure {
    windows::file(path)
}
#[cfg(unix)]
pub fn file(path: &Path) -> Exposure {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.permissions().mode() & 0o077 == 0 => Exposure::OwnerOnly,
        Ok(_) => Exposure::Loose,
        Err(_) => Exposure::Unknown,
    }
}
#[cfg(not(any(unix, windows)))]
pub fn file(_: &Path) -> Exposure {
    Exposure::Unknown
}
