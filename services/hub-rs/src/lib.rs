//! Pure Rust control plane shared by standalone and embedded hosts.
//!
//! This migration crate is not yet a replacement for the production Go stack.
//! See MIGRATION.md for the compatibility and deletion gates.
mod admission;
pub mod auth;
pub mod backend;
pub mod client;
pub mod cli;
pub mod federation;
pub mod mcp;
pub mod model_selection;
pub mod net_address;
pub mod protocol;
pub mod plugins;
mod runtime;
pub mod server;
pub mod services;
pub use runtime::{Caller, Connection, Handle, Hub, Options, Status};
pub(crate) use runtime::LaunchPermit;

pub mod provider_relay;

#[cfg(feature = "test-support")]
pub mod test_support;
