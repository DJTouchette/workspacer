//! Launcher-only parent-death cooperation. No blocking reader threads survive
//! shutdown; stdin is probed before a bounded read, alongside the parent PID.
use anyhow::Result;
pub(super) async fn gone() -> Result<()> {
    let Some(raw) = std::env::var_os("WORKSPACER_PARENT_PID").filter(|v| !v.is_empty()) else {
        return std::future::pending().await;
    };
    let pid = raw
        .to_str()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|p| *p > 0);
    #[cfg(windows)]
    let parent = match pid {
        Some(pid) => match ParentHandle::open(pid) {
            Ok(parent) => Some(parent),
            Err(_) => return Ok(()),
        },
        None => None,
    };
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        #[cfg(unix)]
        {
            if let Some(pid) = pid {
                let status = unsafe { libc::kill(pid as libc::pid_t, 0) };
                if status < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
                {
                    return Ok(());
                }
            }
            let mut event = libc::pollfd {
                fd: 0,
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut event, 1, 0) };
            if ready > 0 {
                if event.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                    return Ok(());
                }
                if event.revents & libc::POLLIN != 0 {
                    let mut bytes = [0u8; 4096];
                    let count = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
                    if count <= 0 {
                        return Ok(());
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            use std::{io::Read, os::windows::io::AsRawHandle};
            use windows_sys::Win32::{
                Foundation::{ERROR_BROKEN_PIPE, ERROR_INVALID_HANDLE, WAIT_TIMEOUT},
                System::{Pipes::PeekNamedPipe, Threading::WaitForSingleObject},
            };
            if let Some(parent) = &parent {
                if unsafe { WaitForSingleObject(parent.0 as _, 0) } != WAIT_TIMEOUT {
                    return Ok(());
                }
            }
            let mut available = 0;
            let ok = unsafe {
                PeekNamedPipe(
                    std::io::stdin().as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let code = std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or_default() as u32;
                if code == ERROR_BROKEN_PIPE || code == ERROR_INVALID_HANDLE {
                    return Ok(());
                }
            } else if available > 0 {
                let mut bytes = [0u8; 4096];
                if std::io::stdin().read(&mut bytes[..(available as usize).min(4096)])? == 0 {
                    return Ok(());
                }
            }
        }
    }
}
#[cfg(windows)]
struct ParentHandle(usize);
#[cfg(windows)]
impl ParentHandle {
    fn open(pid: u32) -> Result<Self> {
        let handle = unsafe {
            windows_sys::Win32::System::Threading::OpenProcess(
                windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(handle as usize))
    }
}
#[cfg(windows)]
impl Drop for ParentHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
