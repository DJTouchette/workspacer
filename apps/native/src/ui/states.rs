//! Honest loading and recovery feedback; actions reuse the controller contracts.
use super::*;

pub(super) struct ConnectionCopy {
    pub label: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub animated: bool,
}

impl Workspace {
    pub(super) fn connection_copy(&self) -> ConnectionCopy {
        let (label, title, description, animated) = if self.view.connected {
            (
                "Connected",
                "Workspace connected",
                "Your sessions are up to date.",
                false,
            )
        } else if self.view.power_paused {
            (
                "Paused",
                "Connection paused",
                if self.view.can_resume_power_pause {
                    "Reconnect when you’re ready. This may wake the workspace."
                } else {
                    "Restart the local workspace through its owning host when you’re ready."
                },
                false,
            )
        } else if self.extras.local_paths {
            if self.view.notice.is_empty() || self.view.notice.starts_with("Starting Rust backend")
            {
                (
                    "Starting…",
                    "Starting your workspace",
                    "Preparing your local workspace. Sessions will appear when it’s ready.",
                    true,
                )
            } else {
                (
                    "Unavailable",
                    "Workspace unavailable",
                    "The local workspace could not stay connected. Restart the app to try again.",
                    false,
                )
            }
        } else if self.has_connected {
            (
                "Reconnecting…",
                "Reconnecting to your workspace",
                "We’ll reconnect automatically. Your draft and saved conversation remain here.",
                true,
            )
        } else {
            (
                "Connecting…",
                "Connecting to your workspace",
                "Waiting for the workspace to connect. Check that it’s running and reachable.",
                true,
            )
        };
        ConnectionCopy {
            label,
            title,
            description,
            animated,
        }
    }

