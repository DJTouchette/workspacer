//! Kill-on-close job ownership for plugin descendants on Windows. Assignment
//! can fail under restrictive parent jobs; the caller reports that limitation.
use anyhow::Result;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};
pub(crate) struct Job(usize);
impl Job {
    pub(crate) fn assign(child: &std::process::Child) -> Result<Self> {
        Self::assign_raw(child.as_raw_handle())
    }
    pub(crate) fn assign_raw(child: std::os::windows::io::RawHandle) -> Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let job = Self(handle as usize);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { AssignProcessToJobObject(handle, child) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(job)
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0 as _);
        }
    }
}
