//! The Windows update hand-off with real processes and the production helper
//! script in real Windows PowerShell.
//!
//! Copies of this test executable stand in for the app (`wks-native.exe` in an
//! install folder whose path has spaces and apostrophes), the NSIS installer,
//! and a process that keeps the install folder busy. Each case uses its own
//! temp folder; nothing touches the network, a provider, the user's
//! installation or the user's update state. Other platforms skip it.

fn main() {
    #[cfg(windows)]
    windows::main();
    #[cfg(not(windows))]
    println!("windows_update_handoff: skipped (Windows only)");
}

#[cfg(windows)]
mod windows {
    use serde_json::{Value, json};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};
    use wks_native::updates::{Handoff, Timeouts, hand_off};

    const VERSION: &str = "9.9.9";
    const PREVIOUS: &str = "9.9.8";
    const SETUP: &str = "Workspacer-Native-Rust-Preview-Setup-9.9.9-x64.exe";

    fn args() -> Vec<String> {
        vec![
            "--local".into(),
            r"C:\dir with space\".into(),
            r#"it's "quoted""#.into(),
            String::new(),
            "’smart’".into(),
        ]
    }

    pub fn main() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        if name.eq_ignore_ascii_case("wks-native.exe") {
            app();
        } else if name.starts_with("Workspacer-Native-Rust-Preview-Setup-") {
            installer();
        } else if name.eq_ignore_ascii_case("busy.exe") {
            std::thread::sleep(Duration::from_secs(120));
        } else {
            driver(&exe);
        }
    }

    fn root() -> PathBuf {
        std::env::var_os("WKS_FIXTURE_ROOT").unwrap().into()
    }

    fn timeouts() -> Timeouts {
        Timeouts {
            // A cold PowerShell start on a busy runner.
            ready: Duration::from_secs(60),
            app_exit: Duration::from_secs(300),
            siblings: Duration::from_secs(4),
            install: Duration::from_secs(60),
        }
    }

    /// The app: hand off once, then quit; when relaunched, record how.
    fn app() {
        let root = root();
        let launched = root.join("launched");
        if launched.exists() {
            let record = json!({
                "args": std::env::args().skip(1).collect::<Vec<_>>(),
                "cwd": std::env::current_dir().unwrap(),
            });
            std::fs::write(root.join("relaunched.json"), record.to_string()).unwrap();
            return;
        }
        std::fs::write(&launched, "").unwrap();
        std::fs::write(root.join("app-pid"), std::process::id().to_string()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.join("go").exists() {
            assert!(Instant::now() < deadline, "the driver never said go");
            std::thread::sleep(Duration::from_millis(50));
        }
        // The production plan for this process, with fixture state and timeouts.
        let mut handoff = Handoff::current(root.join("download").join(SETUP), VERSION).unwrap();
        handoff.state_dir = root.join("state");
        handoff.timeouts = timeouts();
        match hand_off(&handoff) {
            Ok(mut helper) => {
                if root.join("probe-duplicate").exists() {
                    assert!(
                        hand_off(&handoff).is_err(),
                        "a second helper must not arm another installation"
                    );
                    assert!(
                        helper.is_waiting().unwrap(),
                        "the rejected attempt must preserve the first receipt"
                    );
                    std::fs::write(root.join("duplicate-refused"), "").unwrap();
                }
                std::fs::write(root.join("handed-off"), "").unwrap();
                // Quitting takes a moment; the installer must not start sooner.
                std::thread::sleep(Duration::from_millis(1500));
            }
            Err(error) => {
                std::fs::write(root.join("handoff-error"), format!("{error:#}")).unwrap();
                std::process::exit(3);
            }
        }
    }

    /// The installer: record how it was started, then do what the case asks.
    fn installer() {
        let root = root();
        // NSIS consumes the entire raw command-line suffix after /D=, even
        // with spaces. CRT argv would split it and is not an installer oracle.
        let command_line = unsafe {
            let start = GetCommandLineW();
            let mut len = 0;
            while *start.add(len) != 0 {
                len += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(start, len)).unwrap()
        };
        let (prefix, directory) = command_line
            .rsplit_once(" /D=")
            .expect("NSIS directory argument");
        assert!(
            prefix.ends_with(" /S"),
            "silent installer flag missing: {command_line}"
        );
        let args = vec!["/S".to_owned(), format!("/D={directory}")];
        let app: u32 = std::fs::read_to_string(root.join("app-pid"))
            .unwrap()
            .parse()
            .unwrap();
        let record = json!({"args": args, "app_alive": alive(app)});
        std::fs::write(root.join("installer.json"), record.to_string()).unwrap();
        let stamp = std::env::var("WKS_FIXTURE_STAMP").unwrap_or_default();
        if !stamp.is_empty() {
            let dir = args.last().and_then(|a| a.strip_prefix("/D=")).unwrap();
            std::fs::write(
                Path::new(dir).join("build-stamp.json"),
                json!({ "version": stamp }).to_string(),
            )
            .unwrap();
        }
        let code = std::env::var("WKS_FIXTURE_EXIT").unwrap_or_default();
        std::process::exit(code.parse().unwrap_or(0));
    }

    type Handle = *mut core::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCommandLineW() -> *const u16;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateJobObjectW(attributes: *mut core::ffi::c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: u32,
            info: *const core::ffi::c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    }

    fn alive(pid: u32) -> bool {
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const WAIT_TIMEOUT: u32 = 0x102;
        unsafe {
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return false;
            }
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }

    /// A kill-on-close job, like a launcher or terminal can put the app in.
    struct Job(Handle);
    impl Job {
        fn new(breakaway: bool) -> Self {
            #[repr(C)]
            #[derive(Default)]
            struct Limits {
                per_process_user_time: i64,
                per_job_user_time: i64,
                flags: u32,
                minimum_working_set: usize,
                maximum_working_set: usize,
                active_processes: u32,
                affinity: usize,
                priority_class: u32,
                scheduling_class: u32,
                io: [u64; 6],
                process_memory: usize,
                job_memory: usize,
                peak_process_memory: usize,
                peak_job_memory: usize,
            }
            const KILL_ON_JOB_CLOSE: u32 = 0x2000;
            const BREAKAWAY_OK: u32 = 0x0800;
            const EXTENDED_LIMITS: u32 = 9;
            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                assert!(!job.is_null(), "CreateJobObject failed");
                let limits = Limits {
                    flags: KILL_ON_JOB_CLOSE | if breakaway { BREAKAWAY_OK } else { 0 },
                    ..Default::default()
                };
                assert_ne!(
                    SetInformationJobObject(
                        job,
                        EXTENDED_LIMITS,
                        &limits as *const _ as *const core::ffi::c_void,
                        std::mem::size_of::<Limits>() as u32,
                    ),
                    0,
                    "SetInformationJobObject failed"
                );
                Self(job)
            }
        }
        fn assign(&self, child: &Child) {
            assert_ne!(
                unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as Handle) },
                0,
                "AssignProcessToJobObject failed"
            );
        }
    }
    impl Drop for Job {
        /// Closing the last handle ends every process still in the job.
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    struct Case {
        root: PathBuf,
        install: PathBuf,
        work: PathBuf,
    }

    impl Case {
        fn new(driver: &Path, name: &str) -> Self {
            // Cargo's per-target temp folder: a long path, never an 8.3 alias.
            let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "wks update o'neil ’q’ {name} {}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let install = root.join("Programs").join("Workspacer Native Rust Preview");
            let work = root.join("work dir");
            for dir in [&install, &work, &root.join("download")] {
                std::fs::create_dir_all(dir).unwrap();
            }
            std::fs::copy(driver, install.join("wks-native.exe")).unwrap();
            std::fs::copy(driver, root.join("download").join(SETUP)).unwrap();
            std::fs::write(
                install.join("build-stamp.json"),
                json!({ "version": PREVIOUS }).to_string(),
            )
            .unwrap();
            Self {
                root,
                install,
                work,
            }
        }

        /// Run the app until it quits, optionally inside a job that is closed
        /// the moment the app exits.
        fn run_app(&self, exit: i32, stamp: &str, job: Option<Job>) -> std::process::ExitStatus {
            let mut app = Command::new(self.install.join("wks-native.exe"))
                .args(args())
                .current_dir(&self.work)
                .env("WKS_FIXTURE_ROOT", &self.root)
                .env("WKS_FIXTURE_EXIT", exit.to_string())
                .env("WKS_FIXTURE_STAMP", stamp)
                .spawn()
                .unwrap();
            if let Some(job) = &job {
                job.assign(&app);
            }
            std::fs::write(self.root.join("go"), "").unwrap();
            let deadline = Instant::now() + Duration::from_secs(120);
            let status = loop {
                if let Some(status) = app.try_wait().unwrap() {
                    break status;
                }
                assert!(Instant::now() < deadline, "the app never quit");
                std::thread::sleep(Duration::from_millis(50));
            };
            drop(job);
            status
        }

        fn read(&self, name: &str) -> Option<Value> {
            serde_json::from_slice(&std::fs::read(self.root.join(name)).ok()?).ok()
        }

        fn state(&self) -> Value {
            self.read("state/last-update.json").unwrap_or(Value::Null)
        }

        fn log(&self) -> String {
            let mut log = std::fs::read_to_string(self.root.join("state/last-update.log"))
                .unwrap_or_default();
            for file in std::fs::read_dir(self.root.join("state"))
                .into_iter()
                .flatten()
                .flatten()
            {
                if file.file_name().to_string_lossy().ends_with("-output.log") {
                    log.push_str(&std::fs::read_to_string(file.path()).unwrap_or_default());
                }
            }
            log
        }

        fn wait_relaunch(&self) -> Value {
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                if let Some(relaunch) = self.read("relaunched.json") {
                    return relaunch;
                }
                assert!(
                    Instant::now() < deadline,
                    "no relaunch; state {} log:\n{}",
                    self.state(),
                    self.log()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        fn stamp(&self) -> String {
            let stamp: Value = serde_json::from_slice(
                &std::fs::read(self.install.join("build-stamp.json")).unwrap(),
            )
            .unwrap();
            stamp["version"].as_str().unwrap().to_owned()
        }

        fn assert_relaunched_like_the_original(&self) {
            let relaunch = self.wait_relaunch();
            assert_eq!(relaunch["args"], json!(args()), "original arguments");
            assert_eq!(
                Path::new(relaunch["cwd"].as_str().unwrap()),
                self.work,
                "original working directory"
            );
        }

        fn assert_installed_after_exit(&self) {
            let installer = self.read("installer.json").expect("the installer ran");
            let expected = format!("/D={}", self.install.display());
            assert_eq!(
                installer["args"],
                json!(["/S", expected]),
                "silent, in place"
            );
            assert_eq!(installer["app_alive"], false, "only after the app exited");
        }
    }

    impl Drop for Case {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn installs_and_relaunches_even_when_the_app_job_is_closed(driver: &Path) {
        let case = Case::new(driver, "success");
        std::fs::write(case.root.join("probe-duplicate"), "").unwrap();
        let status = case.run_app(0, VERSION, Some(Job::new(true)));
        assert!(status.success(), "app: {status} {}", case.log());
        assert!(case.root.join("handed-off").exists());
        assert!(case.root.join("duplicate-refused").exists());
        case.assert_relaunched_like_the_original();
        case.assert_installed_after_exit();
        let state = case.state();
        assert_eq!(state["state"], "succeeded", "{}", case.log());
        assert_eq!(state["expected"], VERSION);
        assert_eq!(case.stamp(), VERSION);
        assert!(
            !case.root.join("download").join(SETUP).exists(),
            "installer removed"
        );
        let log = case.log();
        assert!(log.contains("job breakaway: true"), "{log}");
        assert!(log.contains("installer exited with code 0"), "{log}");
    }

    fn installer_failure_is_recorded_and_the_previous_version_relaunched(driver: &Path) {
        let case = Case::new(driver, "installer-fails");
        assert!(case.run_app(2, "", None).success());
        case.assert_relaunched_like_the_original();
        case.assert_installed_after_exit();
        let state = case.state();
        assert_eq!(state["state"], "failed");
        assert!(
            state["detail"]
                .as_str()
                .unwrap()
                .contains("exited with code 2"),
            "{state}"
        );
        assert_eq!(case.stamp(), PREVIOUS);
        assert!(
            case.root.join("download").join(SETUP).exists(),
            "kept for a manual run"
        );
    }

    fn a_wrong_installed_version_is_not_success(driver: &Path) {
        let case = Case::new(driver, "wrong-version");
        assert!(case.run_app(0, "9.9.7", None).success());
        case.assert_relaunched_like_the_original();
        let state = case.state();
        assert_eq!(state["state"], "failed");
        assert!(
            state["detail"]
                .as_str()
                .unwrap()
                .contains("9.9.7 instead of 9.9.9"),
            "{state}"
        );
    }

    fn a_busy_install_folder_is_never_overwritten(driver: &Path) {
        let case = Case::new(driver, "busy");
        std::fs::copy(driver, case.install.join("busy.exe")).unwrap();
        let mut busy = Command::new(case.install.join("busy.exe")).spawn().unwrap();
        assert!(case.run_app(0, VERSION, None).success());
        case.assert_relaunched_like_the_original();
        let _ = busy.kill();
        let _ = busy.wait();
        assert!(
            case.read("installer.json").is_none(),
            "installer must not run"
        );
        let state = case.state();
        assert_eq!(state["state"], "failed");
        assert!(
            state["detail"].as_str().unwrap().contains("busy"),
            "{state}"
        );
        assert_eq!(case.stamp(), PREVIOUS);
    }

    fn a_helper_that_cannot_start_keeps_the_app_open(driver: &Path) {
        let case = Case::new(driver, "no-helper");
        let mut handoff =
            Handoff::current(case.root.join("download").join(SETUP), VERSION).unwrap();
        handoff.state_dir = case.root.join("state");
        handoff.timeouts = timeouts();
        let missing = Handoff {
            installer: case.root.join("missing.exe"),
            ..handoff.clone()
        };
        assert!(
            hand_off(&missing)
                .err()
                .unwrap()
                .is::<wks_native::updates::MissingInstaller>()
        );
        handoff.powershell = case.root.join("missing").join("powershell.exe");
        assert!(hand_off(&handoff).is_err(), "a missing helper is an error");
        // A helper that starts but cannot wait for its app reports failure
        // before it is ready, well inside the ready timeout.
        let mut gone = Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/c", "exit 0"])
            .spawn()
            .unwrap();
        let gone_pid = gone.id();
        gone.wait().unwrap();
        handoff.powershell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into();
        handoff.pid = gone_pid;
        let started = Instant::now();
        let error = format!("{:#}", hand_off(&handoff).err().unwrap());
        assert!(
            started.elapsed() < Duration::from_secs(55),
            "reported promptly"
        );
        assert!(error.contains("helper"), "{error}");
        assert_eq!(case.state()["state"], "failed", "{}", case.log());
        assert!(case.read("installer.json").is_none());
    }

    /// Refuse before reporting ready when the launcher would kill the helper
    /// with the app. The real UI stays open on this error; this fixture exits 3.
    fn a_job_that_forbids_breakaway_is_refused(driver: &Path) {
        let case = Case::new(driver, "no-breakaway");
        assert_eq!(
            case.run_app(0, VERSION, Some(Job::new(false))).code(),
            Some(3)
        );
        assert!(!case.root.join("handed-off").exists());
        assert!(case.read("relaunched.json").is_none());
        assert!(case.read("installer.json").is_none());
        let error = std::fs::read_to_string(case.root.join("handoff-error")).unwrap();
        assert!(error.contains("independently"), "{error}");
    }

    type Check = (&'static str, fn(&Path));

    fn driver(exe: &Path) {
        let cases: [Check; 6] = [
            (
                "installs_and_relaunches_even_when_the_app_job_is_closed",
                installs_and_relaunches_even_when_the_app_job_is_closed,
            ),
            (
                "installer_failure_is_recorded_and_the_previous_version_relaunched",
                installer_failure_is_recorded_and_the_previous_version_relaunched,
            ),
            (
                "a_wrong_installed_version_is_not_success",
                a_wrong_installed_version_is_not_success,
            ),
            (
                "a_busy_install_folder_is_never_overwritten",
                a_busy_install_folder_is_never_overwritten,
            ),
            (
                "a_helper_that_cannot_start_keeps_the_app_open",
                a_helper_that_cannot_start_keeps_the_app_open,
            ),
            (
                "a_job_that_forbids_breakaway_is_refused",
                a_job_that_forbids_breakaway_is_refused,
            ),
        ];
        // `cargo test` passes harness flags; a name filters cases.
        let filter: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| !a.starts_with('-'))
            .collect();
        let (mut failed, mut ran) = (Vec::new(), 0);
        for (name, case) in cases {
            if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            ran += 1;
            let started = Instant::now();
            match std::panic::catch_unwind(|| case(exe)) {
                Ok(()) => println!("test {name} ... ok ({:?})", started.elapsed()),
                Err(_) => {
                    println!("test {name} ... FAILED");
                    failed.push(name);
                }
            }
        }
        println!(
            "windows_update_handoff: {} passed, {} failed, {} filtered out",
            ran - failed.len(),
            failed.len(),
            cases.len() - ran
        );
        // An unfiltered run that checked nothing is not a pass.
        if !failed.is_empty() || (filter.is_empty() && ran != cases.len()) {
            eprintln!("failed: {failed:?}");
            std::process::exit(1);
        }
    }
}