    fn wake_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        self.button("wake-workspace", "Reconnect and wake", true)
            .debug_selector(|| "wake-workspace".into())
            .bg(rgb(self.appearance.palette().primary))
            .text_color(rgb(self.appearance.palette().on_primary))
            .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
    }

    pub(super) fn render_connection_banner(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let copy = self.connection_copy();
        div()
            .debug_selector(|| "connection-banner".into())
            .occlude()
            .rounded(px(8.))
            .bg(rgb(p.surface))
            .p_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .child(if copy.animated {
                brand_spinner(12., p, "connection-banner-spinner")
            } else {
                status_dot(p.warning)
            })
            .child(
                div().flex_1().min_w_0().child(copy.title).child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(copy.description),
                ),
            )
            .when(
                self.view.power_paused && self.view.can_resume_power_pause,
                |d| d.child(self.wake_button(cx)),
            )
    }

    pub(super) fn render_empty_state(
        &self,
        compact: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let copy = self.connection_copy();
        let conversation_error =
            self.view.connected && self.view.notice.starts_with("Conversation unavailable:");
        let workspace_error = self.view.connected
            && self.view.selected.is_none()
            && self.view.notice.starts_with("Sessions unavailable:")
            && !self.view.sessions_loading;
        let requested_unavailable =
            self.requested_session.is_some() && self.local_notice.starts_with("Requested session ");
        let (key, title, description, animated) = if self.requested_session.is_some()
            && self.local_notice.starts_with("Opening requested session ")
        {
            (
                "state-requested-session-loading",
                "Opening requested session",
                "Fetching this session’s messages.",
                true,
            )
        } else if requested_unavailable {
            (
                "state-requested-session",
                "Requested session unavailable",
                "Refresh the session list to check for this session, or start a new one.",
                false,
            )
        } else if !self.view.connected {
            (
                "state-connection",
                copy.title,
                copy.description,
                copy.animated,
            )
        } else if self.view.loading {
            (
                "state-conversation-loading",
                "Loading conversation",
                "Fetching this session’s messages. Your draft stays here while we load.",
                true,
            )
        } else if self.view.sessions_loading && self.view.sessions.is_empty() {
            (
                "state-sessions-loading",
                "Loading your sessions",
                "Checking your workspace for existing sessions.",
                true,
            )
        } else if conversation_error {
            (
                "state-conversation-error",
                "Conversation unavailable",
                "We couldn’t load this conversation. Retry to fetch its messages again.",
                false,
            )
        } else if workspace_error {
            (
                "state-workspace-error",
                "Workspace needs attention",
                "We couldn’t complete the workspace request. Refresh to check your sessions again.",
                false,
            )
        } else if self.selected_session().is_some_and(Session::stopped) {
            (
                "state-session-ended",
                "This session has ended",
                "Open session details to review its status or available resume options.",
                false,
            )
        } else if self.selected_session().is_some() {
            (
                "state-new-conversation",
                "Start a conversation",
                "Ask a question, explore your code, or describe what you want to build.",
                false,
            )
        } else if self.view.sessions.is_empty() {
            (
                "state-no-sessions",
                "Your next idea starts here",
                "Start a session in your project, or set up an agent to get ready.",
                false,
            )
        } else {
            (
                "state-select-session",
                "Choose a conversation",
                "Select a session in the sidebar, or start a new session for your next task.",
                false,
            )
        };
        div()
            .id("empty-state-viewport")
            .flex_1()
            .min_h_0()
            .pt(self.header_bounds.size.height + px(12.))
            .pb(if self.view.selected.is_some() {
                self.composer_dock_bounds.size.height + px(12.)
            } else {
                px(20.)
            })
            .px_5()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .debug_selector(move || key.into())
                    .w_full()
                    .max_w(px(460.))
                    .py_3()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(if compact { 36. } else { 48. }))
                            .rounded(px(12.))
                            .bg(rgb(p.surface))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if animated {
                                brand_spinner(24., p, "empty-state-spinner")
                            } else {
                                brand_mark(24., p)
                            }),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(if compact { 20. } else { 24. }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(13.))
                            .line_height(gpui::relative(1.5))
                            .text_color(rgb(p.muted))
                            .child(description),
                    )
                    .when(
                        self.view.power_paused && self.view.can_resume_power_pause,
                        |d| d.child(self.wake_button(cx)),
                    )
                    .when(
                        conversation_error || workspace_error || requested_unavailable,
                        |d| {
                            d.child(
                                self.button("retry-empty", "Try again", true)
                                    .debug_selector(|| "retry-empty".into())
                                    .bg(rgb(p.primary))
                                    .text_color(rgb(p.on_primary))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.command(Command::Refresh, cx)
                                    })),
                            )
                        },
                    )
                    .when(
                        self.view.connected
                            && !self.view.loading
                            && !self.view.sessions_loading
                            && !conversation_error
                            && !workspace_error
                            && self.selected_session().is_none(),
                        |d| {
                            d.child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(
                                        self.button("welcome-new", "Start a session", !self.demo)
                                            .debug_selector(|| "welcome-new".into())
                                            .when(!self.demo, |d| {
                                                d.on_click(cx.listener(|this, _, window, cx| {
                                                    this.show_new_session(window, cx)
                                                }))
                                            }),
                                    )
                                    .child(
                                        self.button("welcome-setup", "Set up an agent", true)
                                            .debug_selector(|| "welcome-setup".into())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_feature(Screen::Setup, window, cx)
                                            })),
                                    ),
                            )
                        },
                    )
                    .when(
                        self.view.connected
                            && !self.view.loading
                            && !self.view.sessions_loading
                            && self.selected_session().is_some_and(|s| !s.stopped())
                            && !conversation_error,
                        |d| {
                            d.child(
                                self.button(
                                    "focus-first-message",
                                    "Write your first message",
                                    true,
                                )
                                .debug_selector(|| "focus-first-message".into())
                                .bg(rgb(p.primary))
                                .text_color(rgb(p.on_primary))
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.composer
                                            .update(cx, |input, cx| input.focus(window, cx))
                                    },
                                )),
                            )
                        },
                    )
                    .when(
                        self.view.connected
                            && !self.view.loading
                            && self.selected_session().is_some_and(Session::stopped),
                        |d| {
                            d.child(
                                self.button("empty-session-details", "Session details", true)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_feature(Screen::Session, window, cx)
                                    })),
                            )
                        },
                    ),
            )
    }
}
