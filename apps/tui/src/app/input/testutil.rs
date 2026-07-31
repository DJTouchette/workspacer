//! Shared fixtures for the input-dispatch tests. These drive real
//! `KeyEvent`s through [`App::handle_key`] and assert on the synchronous
//! state left behind; async effects (network sends) are fire-and-forget
//! against a dead port, so only local state is asserted.

use super::*;
use crate::claudemon::Claudemon;
use crate::config::Config;
use crate::types::Agent;

pub(super) fn test_app() -> App {
    // Redirect the config dir to a per-process temp dir: harpoon/rename/notes
    // dispatch persists to disk, and tests must never touch the real files.
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("wks-tui-input-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);
    });
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
    // An unused port: the background stream tasks fail and retry harmlessly.
    let cm = Claudemon::new("http://127.0.0.1:59999".into());
    App::new(cm, Vec::new(), Vec::new(), Config::default(), tx, ptx)
}

pub(super) fn agent(id: &str, mode: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "cwd": format!("/work/{id}"), "mode": mode
    }))
    .unwrap()
}

/// An agent parked on a structured question (so `has_question()` is true).
pub(super) fn agent_asking(id: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
        "pending": {"kind": "question", "questions": [
            {"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}
        ]}
    }))
    .unwrap()
}

/// An agent parked on a single multi-select question.
pub(super) fn agent_asking_multiselect(id: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
        "pending": {"kind": "question", "questions": [
            {"question": "Choose", "multi_select": true,
             "options": [{"label": "X"}, {"label": "Y"}]}
        ]}
    }))
    .unwrap()
}

/// An agent on a three-question set: a pick, a multi-select, and free text.
pub(super) fn agent_asking_many(id: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
        "pending": {"kind": "question", "questions": [
            {"question": "Pick one",
             "options": [{"label": "A"}, {"label": "B"}]},
            {"question": "Choose tools", "multi_select": true,
             "options": [{"label": "X"}, {"label": "Y"}, {"label": "Z"}]},
            {"question": "Anything else?", "options": []}
        ]}
    }))
    .unwrap()
}

pub(super) fn ch(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

pub(super) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub(super) fn code(k: KeyCode) -> KeyEvent {
    KeyEvent::new(k, KeyModifiers::NONE)
}

/// Feed each character of `s` as a separate key press.
pub(super) fn feed(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(ch(c));
    }
}

/// A list app with `n` agents and the first one selected (row 1).
pub(super) fn app_with_agents(n: usize) -> App {
    let mut app = test_app();
    app.set_agents(
        (1..=n)
            .map(|i| agent(&format!("s{i}"), "responding"))
            .collect(),
    );
    app.selected = 1;
    app
}

// ── normal-mode key → Action resolution ─────────────────────────────────

/// An agent view with `n` agents, the first one open as a single tile.
pub(super) async fn app_in_agent_view(n: usize) -> App {
    let mut app = app_with_agents(n);
    app.open_single("s1".into(), true);
    app
}
