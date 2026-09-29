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
