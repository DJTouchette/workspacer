//! Compile the actual Windows policy/process primitives without native GUI or
//! SQLite C build dependencies. This is a type check, not a Windows test run.
#![cfg(windows)]
#![allow(dead_code)]
#[path = "../../../services/hub-rs/src/services/task_store/directory.rs"]
mod directory;
#[path = "../../../services/hub-rs/src/services/routing/path.rs"]
mod path;
#[path = "../../../services/hub-rs/src/plugins/process_logs.rs"]
mod process_logs;
#[path = "../../../services/hub-rs/src/services/routing/windows_audit.rs"]
mod windows_audit;
#[path = "../../../services/hub-rs/src/plugins/windows_job.rs"]
mod windows_job;

// Exercise the plugin alias's actual launch API rather than suppressing an
// unused-import warning in this deliberately narrow source probe.
const _: fn(
    &mut std::process::Command,
    u32,
) -> anyhow::Result<(std::process::Child, windows_job::Job)> = windows_job::Job::spawn;
const _: fn(
    &mut tokio::process::Command,
    u32,
) -> anyhow::Result<(tokio::process::Child, windows_job::Job)> = windows_job::Job::spawn_tokio;

#[path = "../../../services/hub-rs/src/cli/parent.rs"]
mod cli_parent;

#[path = "../../../services/hub-rs/src/plugins/launch/windows_command.rs"]
mod plugin_windows_command;

mod services;

#[path = "../../../services/hub-rs/src/services/filewatch/sample.rs"]
mod filewatch_sample;

#[path = "../../../services/hub-rs/src/services/atomic_persist.rs"]
mod atomic_persist;
#[path = "../../../services/hub-rs/src/services/worktrees/dependency_junctions.rs"]
mod dependency_junctions;
#[path = "../../../services/claudemon/src/protocol.rs"]
mod protocol;
#[path = "../../../services/claudemon/src/wrapper/pty.rs"]
mod pty;
#[path = "../../../services/claudemon/src/daemon/pty_output.rs"]
mod pty_output;

#[path = "../../../services/hub-rs/src/services/nodes/exposure.rs"]
mod node_exposure;

extern crate self as claudemon;
#[path = "../../../services/claudemon/src/background_process.rs"]
pub mod background_process;
#[path = "../../../services/claudemon/src/child_env.rs"]
pub mod child_env;
#[path = "../../../services/claudemon/src/wrapper/pty_windows_job.rs"]
pub mod child_job;

#[path = "../../../services/hub-rs/src/services/owned_process.rs"]
mod owned_process;

#[path = "../../../services/hub-rs/src/services/account_setup/platform.rs"]
mod account_links;

#[path = "../../../services/hub-rs/src/services/task_store/project.rs"]
mod task_project;

#[path = "../../../services/hub-rs/src/cli/install.rs"]
mod cli_install;

#[cfg(test)]
#[path = "../../../apps/native/tests/background_process.rs"]
mod background_process_tests;

#[cfg(test)]
mod background_capture_tests {
    #[tokio::test]
    async fn owned_capture_does_not_create_console_and_preserves_pipes() {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "background_process_tests::console_fixture",
                "--nocapture",
            ])
            .env("WKS_BACKGROUND_CONSOLE_FIXTURE", "1");
        let output = super::owned_process::capture_input(
            &mut command,
            b"pipe-input",
            4096,
            4096,
            std::time::Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("BACKGROUND_PIPE_REPLY=pipe-input")
        );
    }
}
