//! Session navigation, with a compact rail that preserves window-local state.
use super::*;
use gpui_component::tooltip::Tooltip;

#[derive(Clone)]
pub(super) enum SidebarRow {
    Session {
        index: usize,
        depth: usize,
    },
    Provider {
        parent: String,
        child: Box<wks_native::child_agents::ChildAgent>,
        depth: usize,
    },
}

impl Workspace {
    pub(super) fn sidebar_rows(&self, cx: &App) -> Vec<SidebarRow> {
        let visible = self.visible_sessions(cx);
        let mut rows = Vec::new();
        for (index, depth) in wks_native::navigation::session_tree(&self.view.sessions, &visible) {
            rows.push(SidebarRow::Session { index, depth });
            let parent = &self.view.sessions[index];
            let native = wks_native::child_agents::project(parent, &[], std::iter::empty());
            // Provider-native subagents are the parent's business: once one
            // has finished and the parent's turn is over, it leaves the sidebar
            // (the chat keeps its record). The one being viewed stays put.
            let parent_busy =
                parent.working() || parent.approval.is_some() || parent.questions.is_some();
            for child in native.unanchored.into_iter().filter(|child| {
                !child.settled()
                    || parent_busy
                    || self
                        .view
                        .child
                        .as_ref()
                        .is_some_and(|t| t.parent == parent.id && t.agent == child.id)
            }) {
                rows.push(SidebarRow::Provider {
                    parent: parent.id.clone(),
                    child: Box::new(child),
                    depth: depth + 1,
                });
            }
        }
        rows
    }

    pub(super) fn sidebar_session_position(&self, index: usize, cx: &App) -> usize {
        let sessions = self.visible_sessions(cx);
        self.sidebar_rows(cx)
            .iter()
            .position(|row| {
                matches!(row,
            SidebarRow::Session { index: ix, .. } if Some(ix) == sessions.get(index))
            })
            .unwrap_or(index)
    }

