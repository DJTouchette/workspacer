//! Wire protocol between the PTY wrapper and the daemon.
//!
//! A single bidirectional WebSocket carries JSON-tagged messages. Binary
//! payloads (PTY bytes, input) are base64-encoded to keep the channel a
//! simple text-frame stream, which makes everything debuggable with
//! `websocat` and tolerates intermediaries.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WrapperMessage {
    /// First message after the WS upgrade — wrapper introduces itself.
    Register {
        session_id: String,
        cwd: String,
        argv: Vec<String>,
        cols: u16,
        rows: u16,
    },
    /// Wrapper → daemon: chunk of bytes the child wrote to its PTY.
    Output {
        /// base64-encoded
        bytes: String,
    },
    /// Wrapper → daemon: child has exited.
    Exited {
        code: Option<i32>,
    },
    /// Daemon → wrapper: write these bytes to the child's stdin.
    Input {
        /// base64-encoded
        bytes: String,
    },
    /// Daemon → wrapper: deliver a signal.
    Signal {
        signal: String,
    },
    /// Daemon → wrapper: PTY size changed.
    Resize {
        cols: u16,
        rows: u16,
    },
}
