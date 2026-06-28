//! claudemon — library surface used by the binary and by integration tests.
//!
//! The binary at `src/main.rs` is a thin entry point; the real code lives
//! in these modules. Exposing them via `lib.rs` lets us write integration
//! tests (`tests/*.rs`) and visual previews without redundantly duplicating
//! file references.

pub mod classifier;
pub mod cli;
pub mod daemon;
pub mod protocol;
pub mod providers;
pub mod session;
pub mod store;
pub mod tui;
pub mod wrapper;
