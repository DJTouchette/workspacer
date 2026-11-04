//! Honest loading and recovery feedback; actions reuse the controller contracts.
use super::primitives::{ButtonSize, ButtonVariant};
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
        self.primary_button("wake-workspace", "Reconnect and wake", true)
            .debug_selector(|| "wake-workspace".into())
            .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
    }

    pub(super) fn render_connection_banner(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let copy = self.connection_copy();
        div()
            .debug_selector(|| "connection-banner".into())
            .occlude()
            .rounded(px(p.panel_radius))
            .bg(rgb(p.surface))
            .p_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .child(if copy.animated {
                brand_spinner(12., p)
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
        let p = self.palette();
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
                "Refresh the session list to check for this session, or start a new agent.",
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
        } else if self.view.child.is_some() {
            (
                "state-child-empty",
                "No subagent messages yet",
                "Its transcript appears here as it works.",
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
                "Start an agent in one of your projects, or set one up first.",
                false,
            )
        } else {
            (
                "state-select-session",
                "Choose a conversation",
                "Select a session in the sidebar, or start a new agent for your next task.",
                false,
            )
        };
        if self.design().is_v2() {
            let welcome = matches!(key, "state-no-sessions" | "state-select-session")
                && self.view.child.is_none();
            return self.render_empty_state_v2(
                EmptyV2 {
                    key,
                    title,
                    description,
                    animated,
                    welcome,
                    retry: conversation_error || workspace_error || requested_unavailable,
                    first_message: self.view.connected
                        && !self.view.loading
                        && !self.view.sessions_loading
                        && self.selected_session().is_some_and(|s| !s.stopped())
                        && !conversation_error,
                    ended: self.view.connected
                        && !self.view.loading
                        && self.selected_session().is_some_and(Session::stopped),
                },
                compact,
                cx,
            );
        }
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
                                brand_spinner(24., p)
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
                                self.primary_button("retry-empty", "Try again", true)
                                    .debug_selector(|| "retry-empty".into())
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
                                        self.button("welcome-new", "Start an agent", !self.demo)
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
                                self.primary_button(
                                    "focus-first-message",
                                    "Write your first message",
                                    true,
                                )
                                .debug_selector(|| "focus-first-message".into())
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

// area-composer: design v2 empty states and the first-run welcome.

/// What a v2 empty state shows, decided by `render_empty_state`.
pub(super) struct EmptyV2 {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub animated: bool,
    /// Nothing selected (or no sessions yet): the welcome instead.
    pub welcome: bool,
    pub retry: bool,
    pub first_message: bool,
    pub ended: bool,
}

/// The welcome's one-line promise.
pub(super) const WELCOME_VALUE: &str = "Run and supervise coding agents across your machines";

/// Three starter tasks under the welcome composer: each fills the task.
pub(super) const STARTERS: [(&str, &str); 3] = [
    (
        "welcome-starter-0",
        "Explain how this project is put together",
    ),
    ("welcome-starter-1", "Find a failing test and fix it"),
    (
        "welcome-starter-2",
        "Review my uncommitted changes and suggest improvements",
    ),
];

/// An agent's row in the welcome checklist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentSetup {
    Installed,
    Missing,
    Unknown,
}

impl AgentSetup {
    pub(super) fn of(installed: Option<bool>) -> Self {
        match installed {
            Some(true) => Self::Installed,
            Some(false) => Self::Missing,
            None => Self::Unknown,
        }
    }
}

impl Workspace {
    /// Whether the welcome's task field has focus: its send keys start the
    /// agent rather than reaching a composer that is not there.
    pub(super) fn welcome_prompt_focused(&self, window: &Window, cx: &App) -> bool {
        self.design().is_v2()
            && !self.new_session
            && self.screen == Screen::Conversation
            && self.selected_session().is_none()
            && self.prompt.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Start an agent from the welcome: the task, project and agent chosen
    /// there go straight to a launch. Anything the launch still needs (a
    /// folder, a sign-in) is asked on the New agent form it falls back to.
    pub(super) fn start_from_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.provider;
        let cwd = self.welcome_project().unwrap_or_default();
        self.show_new_session(window, cx);
        if !self.new_session {
            return;
        }
        self.choose_provider(provider, window, cx);
        self.permission = self.settings.default_access(self.provider);
        if !cwd.is_empty() {
            self.seed_project(&cwd, cx);
        }
        self.create(window, cx);
    }

    /// The project the welcome would start in: the chosen folder, else the
    /// most recent known project.
    pub(super) fn welcome_project(&self) -> Option<String> {
        if !self.projects.cwd.is_empty() {
            return Some(self.projects.cwd.clone());
        }
        self.known_projects()
            .into_iter()
            .find(|p| !p.worktree)
            .map(|p| p.path)
    }

    /// Whether an agent is installed where the hub runs, from the hub's
    /// install report (or the Setup page's last check).
    fn agent_installed(&self, provider: &str) -> Option<bool> {
        self.view.installed.get(provider).copied().or_else(|| {
            self.view.requests.get("setup").and_then(|state| {
                state.value["installed"]
                    .as_array()?
                    .iter()
                    .find(|row| row["provider"] == provider)?["found"]
                    .as_bool()
            })
        })
    }

    pub(super) fn render_empty_state_v2(
        &self,
        state: EmptyV2,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let viewport = div()
            .id("empty-state-viewport")
            .flex_1()
            .min_h_0()
            .pt(self.header_bounds.size.height + px(t.space.lg))
            .pb(if self.view.selected.is_some() {
                self.composer_dock_bounds.size.height + px(t.space.lg)
            } else {
                px(t.space.xxl)
            })
            .px(px(t.space.xxl))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center();
        if state.welcome && !compact {
            return viewport
                .justify_center()
                .child(self.render_welcome(state.key, cx));
        }
        let key = state.key;
        let icon = match key {
            "state-session-ended" => IconName::CircleCheck,
            "state-conversation-error" | "state-workspace-error" | "state-requested-session" => {
                IconName::TriangleAlert
            }
            "state-child-empty" => IconName::Bot,
            _ => IconName::Inbox,
        };
        let mark = if state.animated {
            div()
                .size(px(t.icon.xl + t.space.xl))
                .flex()
                .items_center()
                .justify_center()
                .child(primitives::status_indicator(
                    primitives::StatusKind::Working,
                    &p,
                ))
        } else {
            div()
                .size(px(t.icon.xl + t.space.xl))
                .rounded(px(t.radius.panel))
                .bg(primitives::tonal(&p, t.wash.rest))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.roles.text_secondary))
                .child(Icon::new(icon).size(px(t.icon.lg)))
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(t.space.sm))
            .when(
                self.view.power_paused && self.view.can_resume_power_pause,
                |d| {
                    d.child(
                        primitives::button(
                            "wake-workspace",
                            "Reconnect and wake",
                            None,
                            ButtonVariant::Primary,
                            ButtonSize::Small,
                            true,
                            &p,
                        )
                        .debug_selector(|| "wake-workspace".into())
                        .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx))),
                    )
                },
            )
            .when(state.retry, |d| {
                d.child(
                    primitives::button(
                        "retry-empty",
                        "Try again",
                        None,
                        ButtonVariant::Primary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(|| "retry-empty".into())
                    .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx))),
                )
            })
            // A short window's welcome: the two ways forward.
            .when(state.welcome, |d| {
                d.child(
                    primitives::button(
                        "welcome-new",
                        "Start an agent",
                        Some(IconName::Plus),
                        ButtonVariant::Primary,
                        ButtonSize::Small,
                        !self.demo,
                        &p,
                    )
                    .debug_selector(|| "welcome-new".into())
                    .when(!self.demo, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.show_new_session(window, cx)),
                        )
                    }),
                )
                .child(
                    primitives::button(
                        "welcome-setup",
                        "Set up an agent",
                        None,
                        ButtonVariant::Secondary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(|| "welcome-setup".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Setup, window, cx)
                    })),
                )
            })
            .when(state.first_message, |d| {
                // The composer below owns the primary.
                d.child(
                    primitives::button(
                        "focus-first-message",
                        "Write your first message",
                        None,
                        ButtonVariant::Secondary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(|| "focus-first-message".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.composer
                            .update(cx, |input, cx| input.focus(window, cx))
                    })),
                )
            })
            .when(state.ended, |d| {
                d.child(
                    primitives::button(
                        "empty-session-details",
                        "Session details",
                        None,
                        ButtonVariant::Secondary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Session, window, cx)
                    })),
                )
            });
        viewport.justify_center().child(
            div()
                .debug_selector(move || key.into())
                .w_full()
                .max_w(px(t.layout.dialog_width))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(t.space.md))
                .text_center()
                .child(mark)
                .child(
                    div()
                        .mt(px(t.space.xs))
                        .text_size(px(t.text.heading))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(p.text))
                        .child(state.title),
                )
                .child(
                    div()
                        .text_size(px(t.text.body))
                        .line_height(gpui::relative(t.text.line_height))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(state.description),
                )
                .child(div().mt(px(t.space.md)).child(actions)),
        )
    }

    /// First run, or nothing selected: wordmark and promise, a task composer
    /// with the project and agent to run it, three starter tasks and the
    /// agents' setup state.
    fn render_welcome(&self, key: &'static str, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let first_run = self.view.sessions.is_empty();
        let connected = self.view.connected;
        let can_start = connected && !self.demo && !self.spawn_pending && !self.view.creating;
        let projects: Vec<KnownProject> = self
            .known_projects()
            .into_iter()
            .filter(|p| !p.worktree)
            .take(4)
            .collect();
        let chosen = self.welcome_project().unwrap_or_default();
        let project_name = projects
            .iter()
            .find(|p| wks_native::projects::same_dir(&p.path, &chosen))
            .map(|p| p.title().to_owned())
            .or_else(|| {
                std::path::Path::new(&chosen)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            });
        let headline = match &project_name {
            Some(name) => format!("What should we work on in {name}?"),
            None => "What should we work on?".to_owned(),
        };
        let wordmark = div()
            .debug_selector(|| "welcome-wordmark".into())
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .child(brand_mark(t.text.title, p))
            .child(
                div()
                    .text_size(px(t.text.title))
                    .font_weight(FontWeight(t.weight.title))
                    .text_color(rgb(p.text))
                    .child("Workspacer"),
            );
        let intro = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(t.space.sm))
            .child(wordmark)
            .child(
                div()
                    .debug_selector(|| "welcome-value".into())
                    .text_size(px(t.text.subheading))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(WELCOME_VALUE),
            );

        // The composer: the task, then where and with whom it runs.
        let agents = primitives::segmented(&p)
            .debug_selector(|| "welcome-agent".into())
            .children(
                [("claude", "Claude"), ("codex", "Codex")].map(|(id, name)| {
                    primitives::segment(
                        SharedString::from(format!("welcome-agent-{id}")),
                        name,
                        self.provider == id,
                        &p,
                    )
                    .debug_selector(move || format!("welcome-agent-{id}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_provider(id, window, cx);
                        cx.notify();
                    }))
                }),
            );
        let start = primitives::button(
            "welcome-start",
            "Start",
            Some(IconName::ArrowUp),
            ButtonVariant::Primary,
            ButtonSize::Small,
            can_start,
            &p,
        )
        .debug_selector(|| "welcome-start".into())
        .tooltip(primitives::tooltip_view(
            "Start an agent on this task",
            Some(if cfg!(target_os = "macos") {
                "⌘ Enter"
            } else {
                "Ctrl Enter"
            }),
            &p,
        ))
        .when(can_start, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.start_from_welcome(window, cx)))
        });
        let composer = div()
            .debug_selector(|| "welcome-composer".into())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .p(px(t.space.lg))
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .rounded(px(t.radius.composer))
            .shadow(primitives::elevation(&p, t.elevation.floating))
            .child(
                div()
                    .px(px(t.space.xs))
                    .text_size(px(t.text.subheading))
                    .child(Input::new(&self.prompt).appearance(false)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(t.space.md))
                    .child(agents)
                    .child(start),
            );

        // Projects: the recent ones as chips, then any other folder.
        let project_row = div()
            .debug_selector(|| "welcome-projects".into())
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(t.space.sm))
            .child(
                div()
                    .mr(px(t.space.xs))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child("Project"),
            )
            .children(projects.into_iter().enumerate().map(|(ix, project)| {
                let path = project.path.clone();
                primitives::chip(
                    ("welcome-project", ix),
                    project.title().to_owned(),
                    wks_native::projects::same_dir(&project.path, &chosen),
                    true,
                    &p,
                )
                .debug_selector(move || format!("welcome-project-{ix}"))
                .tooltip(primitives::tooltip_view(project.path.clone(), None, &p))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.seed_project(&path, cx);
                    cx.notify();
                }))
            }))
            .child(
                primitives::chip(
                    "welcome-project-other",
                    "Other folder…",
                    false,
                    !self.demo,
                    &p,
                )
                .debug_selector(|| "welcome-project-other".into())
                .when(!self.demo, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| {
                        this.show_new_session(window, cx);
                        this.open_project_picker(window, cx);
                    }))
                }),
            );

        let starters = div()
            .debug_selector(|| "welcome-starters".into())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .children(STARTERS.into_iter().enumerate().map(|(ix, (id, text))| {
                let _ = ix;
                primitives::list_row(id, false, true, &p)
                    .debug_selector(move || id.into())
                    .text_color(rgb(p.roles.text_secondary))
                    .child(
                        Icon::new(IconName::ArrowRight)
                            .size(px(t.icon.sm))
                            .text_color(rgb(p.roles.text_tertiary)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.prompt.update(cx, |input, cx| {
                            input.set_value(text, window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    }))
            }));

        div()
            .debug_selector(move || key.into())
            .w_full()
            .max_w(px(t.layout.chat_width))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(t.space.xxl))
            .child(intro)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.lg))
                    .child(
                        div()
                            .debug_selector(|| "welcome-headline".into())
                            .px(px(t.space.xs))
                            .text_size(px(t.text.heading))
                            .font_weight(FontWeight(t.weight.semibold))
                            .text_color(rgb(p.text))
                            .child(headline),
                    )
                    .child(composer)
                    .child(project_row)
                    .child(starters),
            )
            .child(self.render_agent_checklist(first_run, cx))
    }

    /// The agents and whether each is ready, from the Setup page's data.
    fn render_agent_checklist(&self, first_run: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let rows = [
            ("claude", "Claude Code", "claude"),
            ("codex", "Codex CLI", "codex login"),
        ]
        .map(|(id, name, command)| {
            let setup = AgentSetup::of(self.agent_installed(id));
            let (kind, label) = match setup {
                AgentSetup::Installed => (primitives::StatusKind::Done, "Installed"),
                AgentSetup::Missing => (primitives::StatusKind::NeedsYou, "Not installed"),
                AgentSetup::Unknown => (primitives::StatusKind::Idle, "Not checked"),
            };
            div()
                .debug_selector(move || format!("welcome-agent-row-{id}"))
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .min_h(px(t.layout.row_height))
                .child(chrome::provider_glyph(id, t.icon.md, p))
                .child(
                    div()
                        .w(px(t.layout.status_column))
                        .flex_shrink_0()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(p.text))
                        .child(name),
                )
                .child(
                    primitives::StatusBadge::new(kind)
                        .label(label)
                        .render(&p)
                        .debug_selector(move || format!("welcome-agent-status-{id}")),
                )
                .child(div().flex_1())
                .when(setup == AgentSetup::Missing, |d| {
                    d.child(
                        div()
                            .px(px(t.space.sm))
                            .rounded(px(t.radius.xs))
                            .bg(rgb(p.chat))
                            .border(px(t.stroke.hairline))
                            .border_color(rgb(p.roles.card_border))
                            .font_family(mono_font())
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(command),
                    )
                })
        });
        let ready = [("claude"), ("codex")]
            .iter()
            .filter(|id| self.agent_installed(id) == Some(true))
            .count();
        primitives::card(&p)
            .debug_selector(|| "welcome-setup-checklist".into())
            .w_full()
            .gap(px(t.space.xs))
            .py(px(t.space.md))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(if first_run {
                                format!("Agents · {ready} of 2 ready")
                            } else {
                                "Agents".to_owned()
                            }),
                    )
                    .child(
                        primitives::button(
                            "welcome-setup",
                            "Set up agents",
                            None,
                            ButtonVariant::Ghost,
                            ButtonSize::Small,
                            true,
                            &p,
                        )
                        .debug_selector(|| "welcome-setup".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Setup, window, cx)
                        })),
                    ),
            )
            .children(rows)
    }
}
