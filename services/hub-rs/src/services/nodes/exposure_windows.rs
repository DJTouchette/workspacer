use super::Exposure;
use std::{
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_ALL, GENERIC_READ, HANDLE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, GetAce, GetTokenInformation, IsValidSid,
        OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::FILE_READ_DATA,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Allocation(*mut std::ffi::c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}
fn sid(pointer: *mut std::ffi::c_void) -> Option<String> {
    if pointer.is_null() || unsafe { IsValidSid(pointer) } == 0 {
        return None;
    }
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(pointer, &mut text) } == 0 {
        return None;
    }
    let _allocation = Allocation(text.cast());
    let mut length = 0;
    while length < 1024 && unsafe { *text.add(length) } != 0 {
        length += 1
    }
    if length == 1024 {
        return None;
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).ok()
}
fn self_sid() -> Option<String> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
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
    if length == 0 || length > 64 * 1024 {
        return None;
    }
    let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
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
        return None;
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid(user.User.Sid)
}
pub(super) fn file(path: &Path) -> Exposure {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut owner = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Exposure::Unknown;
    }
    let _descriptor = Allocation(descriptor);
    if dacl.is_null() {
        return Exposure::Loose;
    }
    let owner = sid(owner);
    let current = self_sid();
    let mut denied = std::collections::BTreeSet::new();
    let mut unknown = false;
    for index in 0..unsafe { (*dacl).AceCount } {
        let mut raw = ptr::null_mut();
        if unsafe { GetAce(dacl, index.into(), &mut raw) } == 0 || raw.is_null() {
            return Exposure::Unknown;
        }
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if header.AceFlags & 8 != 0 {
            continue;
        }
        if !matches!(header.AceType, 0 | 1)
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            unknown = true;
            continue;
        }
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        if ace.Mask & (FILE_READ_DATA | GENERIC_READ | GENERIC_ALL) == 0 {
            continue;
        }
        let principal = unsafe {
            std::ptr::addr_of!((*raw.cast::<ACCESS_ALLOWED_ACE>()).SidStart)
                .cast_mut()
                .cast()
        };
        let Some(sid) = sid(principal) else {
            unknown = true;
            continue;
        };
        if header.AceType == 1 {
            denied.insert(sid);
            continue;
        }
        if denied.contains(&sid) {
            continue;
        }
        if matches!(
            sid.as_str(),
            "S-1-1-0"
                | "S-1-2-0"
                | "S-1-5-2"
                | "S-1-5-4"
                | "S-1-5-7"
                | "S-1-5-11"
                | "S-1-5-113"
                | "S-1-5-32-545"
                | "S-1-5-32-546"
                | "S-1-5-32-547"
        ) {
            return Exposure::Loose;
        }
        if owner.as_ref() == Some(&sid)
            || current.as_ref() == Some(&sid)
            || matches!(
                sid.as_str(),
                "S-1-5-18"
                    | "S-1-5-19"
                    | "S-1-5-20"
                    | "S-1-5-32-544"
                    | "S-1-5-114"
                    | "S-1-3-0"
                    | "S-1-3-4"
            )
        {
            continue;
        }
        unknown = true;
    }
    if unknown {
        Exposure::Unknown
    } else {
        Exposure::OwnerOnly
    }
}
