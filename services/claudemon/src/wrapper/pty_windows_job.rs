//! A job belongs to one spawned PTY, never to the embedding UI process.
use anyhow::{bail, Result};
use std::os::windows::io::RawHandle;
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{GetCurrentProcessId, GetProcessId},
    },
};
pub struct Job(usize);
impl Job {
    pub fn assign(child: RawHandle) -> Result<Self> {
        let pid = unsafe { GetProcessId(child) };
        if pid == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if pid == unsafe { GetCurrentProcessId() } {
            bail!("refusing to confine the embedding process to a PTY job");
        }
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
    pub fn terminate(&self) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.0 as _, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0 as _);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_process_is_never_assigned() {
        assert!(
            Job::assign(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
                .is_err()
        );
    }
    #[test]
    fn job_close_terminates_assigned_child() {
        use std::os::windows::io::AsRawHandle;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        let job = match Job::assign(child.as_raw_handle()) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("could not assign child job: {error}");
            }
        };
        drop(job);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("PTY job failed to terminate its child");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    #[test]
    fn job_close_terminates_a_descendant_created_after_assignment() {
        use std::{
            io::{BufRead, Write},
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::Stdio,
        };
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let script="$null=[Console]::ReadLine();$p=Start-Process cmd.exe -ArgumentList '/D','/C','ping -n 30 127.0.0.1 >nul' -PassThru;[Console]::WriteLine($p.Id);Start-Sleep -Seconds 30";
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let job = match Job::assign(child.as_raw_handle()) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("could not assign child job: {error}");
            }
        };
        child.stdin.take().unwrap().write_all(b"go\n").unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = std::io::BufReader::new(stdout)
                .read_line(&mut line)
                .map(|_| line);
            let _ = tx.send(result);
        });
        let pid: u32 = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("descendant ready")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        reader.join().unwrap();
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null(), "open original descendant handle");
        let descendant = unsafe { OwnedHandle::from_raw_handle(raw) };
        drop(job);
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "owned descendant survived job close"
        );
        child.wait().unwrap();
    }
}
