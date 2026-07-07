//! Transcript rendering primitives: the TUI's own markdown renderer and the
//! span-aware, display-width-aware wrap it (and the chat view) build on.

pub mod markdown;
pub mod wrap;

pub use markdown::markdown_lines;
pub use wrap::{truncate_width, wrap_plain};
