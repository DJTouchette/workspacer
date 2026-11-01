//! The hub's "too many sessions" prompt (docs/session-roster.md §6).
//!
//! Nothing leaves the sidebar on its own, so the hub says when it needs a
//! cleanup (`cleanup.suggested` in the `sessionOpen` document). The TUI shows a
//! one-line notice in the footer and a small chooser (`<leader> C`) with the
//! same three actions as every other client. A cleanup archives, so hidden
//! sessions stay in History; `<leader> u` undoes it for a few seconds.

use super::*;
use crate::open_sessions::{DAY_MS, UNDO_WINDOW};
use crossterm::event::{KeyCode, KeyEvent};

/// One of the chooser's actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupChoice {
    /// Hide every counted ended session.
    All,
    /// Hide ended sessions whose ended mark is at least a day old.
    OlderThanDay,
    /// Not now: the hub holds the prompt back until more have ended.
    Snooze,
}

impl App {
    /// The prompt to show, if the hub suggests one.
    pub fn cleanup_offer(&self) -> Option<crate::open_sessions::Cleanup> {
        self.open_sessions.as_ref().and_then(|o| o.cleanup_offer())
    }

    /// The ids the last cleanup hid, while its Undo is still offered.
    pub fn cleanup_undo(&self) -> Option<&[String]> {
        self.cleanup_undo
            .as_ref()
            .filter(|(_, at)| at.elapsed() < UNDO_WINDOW)
            .map(|(ids, _)| ids.as_slice())
    }

    /// `<leader> C`: open the chooser when the hub suggests a cleanup.
    pub(in crate::app) fn open_cleanup(&mut self) {
        if self.bus.is_none() {
            self.set_toast("sidebar cleanup needs the hub bus");
        } else if self.cleanup_offer().is_some() {
            self.cleanup_chooser = true;
        } else {
            self.set_toast("the sidebar does not need a cleanup");
        }
    }

    /// The chooser owns every key while it is open.
    pub(in crate::app) fn handle_cleanup_key(&mut self, key: KeyEvent) {
        let choice = match key.code {
            KeyCode::Char('a') => Some(CleanupChoice::All),
            KeyCode::Char('o') => Some(CleanupChoice::OlderThanDay),
            KeyCode::Char('n') => Some(CleanupChoice::Snooze),
            KeyCode::Esc | KeyCode::Char('q') => None,
            _ => return,
        };
        self.cleanup_chooser = false;
        if let Some(choice) = choice {
            self.run_cleanup(choice);
        }
    }

    fn run_cleanup(&self, choice: CleanupChoice) {
        let Some(bus) = self.bus.clone() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match choice {
                CleanupChoice::Snooze => {
                    match bus
                        .call("sessionOpen.snoozeCleanup", serde_json::json!({}))
                        .await
                    {
                        Ok(doc) => match crate::open_sessions::OpenSessions::from_document(&doc) {
                            Some(set) => AppMsg::OpenSessions(set),
                            None => AppMsg::Toast("cleanup snoozed".into()),
                        },
                        Err(e) => AppMsg::Toast(format!("could not snooze: {e}")),
                    }
                }
                CleanupChoice::All | CleanupChoice::OlderThanDay => {
                    let params = if choice == CleanupChoice::OlderThanDay {
                        serde_json::json!({ "olderThanMs": DAY_MS })
                    } else {
                        serde_json::json!({})
                    };
                    match bus.call("sessionOpen.cleanup", params).await {
                        Ok(r) => AppMsg::CleanupDone(hidden_ids(&r)),
                        Err(e) => AppMsg::Toast(format!("cleanup failed: {e}")),
                    }
                }
            };
            let _ = tx.send(msg);
        });
    }

    /// A cleanup landed: offer its Undo for a few seconds.
    pub(in crate::app) fn cleanup_done(&mut self, hidden: Vec<String>) {
        if hidden.is_empty() {
            self.set_toast("nothing to hide");
            return;
        }
        let n = hidden.len();
        self.cleanup_undo = Some((hidden, Instant::now()));
        let undo = self
            .keymap
            .chord_for(crate::keys::Action::UndoCleanup)
            .map(|k| format!(" · {k} undo"))
            .unwrap_or_default();
        self.set_toast(format!(
            "hid {n} ended session{} (still in History){undo}",
            if n == 1 { "" } else { "s" }
        ));
    }

    /// `<leader> u`: restore what the last cleanup hid, in one archive change.
    pub(in crate::app) fn undo_cleanup(&mut self) {
        let Some(ids) = self.cleanup_undo().map(<[String]>::to_vec) else {
            self.set_toast("nothing to undo");
            return;
        };
        self.cleanup_undo = None;
        let Some(bus) = self.bus.clone() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let n = ids.len();
            let params = serde_json::json!({ "sessionIds": ids, "archived": false });
            let msg = match bus.call("sessionArchive.setMany", params).await {
                Ok(_) => format!("restored {n} session{}", if n == 1 { "" } else { "s" }),
                Err(e) => format!("undo failed: {e}"),
            };
            let _ = tx.send(AppMsg::Toast(msg));
        });
    }
}

/// The ids a `sessionOpen.cleanup` answer says it hid.
pub(in crate::app) fn hidden_ids(answer: &serde_json::Value) -> Vec<String> {
    answer
        .get("hidden")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .map(String::from)
        .collect()
}
