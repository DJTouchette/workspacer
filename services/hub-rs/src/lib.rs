//! Pure Rust control plane shared by standalone and embedded hosts.
//!
//! See MIGRATION.md for retained compatibility and cutover evidence.
mod admission;
pub mod auth;
pub mod backend;
pub mod cli;
pub mod client;
mod diagnostics;
pub mod federation;
pub mod mcp;
pub mod model_selection;
pub mod net_address;
pub mod plugins;
pub mod protocol;
mod runtime;
pub mod server;
pub mod services;
pub(crate) use runtime::LaunchPermit;
pub use runtime::{Caller, Connection, Handle, Hub, Options, Status};

pub mod provider_relay;

#[cfg(feature = "test-support")]
pub mod test_support;

pub(crate) mod state_loss;

/// Build-level cutover milestone; live readiness is reported separately.
pub const MIGRATION_COMPLETE: bool = true;