    fn render_sidebar_provider(
        &self,
        parent: &str,
        child: &wks_native::child_agents::ChildAgent,
        depth: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        // Children inherit the parent's provider unless their model says otherwise.
        let provider = self
            .view
            .sessions
            .iter()
            .find(|s| s.id == parent)
            .map_or("", |s| s.provider.as_str());
        let viewing = self.view.child.as_ref().is_some_and(|t| {
            t.parent == parent && t.agent == child.id && self.navigation_selected.is_none()
        });
        let parent = parent.to_owned();
        let agent = child.id.clone();
        let title = if child.description.is_empty() {
            child.label.clone()
        } else {
            child.description.clone()
        };
        let title = if title.is_empty() {
            "Native agent".into()
        } else {
            title
        };
        let state = if !self.view.connected {
            "Offline"
        } else {
            super::children::status(child, true)
        };
        if self.sidebar_collapsed {
            let details = format!("{title}\nNative agent · {state}");
            return div().h(px(48.)).px_2().pb_1().child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!(
                        "sidebar-native-{parent}-{agent}"
                    ))),
                    p,
                    self.view.connected,
                )
                .debug_selector(move || format!("sidebar-provider-{ix}"))
                .h_full()
                .w_full()
                .rounded(px(p.control_radius))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(IconName::SquareTerminal)
                        .size(px(16.))
                        .text_color(rgb(p.accent)),
                )
                .tooltip(move |window, cx| Tooltip::new(details.clone()).build(window, cx))
                .when(self.view.connected, |d| {
                    d.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_sidebar_child(&parent, &agent, cx);
                        }))
                }),
            );
        }
        // The sidebar is a uniform_list: every row takes the 64px session-card
        // height, so child rows must fit two lines like session cards do.
        div()
            .h(px(64.))
            .px_2()
            .pl(px(8. + depth.min(6) as f32 * 16.))
            .pb_1()
            .child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!(
                        "sidebar-native-{parent}-{agent}"
                    ))),
                    p,
                    self.view.connected,
                )
                .debug_selector(move || format!("sidebar-provider-{ix}"))
                .h_full()
                .w_full()
                .when(viewing, |d| d.bg(rgb(p.selected)))
                .px_3()
                .py_2()
                .rounded(px(p.control_radius))
                .flex()
                .flex_col()
                .justify_center()
                .gap_1()
                .overflow_hidden()
                .tooltip(|window, cx| {
                    Tooltip::new("Provider-native agent · open conversation preview")
                        .build(window, cx)
                })
                // Same type and badge as session cards: title on top, brand
                // model badge and status below.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        )
                        .when(child.running() && self.view.connected, |d| {
                            d.child(brand_spinner(
                                12.,
                                p,
                                SharedString::from(format!("sidebar-native-working-{}", child.id)),
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(
                            div()
                                .debug_selector(move || format!("sidebar-child-model-{ix}"))
                                .flex_1()
                                .min_w_0()
                                .child(chrome::brand_badge(
                                    wks_native::model::model_provider(&child.model)
                                        .unwrap_or(provider),
                                    wks_native::model::model_display_name(&child.model),
                                    p,
                                    11.,
                                )),
                        )
                        .when(!(child.running() && self.view.connected), |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(rgb(if child.failed() {
                                        p.error
                                    } else {
                                        p.success
                                    }))
                                    .child(state),
                            )
                        }),
                )
                .when(self.view.connected, |d| {
                    d.cursor_pointer()
                        .hover(move |s| s.bg(rgb(p.surface)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_sidebar_child(&parent, &agent, cx);
                        }))
                }),
            )
    }

    /// Open a provider-native subagent as its own (read-only) chat.
    pub(super) fn open_sidebar_child(&mut self, parent: &str, agent: &str, cx: &mut Context<Self>) {
        if !self.view.connected {
            return;
        }
        // A window pinned to another session never navigates away from it.
        if self
            .requested_session
            .as_ref()
            .is_some_and(|id| id != parent)
        {
            self.command(Command::Select(parent.to_owned()), cx);
            return;
        }
        self.new_session = false;
        self.screen = Screen::Conversation;
        self.command(
            Command::ViewChild(Some(wks_native::controller::ChildTarget {
                parent: parent.to_owned(),
                agent: agent.to_owned(),
            })),
            cx,
        );
        cx.notify();
    }

    fn sidebar_toggle(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        self.icon_button(
            "toggle-sidebar",
            if self.sidebar_collapsed {
                "Expand sidebar"
            } else {
                "Collapse sidebar"
            },
            if self.sidebar_collapsed {
                IconName::PanelLeftOpen
            } else {
                IconName::PanelLeftClose
            },
            true,
        )
        .debug_selector(|| "sidebar-toggle".into())
        .on_click(cx.listener(|this, _, _, cx| {
            this.sidebar_collapsed = !this.sidebar_collapsed;
            cx.notify();
        }))
    }

    fn render_sidebar_rail(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.appearance.palette();
        let rows = self.sidebar_rows(cx);
        div()
            .debug_selector(|| "session-sidebar".into())
            .w(px(56.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .py_3()
            .when(chrome::custom_caption(), |d| {
                d.child(
                    chrome::drag_region(div())
                        .debug_selector(|| "sidebar-drag-region".into())
                        .w(px(40.))
                        .h(px(32.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(brand_mark(18., p)),
                )
            })
            .child(self.sidebar_toggle(cx))
            .child(
                self.icon_button(
                    "new-session-rail",
                    "New session",
                    IconName::Plus,
                    !self.demo,
                )
                .debug_selector(|| "new-session-button".into())
                .when(!self.demo, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx)))
                }),
            )
            .child(
                self.icon_button("search-rail", "Search sessions", IconName::Search, true)
                    .debug_selector(|| "sidebar-search".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sidebar_collapsed = false;
                        this.search.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    })),
            )
            .child(
                self.icon_button("projects-rail", "All projects", IconName::Folder, true)
                    .when(!self.new_session && self.screen == Screen::Projects, |d| {
                        d.bg(rgb(p.selected)).text_color(rgb(p.text))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_screen(Screen::Projects, window, cx)
                    })),
            )
            .child(
                self.icon_button("history-rail", "Session history", IconName::BookOpen, true)
                    .when(!self.new_session && self.screen == Screen::Recent, |d| {
                        d.bg(rgb(p.selected)).text_color(rgb(p.text))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Recent, window, cx)
                    })),
            )
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(
                    self.icon_button(
                        "notice-rail",
                        "Workspace notice — expand sidebar",
                        IconName::Info,
                        true,
                    )
                    .text_color(rgb(p.warning))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_collapsed = false;
                        cx.notify();
                    })),
                )
            })
            .child(
                uniform_list(
                    "session-rail",
                    rows.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                let index = match &rows[ix] {
                                    SidebarRow::Session { index, .. } => *index,
                                    SidebarRow::Provider {
                                        parent,
                                        child,
                                        depth,
                                    } => {
                                        return this.render_sidebar_provider(
                                            parent, child, *depth, ix, cx,
                                        );
                                    }
                                };
                                let session = &this.view.sessions[index];
                                let id = session.id.clone();
                                // A viewed subagent owns the highlight, not its parent.
                                let active = this
                                    .navigation_selected
                                    .as_ref()
                                    .or(this.view.selected.as_ref())
                                    == Some(&id)
                                    && (this.navigation_selected.is_some()
                                        || this.view.child.is_none());
                                let title = this.session_title(session);
                                let initial = title
                                    .split_whitespace()
                                    .take(2)
                                    .filter_map(|word| word.chars().next())
                                    .flat_map(char::to_uppercase)
                                    .collect::<String>();
                                let status = if this.view.connected {
                                    session_status(session, p).0
                                } else {
                                    "Offline"
                                };
                                let details = format!(
                                    "{title}\n{}\n{} · {status}",
                                    session.cwd, session.provider
                                );
                                div().h(px(48.)).px_2().pb_1().child(
                                    chrome::interactive_control(
                                        div().id(SharedString::from(format!(
                                            "session-{}",
                                            session.id
                                        ))),
                                        p,
                                        true,
                                    )
                                    .debug_selector(move || format!("sidebar-session-{ix}"))
                                    .h_full()
                                    .w_full()
                                    .rounded(px(p.control_radius))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if active { p.text } else { p.muted }))
                                    .when(active, |d| d.bg(rgb(p.selected)))
                                    .hover(move |style| {
                                        style.bg(rgb(if active { p.selected } else { p.surface }))
                                    })
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(details.clone()).build(window, cx)
                                    })
                                    .child(initial)
                                    .when(session.working() && this.view.connected, |d| {
                                        d.child(brand_spinner(
                                            8.,
                                            p,
                                            SharedString::from(format!(
                                                "rail-working-{}",
                                                session.id
                                            )),
                                        ))
                                    })
                                    .when(
                                        session.approval.is_some() || session.questions.is_some(),
                                        |d| d.child(status_dot(p.warning)),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.new_session = false;
                                            this.screen = Screen::Conversation;
                                            this.command(Command::Select(id.clone()), cx);
                                        },
                                    )),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .w_full()
                .flex_1()
                .min_h_0()
                .track_scroll(self.sidebar_scroll.clone()),
            )
            .when(self.view.power_paused, |d| {
                d.child(
                    self.icon_button(
                        "resume-rail",
                        "Reconnect and wake",
                        IconName::Redo,
                        self.view.can_resume_power_pause,
                    )
                    .text_color(rgb(p.warning))
                    .when(self.view.can_resume_power_pause, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                    }),
                )
            })
            .child(
                self.icon_button("settings-rail", "Settings", IconName::Settings, true)
                    .when(!self.new_session && self.screen == Screen::Settings, |d| {
                        d.bg(rgb(p.selected)).text_color(rgb(p.text))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_screen(Screen::Settings, window, cx)
                    })),
            )
            .child(div().py_1().child(status_dot(if self.view.connected {
                p.success
            } else {
                p.warning
            })))
            .into_any_element()
    }

    pub(super) fn render_sidebar(
        &mut self,
        _narrow: bool,
        _compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.appearance.palette();
        if self.sidebar_collapsed {
            return self.render_sidebar_rail(window, cx);
        }
        let editing = [
            &self.composer,
            &self.search,
            &self.project_path,
            &self.project_query,
            &self.label,
            &self.model,
            &self.prompt,
            &self.extras.name,
        ]
        .into_iter()
        .chain(self.extras.answers.iter())
        .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let control_focused = !self.focus.is_focused(window) && !editing;
        let visible_sessions = self.sidebar_rows(cx);
        let filtered = !self.search.read(cx).value().is_empty() || self.project_filter.is_some();
        let no_visible_sessions = visible_sessions.is_empty();
        div()
            .relative()
            .w(px(wks_native::navigation::sidebar_width(
                self.settings.sidebar_width,
                unzoom(window.viewport_size().width),
            )))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .debug_selector(|| "session-sidebar".into())
            .child(
                // Padding and the logo remain a useful drag target even at
                // minimum sidebar width. The occluding action group excludes
                // this native hitbox from GPUI's reverse-order hit test.
                chrome::drag_region(div())
                    .debug_selector(|| "sidebar-drag-region".into())
                    .px_3()
                    .py_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(brand_mark(18., p))
                    .child(div().flex_1())
                    .child(div().flex().items_center().gap_2().flex_shrink_0().occlude()
                    .child(
                        self.icon_button("new-session", "New session", IconName::Plus, !self.demo)
                            .debug_selector(|| "new-session-button".into())
                            .when(!self.demo, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.show_new_session(window, cx)
                                }))
                            }),
                    )
                    .child(
                        self.icon_button("nav-projects", "All projects", IconName::Folder, true)
                            .when(!self.new_session && self.screen == Screen::Projects, |d| {
                                d.bg(rgb(p.selected)).text_color(rgb(p.text))
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_screen(Screen::Projects, window, cx)
                            })),
                    )
                    .child(
                        self.icon_button(
                            "nav-history",
                            "Session history",
                            IconName::BookOpen,
                            true,
                        )
                        .when(!self.new_session && self.screen == Screen::Recent, |d| {
                            d.bg(rgb(p.selected)).text_color(rgb(p.text))
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Recent, window, cx)
                        })),
                    )
                    .child(self.sidebar_toggle(cx))),
            )
            .child(
                div().px_3().pb_2().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .child(
                            Icon::new(IconName::Search)
                                .size(px(14.))
                                .text_color(rgb(p.muted)),
                        )
                        .child(Input::new(&self.search).appearance(false)),
                ),
            )
            .when(self.view.power_paused, |d| {
                d.child(
                    div()
                        .px_3()
                        .pb_3()
                        .text_size(px(12.))
                        .text_color(rgb(p.warning))
                        .child(self.view.notice.clone())
                        .when(self.view.can_resume_power_pause, |d| {
                            d.child(
                                self.button("resume-power-pause", "Reconnect and wake", true)
                                    .mt_2()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.command(Command::Refresh, cx)
                                    })),
                            )
                        }),
                )
            })
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(self.render_ui_notice(cx))
            })
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .flex()
                    .justify_between()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child("SESSIONS")
                    .child(if filtered {
                        let total = self
                            .view
                            .sessions
                            .iter()
                            .filter(|s| !self.archived(&s.id))
                            .count();
                        format!("{} / {total}", self.visible_sessions(cx).len())
                    } else {
                        visible_sessions.len().to_string()
                    }),
            )
            .when_some(self.project_filter.as_ref(), |d, path| {
                d.child(
                    div()
                        .px_3()
                        .pb_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(p.accent))
                                .child(path.clone()),
                        )
                        .child(
                            self.button("all-projects", "All", true)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.project_filter = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(no_visible_sessions, |d| {
                d.child(
                    div()
                        .px_4()
                        .py_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(px(12.))
                        .child(if !self.view.connected {
                            self.connection_copy().label
                        } else if self.view.sessions_loading {
                            "Loading sessions…"
                        } else if filtered {
                            "No matching sessions"
                        } else {
                            "No sessions yet"
                        })
                        .child(div().text_color(rgb(p.muted)).text_size(px(11.)).child(
                            if !self.view.connected {
                                self.connection_copy().description
                            } else if self.view.sessions_loading {
                                "Checking your workspace for sessions."
                            } else if filtered {
                                "Try another search or clear the filters."
                            } else {
                                "Start a new session to begin."
                            },
                        ))
                        .when(filtered, |d| {
                            d.child(
                                self.button("clear-session-filters", "Clear filters", true)
                                    .debug_selector(|| "clear-session-filters".into())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.project_filter = None;
                                        this.search.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                        cx.notify();
                                    })),
                            )
                        }),
                )
            })
            .child(
                uniform_list(
                    "sessions",
                    visible_sessions.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                let (index, depth) = match &visible_sessions[ix] {
                                    SidebarRow::Session { index, depth } => (*index, *depth),
                                    SidebarRow::Provider {
                                        parent,
                                        child,
                                        depth,
                                    } => {
                                        return this.render_sidebar_provider(
                                            parent, child, *depth, ix, cx,
                                        );
                                    }
                                };
                                let session = &this.view.sessions[index];
                                let id = session.id.clone();
                                // A viewed subagent owns the highlight, not its parent.
                                let active = this
                                    .navigation_selected
                                    .as_ref()
                                    .or(this.view.selected.as_ref())
                                    == Some(&id)
                                    && (this.navigation_selected.is_some() || this.view.child.is_none());
                                let title = this.session_title(session);
                                let (status, color) = if this.view.connected {
                                    session_status(session, p)
                                } else {
                                    ("Offline", p.muted)
                                };
                                let provider = match session.provider.as_str() {
                                    "claude" => "Claude",
                                    "codex" => "Codex",
                                    "" => "Agent",
                                    other => other,
                                };
                                let model = session.display_model();
                                let model = if model.is_empty() {
                                    "Provider default".to_owned()
                                } else {
                                    model
                                };
                                let model_info = format!("{provider} · {model}");
                                let working = session.working();
                                let badge = chrome::model_badge(session, p, 11.);
                                let context = session
                                    .context_window
                                    .map(|tokens| format!(" · {}K context", tokens / 1000))
                                    .unwrap_or_default();
                                let details = format!(
                                    "{title}\n{}\n{model_info}{context} · {status}",
                                    session.cwd
                                );
                                div()
                                    .h(px(64.))
                                    .px_2()
                                    .pl(px(8. + depth.min(6) as f32 * 16.))
                                    .pb_1()
                                    .child(
                                        chrome::interactive_control(
                                            div().id(SharedString::from(format!(
                                                "session-{}",
                                                session.id
                                            ))),
                                            p,
                                            true,
                                        )
                                        .debug_selector(move || format!("sidebar-session-{ix}"))
                                        .h_full()
                                        .px_3()
                                        .py_2()
                                        .rounded(px(p.control_radius))
                                        .cursor_pointer()
                                        .overflow_hidden()
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .gap_1()
                                        .when(active, |d| d.bg(rgb(p.selected)))
                                        .hover(move |style| {
                                            style.bg(rgb(if active {
                                                p.selected
                                            } else {
                                                p.surface
                                            }))
                                        })
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(details.clone()).build(window, cx)
                                        })
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(px(14.))
                                                        .font_weight(if active {
                                                            FontWeight::SEMIBOLD
                                                        } else {
                                                            FontWeight::MEDIUM
                                                        })
                                                        .child(title),
                                                )
                                                .when(
                                                    session.working() && this.view.connected,
                                                    |d| {
                                                        d.child(brand_spinner(
                                                            12.,
                                                            p,
                                                            SharedString::from(format!(
                                                                "sidebar-working-{}",
                                                                session.id
                                                            )),
                                                        ))
                                                    },
                                                )
                                                .when(
                                                    session.approval.is_some()
                                                        || session.questions.is_some(),
                                                    |d| d.child(status_dot(p.warning)),
                                                )
                                                .child(
                                                    this.icon_button(
                                                        SharedString::from(format!("archive-sidebar-{}", session.id)),
                                                        "Archive on this device · restore from Session history → Archived",
                                                        IconName::Inbox,
                                                        true,
                                                    )
                                                    .debug_selector(move || format!("sidebar-archive-{ix}"))
                                                    .size(px(20.))
                                                    .on_click(cx.listener({
                                                        let id = id.clone();
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.toggle_archive(&id, cx);
                                                        }
                                                    })),
                                                ),
                                        )
                                        // Model and folder share one line; the
                                        // title-row loader already says "working".
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.))
                                                .text_size(px(11.))
                                                .text_color(rgb(p.muted))
                                                .child(
                                                    div()
                                                        .debug_selector(move || {
                                                            format!("sidebar-model-{ix}")
                                                        })
                                                        .flex_shrink_0()
                                                        .max_w(gpui::relative(0.55))
                                                        .child(badge),
                                                )
                                                .child(div().text_color(rgb(p.disabled)).child("·"))
                                                .child(
                                                    Icon::new(IconName::Folder)
                                                        .size(px(11.))
                                                        .flex_shrink_0(),
                                                )
                                                .child(div().flex_1().min_w_0().truncate().child(
                                                    chrome::project_label(&session.cwd).to_owned(),
                                                ))
                                                .when(!(working && this.view.connected), |d| {
                                                    d.child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .text_color(rgb(color))
                                                            .child(status.to_owned()),
                                                    )
                                                }),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.new_session = false;
                                                this.screen = Screen::Conversation;
                                                this.command(Command::Select(id.clone()), cx)
                                            }),
                                        ),
                                    )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.sidebar_scroll.clone())
                .flex_1()
                .min_h_0(),
            )
            .children(self.render_usage_strip(cx))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(keycap(
                        if control_focused {
                            "CONTROLS"
                        } else if !self.settings.vim_navigation {
                            "SHORTCUTS"
                        } else if self.focus.is_focused(window) {
                            "NORMAL"
                        } else {
                            "INSERT"
                        },
                        p,
                    ))
                    .child(div().text_size(px(10.)).text_color(rgb(p.muted)).child(
                        if control_focused {
                            "Tab next · Enter act"
                        } else if !self.settings.vim_navigation {
                            "Ctrl P · Ctrl ,"
                        } else if self.focus.is_focused(window) {
                            "i edit · g p projects"
                        } else {
                            "Esc to navigate"
                        },
                    )),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.update_available(), |d| {
                        d.child(
                            div()
                                .id("update-pill")
                                .debug_selector(|| "update-pill".into())
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .bg(gpui::Hsla::from(rgb(p.accent)).opacity(0.15))
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(p.accent))
                                .cursor_pointer()
                                .hover(move |s| s.bg(gpui::Hsla::from(rgb(p.accent)).opacity(0.25)))
                                .child("Update")
                                .tooltip(|window, cx| {
                                    Tooltip::new("A newer build is available on your channel").build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings_section = super::settings::SettingsSection::About;
                                    this.show_screen(Screen::Settings, window, cx)
                                })),
                        )
                    })
                    .child(
                        self.quiet_button("nav-settings", "Settings", IconName::Settings, true)
                            .when(!self.new_session && self.screen == Screen::Settings, |d| {
                                d.bg(rgb(p.selected)).text_color(rgb(p.text))
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_screen(Screen::Settings, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child(if self.demo {
                                "Demo"
                            } else {
                                self.connection_copy().label
                            }),
                    )
                    .child(status_dot(if self.view.connected {
                        p.success
                    } else {
                        p.warning
                    })),
            )
            .child(
                div()
                    .id("sidebar-resize")
                    .debug_selector(|| "sidebar-resize".into())
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(6.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(move |s| s.bg(rgb(p.border)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Drag to resize · double-click to reset").build(window, cx)
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                            if event.click_count == 2 {
                                this.settings.sidebar_width = 304.;
                                this.sidebar_drag = None;
                                this.save_settings(cx);
                            } else {
                                let width = wks_native::navigation::sidebar_width(
                                    this.settings.sidebar_width,
                                    unzoom(window.viewport_size().width),
                                );
                                this.sidebar_drag = Some((event.position.x, width));
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child({
                let entity = cx.entity().downgrade();
                let dragging = self.sidebar_drag.is_some();
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        if !dragging {
                            return;
                        }
                        let moving = entity.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                                if phase.capture() {
                                    let _ = moving.update(cx, |this, cx| {
                                        if let Some((start, width)) = this.sidebar_drag {
                                            this.settings.sidebar_width =
                                                wks_native::navigation::sidebar_width(
                                                    width + unzoom(event.position.x - start),
                                                    unzoom(window.viewport_size().width),
                                                );
                                            cx.stop_propagation();
                                            cx.notify();
                                        }
                                    });
                                }
                            },
                        );
                        let releasing = entity.clone();
                        window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                            if phase.capture() {
                                let _ = releasing.update(cx, |this, cx| {
                                    if this.sidebar_drag.take().is_some() {
                                        this.save_settings(cx);
                                        cx.stop_propagation();
                                    }
                                });
                            }
                        });
                    },
                )
                .absolute()
                .size(px(0.))
            })
            .into_any_element()
    }
}
