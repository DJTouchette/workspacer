//! Per-child job ownership. The embedding process is never assigned to a job.
use anyhow::{bail, Result};
use std::os::windows::{
    io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    process::CommandExt,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            GetCurrentProcessId, GetProcessId, GetProcessIdOfThread, OpenThread, ResumeThread,
            TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
        },
    },
};
pub struct Job(usize);
impl Job {
    /// Spawn without running user code until a kill-on-close job owns the child.
    /// `flags` preserves the caller's explicit creation options (e.g. no window).
    pub fn spawn(
        command: &mut std::process::Command,
        flags: u32,
    ) -> Result<(std::process::Child, Self)> {
        Self::spawn_before_resume(command, flags, |_, _| Ok(()))
    }
    fn spawn_before_resume(
        command: &mut std::process::Command,
        flags: u32,
        before_resume: impl FnOnce(&std::process::Child, &Self) -> Result<()>,
    ) -> Result<(std::process::Child, Self)> {
        let mut child = command.creation_flags(flags | CREATE_SUSPENDED).spawn()?;
        let result = Self::assign(child.as_raw_handle()).and_then(|job| {
            before_resume(&child, &job)?;
            resume_initial_thread(child.as_raw_handle())?;
            Ok(job)
        });
        match result {
            Ok(job) => Ok((child, job)),
            Err(error) => {
                let stopped = terminate_failed_spawn(child.as_raw_handle());
                let _ = child.try_wait();
                if !stopped {
                    return Err(error.context("suspended child cleanup was not confirmed"));
                }
                Err(error)
            }
        }
    }
    pub fn spawn_tokio(
        command: &mut tokio::process::Command,
        flags: u32,
    ) -> Result<(tokio::process::Child, Self)> {
        let mut child = command.creation_flags(flags | CREATE_SUSPENDED).spawn()?;
        let result = child
            .raw_handle()
            .ok_or_else(|| anyhow::anyhow!("created child lacks a process handle"))
            .and_then(Self::assign_suspended);
        match result {
            Ok(job) => Ok((child, job)),
            Err(error) => {
                let stopped = child.raw_handle().is_some_and(terminate_failed_spawn);
                let _ = child.try_wait();
                if !stopped {
                    return Err(error.context("suspended child cleanup was not confirmed"));
                }
                Err(error)
            }
        }
    }
    /// The caller must create this child with CREATE_SUSPENDED. Used by the
    /// ConPTY adapter, whose process construction belongs to portable-pty.
    pub fn assign_suspended(child: RawHandle) -> Result<Self> {
        let job = Self::assign(child)?;
        resume_initial_thread(child)?;
        Ok(job)
    }
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
fn terminate_failed_spawn(child: RawHandle) -> bool {
    if unsafe { WaitForSingleObject(child, 0) } == WAIT_OBJECT_0 {
        return true;
    }
    if unsafe { TerminateProcess(child, 1) } == 0 {
        return false;
    }
    (unsafe { WaitForSingleObject(child, 2000) }) == WAIT_OBJECT_0
}
fn resume_initial_thread(child: RawHandle) -> Result<()> {
    let pid = unsafe { GetProcessId(child) };
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        bail!("invalid suspended child identity");
    }
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut present = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    let mut threads = Vec::new();
    while present != 0 {
        if entry.th32OwnerProcessID == pid {
            let raw = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                    0,
                    entry.th32ThreadID,
                )
            };
            if raw.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            threads.push(unsafe { OwnedHandle::from_raw_handle(raw) });
        }
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        present = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err(std::io::Error::last_os_error().into());
    }
    if threads.len() != 1 {
        bail!("suspended child has unexpected thread topology");
    }
    let thread = &threads[0];
    // Check the original process handle AFTER opening the thread handle. A
    // recycled numeric PID/TID cannot satisfy both this liveness check and the
    // opened thread's owner identity; handles never retarget on process exit.
    if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != pid
        || unsafe { WaitForSingleObject(child, 0) } != WAIT_TIMEOUT
    {
        bail!("suspended child identity changed before resume");
    }
    let previous = unsafe { ResumeThread(thread.as_raw_handle()) };
    if previous == u32::MAX {
        return Err(std::io::Error::last_os_error().into());
    }
    if previous != 1 {
        bail!("child initial thread was not singly suspended");
    }
    Ok(())
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
        let script = "$null=[Console]::ReadLine();$p=Start-Process cmd.exe -ArgumentList '/D','/C','ping -n 30 127.0.0.1 >nul' -PassThru;[Console]::WriteLine($p.Id);Start-Sleep -Seconds 30";
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
    #[test]
    fn immediate_fork_child_helper() {
        let Some(marker) = std::env::var_os("WORKSPACER_JOB_TEST_MARKER") else {
            return;
        };
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        std::fs::write(marker, child.id().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = child.kill();
        let _ = child.wait();
    }
    fn immediate_command(marker: &std::path::Path) -> std::process::Command {
        let module = module_path!().split_once("::").unwrap().1;
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                &format!("{module}::immediate_fork_child_helper"),
                "--nocapture",
            ])
            .env("WORKSPACER_JOB_TEST_MARKER", marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }
    fn descendant(marker: &std::path::Path) -> OwnedHandle {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(text) = std::fs::read_to_string(marker) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    let handle = unsafe {
                        windows_sys::Win32::System::Threading::OpenProcess(
                            windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
                            0,
                            pid,
                        )
                    };
                    assert!(!handle.is_null());
                    return unsafe { OwnedHandle::from_raw_handle(handle) };
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "immediate child did not publish its descendant"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn suspended_std_launch_owns_job_before_any_child_code_and_confines_immediate_fork() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant.pid");
        let mut command = immediate_command(&marker);
        let (mut child, job) = Job::spawn_before_resume(&mut command, 0, |child, job| {
            let mut child_in_job = 0;
            let mut host_in_job = 0;
            unsafe {
                assert_ne!(
                    windows_sys::Win32::System::JobObjects::IsProcessInJob(
                        child.as_raw_handle(),
                        job.0 as _,
                        &mut child_in_job
                    ),
                    0
                );
                assert_ne!(
                    windows_sys::Win32::System::JobObjects::IsProcessInJob(
                        windows_sys::Win32::System::Threading::GetCurrentProcess(),
                        job.0 as _,
                        &mut host_in_job
                    ),
                    0
                );
            }
            assert_ne!(child_in_job, 0);
            assert_eq!(host_in_job, 0);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(
                !marker.exists(),
                "user code executed before the job/resume boundary"
            );
            Ok(())
        })
        .unwrap();
        let descendant = descendant(&marker);
        drop(job);
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        child.wait().unwrap();
    }
    #[tokio::test]
    async fn suspended_tokio_launch_confines_an_immediate_fork_without_a_stdin_gate() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant.pid");
        let mut command = tokio::process::Command::from(immediate_command(&marker));
        let (mut child, job) = Job::spawn_tokio(&mut command, 0).unwrap();
        let descendant = descendant(&marker);
        drop(job);
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        child.wait().await.unwrap();
    }
}
