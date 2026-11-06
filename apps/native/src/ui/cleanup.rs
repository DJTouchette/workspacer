//! The hub's "too many ended sessions" prompt (docs/session-roster.md §6).
//!
//! Nothing leaves the sidebar on its own: a session stays until it is hidden
//! (archived). When ended sessions pile up, the hub sets `cleanup.suggested`
//! in its shared open document and every client shows the same prompt. Its
//! actions are hub calls, so the count, the snooze and what was hidden agree
//! across clients. A cleanup offers a short Undo that restores exactly the ids
//! the hub reported hiding. Hidden sessions stay in History.
use super::*;
use wks_native::features::Request;

/// How long the Undo after a cleanup stays up.
pub(super) const UNDO_MS: i64 = 8_000;
/// "Hide ended older than a day".
pub(super) const DAY_MS: u64 = 86_400_000;

#[derive(Default)]
pub(super) struct CleanupUi {
    /// The last `session-cleanup` / `session-undo` replies handled, by
    /// request number, so each lands once.
    seen_cleanup: u64,
    seen_undo: u64,
    /// Ids the last cleanup hid, offered back until `until` (ms).
    pub(super) undo: Option<Undo>,
    expiry: Option<gpui::Task<()>>,
}

pub(super) struct Undo {
    pub(super) ids: Vec<String>,
    pub(super) until: i64,
}

impl Workspace {
    /// How many ended sessions the hub's prompt names, while it suggests a
    /// cleanup and none of its actions is on its way. A hub without the
    /// prompt (no `cleanup` block) shows nothing.
    pub(super) fn cleanup_prompt(&self) -> Option<u64> {
        if self.demo || !self.view.connected {
            return None;
        }
        let ended = self
            .view
            .session_open
            .as_deref()
            .and_then(wks_native::features::cleanup_suggested)?;
        let busy = ["session-cleanup", "session-snooze"]
            .iter()
            .any(|key| self.view.requests.get(key).is_some_and(|s| s.loading));
        (!busy).then_some(ended)
    }

    /// The Undo for the last cleanup, while it is still offered.
    pub(super) fn cleanup_undo(&self) -> Option<&Undo> {
        self.cleanup
            .undo
            .as_ref()
            .filter(|undo| timing::now_ms() < undo.until)
    }

    /// Settle cleanup and undo replies: a cleanup that hid something offers
    /// its Undo; a failure says so.
    pub(super) fn sync_cleanup(&mut self, view: &View, cx: &mut Context<Self>) {
        if let Some(state) = view.requests.get("session-cleanup")
            && !state.loading
            && state.number > self.cleanup.seen_cleanup
        {
            self.cleanup.seen_cleanup = state.number;
            match &state.error {
                None => {
                    let ids: Vec<String> = state.value["hidden"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect();
                    if !ids.is_empty() {
                        self.offer_undo(ids, cx);
                    }
                }
                Some(error) => {
                    self.extras.notice = format!("Could not hide ended sessions: {error}")
                }
            }
        }
        if let Some(state) = view.requests.get("session-undo")
            && !state.loading
            && state.number > self.cleanup.seen_undo
        {
            self.cleanup.seen_undo = state.number;
            if let Some(error) = &state.error {
                self.extras.notice = format!("Could not bring the sessions back: {error}");
            }
        }
    }

    fn offer_undo(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.cleanup.undo = Some(Undo {
            ids,
            until: timing::now_ms() + UNDO_MS,
        });
        // Redraw when the offer lapses, so the Undo leaves on its own.
        self.cleanup.expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(UNDO_MS as u64))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .cleanup
                    .undo
                    .as_ref()
                    .is_some_and(|undo| timing::now_ms() >= undo.until)
                {
                    this.cleanup.undo = None;
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn run_cleanup(&mut self, older_than_ms: Option<u64>, cx: &mut Context<Self>) {
        self.cleanup.undo = None;
        if let Err(error) = self
            .controller
            .command(Command::Request(Request::CleanupSessions { older_than_ms }))
        {
            self.extras.notice = error.to_string();
        }
        cx.notify();
    }

    pub(super) fn snooze_cleanup(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self
            .controller
            .command(Command::Request(Request::SnoozeCleanup))
        {
            self.extras.notice = error.to_string();
        }
        cx.notify();
    }

    pub(super) fn undo_cleanup(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.cleanup.undo.take() else {
            return;
        };
        self.cleanup.expiry = None;
        if let Err(error) = self
            .controller
            .command(Command::Request(Request::UndoCleanup {
                sessions: undo.ids,
            }))
        {
            self.extras.notice = error.to_string();
        }
        cx.notify();
    }

    /// The prompt, or the Undo after a cleanup, below the sidebar search.
    pub(super) fn render_cleanup_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let p = self.palette();
        let card = || {
            div()
                .mx_3()
                .mb_2()
                .px_3()
                .py_2()
                .rounded(px(p.control_radius))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .flex()
                .flex_col()
                .gap_1()
        };
        if let Some(undo) = self.cleanup_undo() {
            let n = undo.ids.len();
            let text = if n == 1 {
                "Hid 1 ended session".to_owned()
            } else {
                format!("Hid {n} ended sessions")
            };
            return Some(
                card()
                    .debug_selector(|| "cleanup-undo".into())
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(chrome::notice_line(
                        text,
                        chrome::Tone::Success,
                        p,
                    )))
                    .child(
                        self.quiet_button("cleanup-undo-button", "Undo", IconName::Undo, true)
                            .debug_selector(|| "cleanup-undo-button".into())
                            .on_click(cx.listener(|this, _, _, cx| this.undo_cleanup(cx))),
                    ),
            );
        }
        let ended = self.cleanup_prompt()?;
        let text = if ended == 1 {
            "1 ended session on your sidebar".to_owned()
        } else {
            format!("{ended} ended sessions on your sidebar")
        };
        Some(
            card()
                .debug_selector(|| "cleanup-banner".into())
                .child(chrome::notice_line(text, chrome::Tone::Info, p))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(
                            self.quiet_button(
                                "cleanup-all",
                                "Hide all ended",
                                IconName::EyeOff,
                                true,
                            )
                            .debug_selector(|| "cleanup-all".into())
                            .on_click(cx.listener(|this, _, _, cx| this.run_cleanup(None, cx))),
                        )
                        .child(
                            self.quiet_button(
                                "cleanup-older",
                                "Hide ended older than a day",
                                IconName::Calendar,
                                true,
                            )
                            .debug_selector(|| "cleanup-older".into())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.run_cleanup(Some(DAY_MS), cx)),
                            ),
                        )
                        .child(
                            self.quiet_button("cleanup-snooze", "Not now", IconName::Close, true)
                                .debug_selector(|| "cleanup-snooze".into())
                                .on_click(cx.listener(|this, _, _, cx| this.snooze_cleanup(cx))),
                        ),
                ),
        )
    }
}
