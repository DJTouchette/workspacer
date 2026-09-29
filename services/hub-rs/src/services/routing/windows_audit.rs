//! Windows audit privacy is a protected owner/SYSTEM/Administrators DACL,
//! installed atomically at creation and repaired on the same opened handle.
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SE_FILE_OBJECT, SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
        PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_ALWAYS, READ_CONTROL, WRITE_DAC,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct LocalAllocation(*mut std::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}
fn last_error() -> anyhow::Error {
    std::io::Error::last_os_error().into()
}
fn descriptor() -> Result<LocalAllocation> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error()).context("open current token");
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut length,
        );
    }
    if length == 0 {
        return Err(last_error()).context("read token user length");
    }
    let words = (length as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; words];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(last_error()).context("read token user");
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(last_error()).context("encode token user SID");
    }
    let sid_owner = LocalAllocation(sid.cast());
    let mut count = 0;
    while unsafe { *sid.add(count) } != 0 {
        count += 1;
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid, count) })?;
    drop(sid_owner);
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})(A;;GA;;;SY)(A;;GA;;;BA)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(last_error()).context("create protected audit descriptor");
    }
    Ok(LocalAllocation(descriptor))
}
fn repair(file: &File, descriptor: &LocalAllocation) -> Result<()> {
    let (mut present, mut defaulted) = (0, 0);
    let mut dacl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(last_error()).context("read private DACL");
    }
    if present == 0 || dacl.is_null() {
        bail!("private audit descriptor has no DACL");
    }
    let error = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null(),
        )
    };
    if error != 0 {
        return Err(std::io::Error::from_raw_os_error(error as i32).into());
    }
    Ok(())
}
fn create(path: &Path, descriptor: &LocalAllocation) -> Result<File> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if name[..name.len() - 1].contains(&0) {
        bail!("audit path contains NUL");
    }
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_APPEND_DATA | WRITE_DAC | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attrs,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error()).context("open private audit log");
    }
    let file = unsafe { File::from_raw_handle(handle) };
    Ok(file)
}
pub(super) fn open(path: &Path) -> Result<File> {
    let descriptor = descriptor()?;
    let file = create(path, &descriptor)?;
    repair(&file, &descriptor)?;
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo,
    };
    use windows_sys::Win32::Security::{GetSecurityDescriptorControl, SE_DACL_PROTECTED};
    fn file_descriptor(file: &File) -> LocalAllocation {
        let mut descriptor = ptr::null_mut();
        let error = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(error, 0);
        LocalAllocation(descriptor)
    }
    fn dacl_text(descriptor: &LocalAllocation) -> String {
        let mut text = ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor.0,
                    1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    ptr::null_mut(),
                )
            },
            0
        );
        let owner = LocalAllocation(text.cast());
        let mut len = 0;
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        let out = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) }).unwrap();
        drop(owner);
        out
    }
    fn grants(descriptor: &LocalAllocation) -> Vec<(String, u32)> {
        use windows_sys::Win32::{
            Security::{ACCESS_ALLOWED_ACE, GENERIC_MAPPING, GetAce, MapGenericMask},
            Storage::FileSystem::{
                FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            },
        };
        let (mut present, mut defaulted) = (0, 0);
        let mut acl = ptr::null_mut();
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
            },
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null(), "null DACL grants unrestricted access");
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ,
            GenericWrite: FILE_GENERIC_WRITE,
            GenericExecute: FILE_GENERIC_EXECUTE,
            GenericAll: FILE_ALL_ACCESS,
        };
        let mut grants = Vec::new();
        for index in 0..unsafe { (*acl).AceCount } {
            let mut raw = ptr::null_mut();
            assert_ne!(unsafe { GetAce(acl, u32::from(index), &mut raw) }, 0);
            let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
            assert_eq!(
                ace.Header.AceType, 0,
                "only ACCESS_ALLOWED_ACE entries expected"
            );
            assert_eq!(
                ace.Header.AceFlags, 0,
                "no inherited or propagation ACEs permitted"
            );
            let mut mask = ace.Mask;
            unsafe { MapGenericMask(&mut mask, &mapping) };
            assert_eq!(mask, FILE_ALL_ACCESS);
            let sid = ptr::addr_of!(ace.SidStart).cast_mut().cast();
            let mut text = ptr::null_mut();
            assert_ne!(unsafe { ConvertSidToStringSidW(sid, &mut text) }, 0);
            let allocation = LocalAllocation(text.cast());
            let mut len = 0;
            while unsafe { *text.add(len) } != 0 {
                len += 1;
            }
            let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) }).unwrap();
            drop(allocation);
            grants.push((sid, mask));
        }
        grants.sort();
        grants
    }
    fn assert_private(file: &File, expected: &LocalAllocation) {
        let descriptor = file_descriptor(file);
        let (mut control, mut revision) = (0, 0);
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) },
            0
        );
        assert_ne!(control & SE_DACL_PROTECTED, 0);
        // Windows expands generic-all to file-all and may retain the harmless
        // AUTO_INHERITED descriptor bit. Compare concrete rights and exact SIDs,
        // while separately requiring protection and non-inherited ACEs.
        let actual = grants(&descriptor);
        assert_eq!(
            actual.len(),
            3,
            "only current user, SYSTEM and Administrators"
        );
        assert_eq!(actual, grants(expected));
    }
    #[test]
    fn creates_with_private_dacl_before_any_repair() {
        let dir = tempfile::tempdir().unwrap();
        let expected = descriptor().unwrap();
        let file = create(&dir.path().join("new.jsonl"), &expected).unwrap();
        assert_private(&file, &expected);
    }
    #[test]
    fn repairs_an_existing_everyone_dacl_before_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loose.jsonl");
        let file = open(&path).unwrap();
        let loose: Vec<u16> = "D:(A;;GA;;;WD)".encode_utf16().chain(Some(0)).collect();
        let mut sd = ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    loose.as_ptr(),
                    1,
                    &mut sd,
                    ptr::null_mut(),
                )
            },
            0
        );
        let loose = LocalAllocation(sd);
        repair(&file, &loose).unwrap();
        assert!(dacl_text(&file_descriptor(&file)).contains(";;;WD)"));
        drop(file);
        let repaired = open(&path).unwrap();
        assert_private(&repaired, &descriptor().unwrap());
    }
}
