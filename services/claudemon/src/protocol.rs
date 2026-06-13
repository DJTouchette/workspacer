//! Wire protocol between the PTY wrapper and the daemon.
//!
//! A single bidirectional WebSocket carries JSON-tagged messages. Binary
//! payloads (PTY bytes, input) are base64-encoded to keep the channel a
//! simple text-frame stream, which makes everything debuggable with
//! `websocat` and tolerates intermediaries.

use serde::{Deserialize, Serialize};

/// Signal names that the daemon can ask the wrapper to deliver to the child
/// process.  The serialized form is UPPERCASE — byte-identical to the string
/// literals previously accepted as a plain `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Signal {
    Sigint,
    Sigterm,
    Sigkill,
}

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
        signal: Signal,
    },
    /// Daemon → wrapper: PTY size changed.
    Resize {
        cols: u16,
        rows: u16,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_serializes_to_uppercase_strings() {
        assert_eq!(
            serde_json::to_string(&Signal::Sigint).unwrap(),
            "\"SIGINT\""
        );
        assert_eq!(
            serde_json::to_string(&Signal::Sigterm).unwrap(),
            "\"SIGTERM\""
        );
        assert_eq!(
            serde_json::to_string(&Signal::Sigkill).unwrap(),
            "\"SIGKILL\""
        );
    }

    #[test]
    fn signal_round_trips() {
        for (json_str, expected) in [
            ("\"SIGINT\"", Signal::Sigint),
            ("\"SIGTERM\"", Signal::Sigterm),
            ("\"SIGKILL\"", Signal::Sigkill),
        ] {
            let parsed: Signal = serde_json::from_str(json_str).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(serde_json::to_string(&parsed).unwrap(), json_str);
        }
    }

    #[test]
    fn wrapper_message_signal_round_trips() {
        let msg = WrapperMessage::Signal {
            signal: Signal::Sigint,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"SIGINT\""), "wire form must be SIGINT: {json}");
        let parsed: WrapperMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            WrapperMessage::Signal { signal } => assert_eq!(signal, Signal::Sigint),
            other => panic!("expected Signal variant, got {other:?}"),
        }
    }
}
