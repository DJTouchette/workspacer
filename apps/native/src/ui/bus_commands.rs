//! Navigation requests are consumed only by the visible workspace. Unsupported
//! panes remain explicit; an event never substitutes an invisible backend job.
use super::*;
use wks_native::ui_requests::{Effect, Intent, effect};
#[derive(Default)]
pub(super) struct UiState {
    seen: u64,
    warning: String,
    pub notice: String,
    payload: Option<serde_json::Value>,
    guide: bool,
}
#[cfg(test)]
impl UiState {
    pub(super) fn payload_for_test(&mut self, payload: serde_json::Value) {
        self.payload = Some(payload);
    }
}
impl Workspace {
    pub(super) fn apply_ui_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui_bus.warning != self.view.ui_request_warning {
            self.ui_bus.warning = self.view.ui_request_warning.clone();
            if !self.ui_bus.warning.is_empty() {
                self.ui_bus.notice = self.ui_bus.warning.clone();
                self.ui_bus.payload = None;
                self.ui_bus.guide = false;
            }
        }
        if self.spawn_pending || self.view.creating {
            return;
        }
        let requests = self.view.ui_requests.clone();
        for request in requests {
            if request.number <= self.ui_bus.seen {
                continue;
            }
            if let Err(error) = self
                .controller
                .command(Command::ConsumeUiRequest(request.number))
            {
                self.ui_bus.notice = error.to_string();
                break;
            }
            self.ui_bus.seen = request.number;
            let previous = (
                self.ui_bus.notice.clone(),
                self.ui_bus.payload.clone(),
                self.ui_bus.guide,
            );
            self.ui_bus.payload = Some(request.payload.clone());
            self.ui_bus.guide = matches!(&request.intent, Intent::OpenGuide)
                || matches!(&request.intent,Intent::RunAction{action,..} if action=="toggle-help");
            let action = effect(&request.intent);
            if self.requested_session.is_some()
                && !matches!(action, Effect::Unsupported(_))
                && !matches!(&action,Effect::FocusAgent(id) if Some(id)==self.requested_session.as_ref())
            {
                self.ui_bus.notice =
                    "This window is pinned to a session; the navigation request was not applied."
                        .into();
                continue;
            }
            self.ui_bus.notice.clear();
            match action {
                Effect::Unsupported(message) => self.ui_bus.notice = message,
                Effect::FocusAgent(id) => {
                    if self.view.sessions.iter().any(|s| s.id == id) {
                        self.project_filter = None;
                        self.show_screen(Screen::Conversation, window, cx);
                        self.command(Command::Select(id), cx);
                    } else {
                        // The full ID stays in the copyable request payload.
                        self.ui_bus.notice = format!(
                            "Requested session {} is unavailable on this hub.",
                            wks_native::transcript::head(&id, 8)
                        );
                    }
                }
                Effect::Conversation => self.show_screen(Screen::Conversation, window, cx),
                Effect::Settings => self.show_screen(Screen::Settings, window, cx),
                Effect::Recent => self.open_feature(Screen::Recent, window, cx),
                Effect::Inspector => {
                    if self.selected_session().is_some() {
                        self.open_feature(Screen::Session, window, cx);
                    } else {
                        self.ui_bus.notice = "Select a session before opening its details.".into();
                    }
                }
                Effect::Changes(cwd) => {
                    let cwd = if cwd.is_empty() {
                        self.selected_session()
                            .map(|s| s.cwd.clone())
                            .unwrap_or_default()
                    } else {
                        cwd
                    };
                    if !wks_native::launch::absolute_directory(&cwd) {
                        self.ui_bus.notice =
                            "Changes requires an absolute project directory on the connected hub."
                                .into();
                        continue;
                    }
                    self.show_screen(Screen::Changes, window, cx);
                    self.request(wks_native::features::Request::Changes { cwd }, cx);
                }
                Effect::Spawn { cwd, claude } => {
                    if self.demo {
                        self.ui_bus.notice =
                            "Session creation is unavailable in the fixture workspace.".into();
                        continue;
                    }
                    self.show_new_session(window, cx);
                    if claude {
                        self.choose_provider("claude", window, cx);
                    }
                    if !cwd.is_empty() {
                        self.seed_project(&cwd, cx);
                        if !self.projects.picker_open {
                            self.prompt.update(cx, |input, cx| input.focus(window, cx));
                        }
                        self.load_models(false, cx);
                    }
                }
                Effect::Terminal { toggle } => {
                    if toggle || !self.terminal.open {
                        self.toggle_terminal(window, cx);
                    }
                }
                Effect::PreviousAgent => self.move_selection(-1, cx),
                Effect::NextAgent => self.move_selection(1, cx),
                Effect::NextAttention => {
                    let ids: Vec<_> = self
                        .visible_sessions(cx)
                        .into_iter()
                        .map(|i| &self.view.sessions[i])
                        .collect();
                    let start = ids
                        .iter()
                        .position(|s| Some(&s.id) == self.view.selected.as_ref())
                        .map(|n| n + 1)
                        .unwrap_or(0);
                    let next = (0..ids.len())
                        .map(|offset| ids[(start + offset) % ids.len()])
                        .find(|s| s.approval.is_some() || s.questions.is_some())
                        .map(|s| s.id.clone());
                    if let Some(id) = next {
                        self.show_screen(Screen::Conversation, window, cx);
                        self.command(Command::Select(id), cx);
                    } else {
                        self.ui_bus.notice = "No visible session is waiting for a decision.".into();
                    }
                }
            }
            if self.ui_bus.notice.is_empty() {
                (self.ui_bus.notice, self.ui_bus.payload, self.ui_bus.guide) = previous;
            }
        }
    }
    /// A hub navigation request the window could not apply: a quiet card in
    /// the sidebar with its tone icon, the request copy action and dismiss.
    pub(super) fn render_ui_notice(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let payload = self
            .ui_bus
            .payload
            .clone()
            .filter(|p| p.as_object().is_some_and(|p| !p.is_empty()));
        let tone = match chrome::notice_tone(&self.ui_bus.notice) {
            // Requests that could not be applied are not app failures.
            chrome::Tone::Error | chrome::Tone::Success => chrome::Tone::Warning,
            tone => tone,
        };
        div().px_3().pb_3().child(
            chrome::card(p)
                .debug_selector(|| "sidebar-ui-notice".into())
                .rounded(px(p.control_radius))
                .pl_3()
                .pr_1()
                .py_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pt_2()
                                .text_color(rgb(p.text))
                                .child(chrome::notice_line(
                                    self.ui_bus.notice.clone(),
                                    tone,
                                    p,
                                    "sidebar-ui-notice-icon",
                                )),
                        )
                        .child(
                            self.icon_button("dismiss-ui-request", "Dismiss", IconName::Close, true)
                                .debug_selector(|| "dismiss-ui-request".into())
                                .size(px(24.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.ui_bus.notice.clear();
                                    this.ui_bus.payload = None;
                                    cx.notify();
                                })),
                        ),
                )
                .when(payload.is_some() || self.ui_bus.guide, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            // Align the quiet buttons' text with the message
                            // past the tone icon.
                            .ml(px(13.))
                            .when_some(payload, |d, payload| {
                                d.child(
                                    self.quiet_button(
                                        "copy-ui-request",
                                        "Copy request",
                                        IconName::Copy,
                                        true,
                                    )
                                    .debug_selector(|| "copy-ui-request".into())
                                    .py_1()
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if let Ok(text) = serde_json::to_string_pretty(&payload) {
                                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                                        }
                                    })),
                                )
                            })
                            .when(self.ui_bus.guide, |d| {
                                d.child(
                                    self.quiet_button(
                                        "native-guide",
                                        "Read native guide",
                                        IconName::BookOpen,
                                        true,
                                    )
                                    .py_1()
                                    .on_click(|_, _, cx| {
                                        cx.open_url("https://github.com/DJTouchette/workspacer/blob/main/apps/native/README.md")
                                    }),
                                )
                            }),
                    )
                }),
        )
    }
}
