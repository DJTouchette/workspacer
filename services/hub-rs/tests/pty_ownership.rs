#![cfg(unix)]
use claudemon::{protocol::Signal, wrapper::pty};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn stopped(pid: i32) -> bool {
    #[cfg(target_os = "linux")]
    {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(") ")
                .is_some_and(|(_, tail)| matches!(tail.as_bytes().first(), Some(b'Z' | b'X'))),
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        (unsafe { libc::kill(pid, 0) }) != 0
    }
}
#[test]
fn owned_pty_kills_ignored_hup_descendant_before_reaping_anchor() {
    run_case(false);
}
#[test]
fn owned_pty_also_stops_its_interactive_foreground_job() {
    run_case(true);
}
fn run_case(interactive: bool) {
    struct Stop(Arc<pty::PtyHandle>);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = pty::signal_child(&self.0, Signal::Sigkill);
        }
    }
    let root = tempfile::tempdir().unwrap();
    let pids = root.path().join("pids");
    let script = "trap '' HUP; sh -c 'trap \"\" HUP; printf \"%s %s\\n\" \"$2\" \"$$\" > \"$1\"; printf ready > \"$1.ready\"; while :; do sleep 1; done' fixture \"$1\" \"$$\"; wait";
    let handle = Arc::new(
        pty::spawn(
            &[
                "/bin/sh".into(),
                if interactive {
                    "-ic".into()
                } else {
                    "-c".into()
                },
                script.into(),
                "fixture".into(),
                pids.to_string_lossy().into_owned(),
            ],
            root.path().to_str().unwrap(),
            Default::default(),
            &Default::default(),
        )
        .unwrap(),
    );
    let _stop = Stop(handle.clone());
    let deadline = Instant::now() + Duration::from_secs(3);
    let ids = loop {
        if let Ok(text) = std::fs::read_to_string(&pids) {
            let ids: Vec<i32> = text
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            if ids.len() == 2 && root.path().join("pids.ready").exists() {
                break ids;
            }
        }
        assert!(
            Instant::now() < deadline,
            "child did not publish identities"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let leader = ids[0];
    let descendant = ids[1];
    assert_ne!(leader, descendant, "fixture must contain a descendant");
    assert_ne!(leader, unsafe { libc::getpgrp() });
    assert_eq!(unsafe { libc::getsid(leader) }, leader);
    if interactive {
        assert_ne!(unsafe { libc::getpgid(descendant) }, leader);
    } else {
        assert_eq!(unsafe { libc::getpgid(descendant) }, leader);
    }
    assert!(!stopped(descendant));
    unsafe {
        libc::kill(descendant, libc::SIGHUP);
    };
    std::thread::sleep(Duration::from_millis(30));
    assert!(!stopped(descendant), "fixture must ignore HUP");
    let (tx, rx) = std::sync::mpsc::channel();
    let waiting = handle.clone();
    let waiter = std::thread::spawn(move || {
        tx.send(pty::wait_child(&waiting).is_ok()).unwrap();
    });
    let began = Instant::now();
    pty::signal_child(&handle, Signal::Sigkill).unwrap();
    assert!(began.elapsed() < Duration::from_secs(1));
    assert!(rx.recv_timeout(Duration::from_secs(3)).unwrap());
    waiter.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !stopped(descendant) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        stopped(descendant),
        "owned group descendant survived SIGKILL"
    );
    assert_ne!(
        unsafe { libc::kill(leader, 0) },
        0,
        "direct child must be reaped"
    );
    pty::signal_child(&handle, Signal::Sigkill).unwrap();
}

#[test]
fn externally_reaped_anchor_refuses_any_later_numeric_group_signal() {
    let root = tempfile::tempdir().unwrap();
    let handle = pty::spawn(
        &["/bin/sh".into(), "-c".into(), "exit 0".into()],
        root.path().to_str().unwrap(),
        Default::default(),
        &Default::default(),
    )
    .unwrap();
    let pid = handle.child.lock().unwrap().process_id().unwrap() as i32;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if result == pid {
            break;
        }
        assert_eq!(result, 0);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        pty::signal_child(&handle, Signal::Sigkill).is_err(),
        "an unowned/reaped PID must never authorize killpg"
    );
}
