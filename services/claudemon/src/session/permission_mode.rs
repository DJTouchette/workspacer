//! Live permission-mode switching for PTY-backed Claude sessions.
//!
//! Claude Code has no slash command or CLI flag to change the permission mode
//! of a *running* session — the TUI cycles modes with Shift+Tab and shows the
//! result as a footer marker ("⏵⏵ accept edits on", "⏸ plan mode on",
//! "⏵⏵ bypass permissions on"; the default mode shows no marker). The daemon
//! owns the PTY byte stream, so it can do what a human does: press Shift+Tab,
//! watch the footer, and stop when the target mode is showing. The screen is
//! reconstructed on demand from the session's output ring buffer with `vt100`
//! (see `SessionStore::set_permission_mode` for the press-and-verify loop).

use serde::{Deserialize, Serialize};

use super::state::SessionMode;

/// The four Claude Code permission modes, using the CLI's own spelling
/// (`--permission-mode` values / hook `permission_mode` payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
}

impl PermissionMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "acceptEdits" => Some(Self::AcceptEdits),
            "plan" => Some(Self::Plan),
            "bypassPermissions" => Some(Self::BypassPermissions),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::BypassPermissions => "bypassPermissions",
        }
    }
}

/// How many footer rows to scan for the mode marker. The marker sits directly
/// under the composer; scanning only the bottom of the screen keeps
/// conversation text that merely *mentions* a mode from being misread.
const FOOTER_ROWS: usize = 6;

/// Classify the current permission mode from rendered screen text (rows as
/// `\n`-separated lines, e.g. `vt100::Screen::contents()`). Only the bottom
/// [`FOOTER_ROWS`] rows are considered. No marker → `Default`.
pub fn classify_screen(contents: &str) -> PermissionMode {
    let rows: Vec<&str> = contents.lines().collect();
    let start = rows.len().saturating_sub(FOOTER_ROWS);
    let footer = rows[start..].join("\n").to_lowercase();
    if footer.contains("bypass permissions on") {
        PermissionMode::BypassPermissions
    } else if footer.contains("accept edits on") {
        PermissionMode::AcceptEdits
    } else if footer.contains("plan mode on") {
        PermissionMode::Plan
    } else {
        PermissionMode::Default
    }
}

/// Why a live permission-mode switch could not be performed. Keeping the
/// policy out of the HTTP handler mirrors [`super::store::MessageOutcome`].
#[derive(Debug, PartialEq, Eq)]
pub enum PermissionSwitchError {
    /// No session with this id.
    NoSession,
    /// Session exists but has no wrapper attached to receive input.
    NoWrapper,
    /// Managed (adapter-driven) session with no live-switchable policy
    /// registered — the provider (or its fallback transport) freezes its
    /// permission policy at spawn; restart to change it.
    Managed,
    /// The session is paused on a dialog (`Approval`/`Question`) or not yet
    /// past cold start (`Unknown`) — Shift+Tab there could act on the dialog
    /// instead of the mode cycle. `Stopped` also lands here.
    Busy(SessionMode),
    /// Cycled all the way around without the target appearing — the mode is
    /// not in this session's Shift+Tab cycle (e.g. `bypassPermissions` when
    /// not enabled). The session is back at the mode it started in.
    Unavailable(PermissionMode),
    /// A press produced no observable footer change — the TUI ignored the
    /// keystroke or is not redrawing. Reports the last mode seen.
    Unverified(PermissionMode),
    /// Managed session whose live policy can't reach the requested mode —
    /// e.g. codex yolo→ask when the TUI was spawned with the bypass flag
    /// (approvals are skipped at the source, so un-skipping them needs a
    /// restart). Reports the mode the session stays in.
    ManagedUnavailable { current: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(footer: &str) -> String {
        // 20 rows of conversation, then the footer line.
        let mut s = "conversation text\n".repeat(20);
        s.push_str(footer);
        s
    }

    #[test]
    fn parse_round_trips() {
        for id in ["default", "acceptEdits", "plan", "bypassPermissions"] {
            assert_eq!(PermissionMode::parse(id).unwrap().as_str(), id);
        }
        assert_eq!(PermissionMode::parse("yolo"), None);
    }

    #[test]
    fn classifies_footer_markers() {
        assert_eq!(
            classify_screen(&screen("⏵⏵ accept edits on (shift+tab to cycle)")),
            PermissionMode::AcceptEdits
        );
        assert_eq!(
            classify_screen(&screen("⏸ plan mode on (shift+tab to cycle)")),
            PermissionMode::Plan
        );
        assert_eq!(
            classify_screen(&screen("⏵⏵ bypass permissions on (shift+tab to cycle)")),
            PermissionMode::BypassPermissions
        );
        assert_eq!(
            classify_screen(&screen("? for shortcuts")),
            PermissionMode::Default
        );
        assert_eq!(classify_screen(""), PermissionMode::Default);
    }

    #[test]
    fn conversation_mentions_above_the_footer_do_not_count() {
        let mut s = "user: how do I turn plan mode on?\n".to_string();
        s.push_str(&"conversation text\n".repeat(20));
        s.push_str("? for shortcuts");
        assert_eq!(classify_screen(&s), PermissionMode::Default);
    }
}
