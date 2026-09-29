//! SDK/linker-free type check of the exact process-ownership source and old ABI.
#[cfg(not(target_os = "macos"))]
compile_error!("run this probe with --target aarch64-apple-darwin or x86_64-apple-darwin");
extern crate self as nix;
pub extern crate libc;
#[path = "../../services/claudemon/src/child_group.rs"]
pub mod child_group;
