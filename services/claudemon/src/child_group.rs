//! Signal a group while its owner still holds the unreaped direct-child anchor.
//! Callers must establish the group/session relationship before using this API.
use nix::libc;
use std::io;

pub fn signal(group: i32, signal: i32) -> io::Result<()> {
    if group <= 1 || group == unsafe { libc::getpgrp() } {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing unowned process group",
        ));
    }
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    // XNU killpg1 excludes SZOMB entries, then returns EPERM if no eligible
    // member remains. An unreaped anchor is intentionally such a zombie.
    // EPERM can ALSO mean a live process denied the signal: never ignore it
    // without a complete process-group snapshot proving every member exited.
    // https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::EPERM) && only_zombies(group) {
        return Ok(());
    }
    Err(error)
}

/// Verify the original, still-unreaped child owns a distinct group. The result
/// reports whether it has already exited; it does not assert a historical SID.
#[cfg(target_os = "macos")]
pub fn verify_anchor(pid: u32) -> io::Result<bool> {
    if pid <= 1 || pid > i32::MAX as u32 || pid as i32 == unsafe { libc::getpgrp() } {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid child group anchor",
        ));
    }
    let mut exited = observe_exit(pid)?;
    if unsafe { libc::getpgid(pid as i32) } == pid as i32 {
        return Ok(exited);
    }
    // XNU getpgid/getsid omit zombies. A process can exit between spawn's
    // exec acknowledgement and this check, or between the two observations.
    if !exited {
        exited = observe_exit(pid)?;
    }
    if exited {
        if let Some(info) = process_info(pid as i32) {
            if info.pbi_pid == pid
                && info.pbi_ppid == unsafe { libc::getpid() } as u32
                && info.pbi_pgid == pid
                && info.pbi_status == libc::SZOMB
            {
                return Ok(true);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "child does not own a verified separate process group",
    ))
}
#[cfg(target_os = "macos")]
fn observe_exit(pid: u32) -> io::Result<bool> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}
#[cfg(target_os = "macos")]
fn process_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    // Full BSD info is available in claudemon's independently locked libc
    // 0.2.186 as well as the hub/native lock. The short-info Rust bindings were
    // only added later; do not accidentally require a different crate graph.
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // arg=1 explicitly includes zombie records, unlike the default lookup.
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            1,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (got == size).then(|| unsafe { info.assume_init() })
}

#[cfg(target_os = "macos")]
fn only_zombies(group: i32) -> bool {
    // proc_listpids includes both allproc and zombproc. A full buffer is an
    // incomplete observation, never evidence that the remainder has exited.
    const PROC_PGRP_ONLY: u32 = 2; // <sys/proc_info.h>
    let mut pids = vec![0i32; 4096];
    let capacity = std::mem::size_of_val(pids.as_slice());
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_PGRP_ONLY,
            group as u32,
            pids.as_mut_ptr().cast(),
            capacity as i32,
        )
    };
    if bytes <= 0
        || bytes as usize >= capacity
        || !(bytes as usize).is_multiple_of(std::mem::size_of::<i32>())
    {
        return false;
    }
    pids.truncate(bytes as usize / std::mem::size_of::<i32>());
    pids.into_iter().all(|pid| {
        if pid <= 0 {
            return false;
        }
        let Some(info) = process_info(pid) else {
            return false;
        };
        info.pbi_pid == pid as u32
            && info.pbi_pgid == group as u32
            && info.pbi_status == libc::SZOMB
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn own_application_group_is_never_a_cleanup_target() {
        for group in [-1, 0, 1, unsafe { libc::getpgrp() }] {
            assert_eq!(
                signal(group, 0).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn zombie_anchor_is_successful_cleanup_but_live_group_is_not_empty() {
        use std::{
            os::unix::process::CommandExt,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read line; exit 0"])
            .stdin(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        assert!(!only_zombies(pid));
        drop(child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            assert_eq!(
                unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        info.as_mut_ptr(),
                        libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                    )
                },
                0
            );
            if unsafe { info.assume_init().si_pid() } != 0 {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not exit");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(only_zombies(pid));
        signal(pid, libc::SIGKILL).unwrap();
        assert!(child.wait().unwrap().success());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn already_exited_anchor_is_verified_but_reaped_and_inherited_groups_are_refused() {
        use std::{
            os::unix::process::CommandExt,
            process::Command,
            time::{Duration, Instant},
        };
        for separate in [true, false] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "exit 0"]);
            if separate {
                command.process_group(0);
            }
            let mut child = command.spawn().unwrap();
            let pid = child.id();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !observe_exit(pid).unwrap() {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not exit");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if separate {
                assert!(verify_anchor(pid).unwrap());
                signal(pid as i32, libc::SIGKILL).unwrap();
            } else {
                assert!(verify_anchor(pid).is_err());
            }
            assert!(child.wait().unwrap().success());
            assert!(
                verify_anchor(pid).is_err(),
                "reaped PID cannot regain authority"
            );
        }
    }
}
