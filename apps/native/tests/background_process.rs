use claudemon::background_process::BackgroundCommand;
#[cfg(windows)]
use claudemon::background_process::CREATION_FLAGS;
use std::{
    io::{Read, Write},
    process::Stdio,
};

#[test]
fn console_fixture() {
    if std::env::var("WKS_BACKGROUND_CONSOLE_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        assert!(
            unsafe { GetConsoleWindow() }.is_null(),
            "background child acquired a console window"
        );
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    assert_eq!(input, "pipe-input");
    println!("BACKGROUND_PIPE_REPLY=pipe-input");
}

fn command() -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    let fixture = format!("{}::console_fixture", module_path!())
        .split_once("::")
        .unwrap()
        .1
        .to_owned();
    command
        .args(["--exact", &fixture, "--nocapture"])
        .env("WKS_BACKGROUND_CONSOLE_FIXTURE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn verify(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("BACKGROUND_PIPE_REPLY=pipe-input"));
}

#[test]
fn background_std_child_retains_piped_io_without_console() {
    let mut child = command().no_console_window().spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"pipe-input")
        .unwrap();
    verify(child.wait_with_output().unwrap());
}

#[tokio::test]
async fn background_tokio_child_retains_piped_io_without_console() {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::from(command())
        .no_console_window()
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"pipe-input")
        .await
        .unwrap();
    verify(child.wait_with_output().await.unwrap());
}

#[cfg(windows)]
#[tokio::test]
async fn background_suspended_job_retains_piped_io_without_console() {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::from(command());
    let (mut child, _job) =
        claudemon::child_job::Job::spawn_tokio(&mut command, CREATION_FLAGS).unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"pipe-input")
        .await
        .unwrap();
    verify(child.wait_with_output().await.unwrap());
}

#[cfg(windows)]
#[test]
fn background_std_suspended_job_retains_piped_io_without_console() {
    let mut command = command();
    let (mut child, _job) = claudemon::child_job::Job::spawn(&mut command, CREATION_FLAGS).unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"pipe-input")
        .unwrap();
    verify(child.wait_with_output().unwrap());
}
