//! Session navigation, with a compact rail that preserves window-local state.
use super::*;
use gpui_component::tooltip::Tooltip;

impl Workspace {
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
        let rows = self.visible_sessions(cx);
        div()
            .debug_selector(|| "session-sidebar".into())
            .w(px(56.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .border_r_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .py_3()
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
                                let session = &this.view.sessions[rows[ix]];
                                let id = session.id.clone();
                                let active = this
                                    .navigation_selected
                                    .as_ref()
                                    .or(this.view.selected.as_ref())
                                    == Some(&id);
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
        narrow: bool,
        compact: bool,
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
            &self.project,
            &self.label,
            &self.model,
            &self.prompt,
            &self.extras.name,
        ]
        .into_iter()
        .chain(self.extras.answers.iter())
        .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let control_focused = !self.focus.is_focused(window) && !editing;
        let visible_sessions = self.visible_sessions(cx);
        let filtered = !self.search.read(cx).value().is_empty() || self.project_filter.is_some();
        let no_visible_sessions = visible_sessions.is_empty();
        div()
            .w(px(if narrow && compact {
                200.
            } else if narrow {
                232.
            } else {
                264.
            }))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .border_r_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .debug_selector(|| "session-sidebar".into())
            .child(
                div()
                    .px_3()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(brand_mark(18., p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Workspacer"),
                    )
                    .child(self.sidebar_toggle(cx)),
            )
            .child(
                div().px_3().pb_2().child(
                    self.quiet_button("new-session", "New session", IconName::Plus, !self.demo)
                        .debug_selector(|| "new-session-button".into())
                        .w_full()
                        .bg(rgb(p.surface))
                        .when(!self.demo, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.show_new_session(window, cx)
                                }),
                            )
                        }),
                ),
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
            .child(
                div().px_3().pb_1().child(
                    self.quiet_button("nav-projects", "All projects", IconName::Folder, true)
                        .w_full()
                        .when(!self.new_session && self.screen == Screen::Projects, |d| {
                            d.bg(rgb(p.selected)).text_color(rgb(p.text))
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_screen(Screen::Projects, window, cx)
                        })),
                ),
            )
            .child(
                div().px_3().pb_3().child(
                    self.quiet_button("nav-history", "Session history", IconName::BookOpen, true)
                        .w_full()
                        .when(!self.new_session && self.screen == Screen::Recent, |d| {
                            d.bg(rgb(p.selected)).text_color(rgb(p.text))
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Recent, window, cx)
                        })),
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
                        format!("{} / {total}", visible_sessions.len())
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
                                let session = &this.view.sessions[visible_sessions[ix]];
                                let id = session.id.clone();
                                let active = this
                                    .navigation_selected
                                    .as_ref()
                                    .or(this.view.selected.as_ref())
                                    == Some(&id);
                                let title = this.session_title(session);
                                let (status, color) = if this.view.connected {
                                    session_status(session, p)
                                } else {
                                    ("Offline", p.muted)
                                };
                                let details = format!(
                                    "{title}\n{}\n{} · {status}",
                                    session.cwd, session.provider
                                );
                                div().h(px(68.)).px_2().pb_1().child(
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
                                        style.bg(rgb(if active { p.selected } else { p.surface }))
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
                                            .when(session.working() && this.view.connected, |d| {
                                                d.child(brand_spinner(
                                                    12.,
                                                    p,
                                                    SharedString::from(format!(
                                                        "sidebar-working-{}",
                                                        session.id
                                                    )),
                                                ))
                                            })
                                            .when(
                                                session.approval.is_some()
                                                    || session.questions.is_some(),
                                                |d| d.child(status_dot(p.warning)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_size(px(11.))
                                            .text_color(rgb(p.muted))
                                            .child(Icon::new(IconName::Folder).size(px(11.)))
                                            .child(div().flex_1().min_w_0().truncate().child(
                                                chrome::project_label(&session.cwd).to_owned(),
                                            ))
                                            .when(status != "Ready", |d| {
                                                d.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .text_color(rgb(color))
                                                        .child(status.to_owned()),
                                                )
                                            }),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.new_session = false;
                                            this.screen = Screen::Conversation;
                                            this.command(Command::Select(id.clone()), cx)
                                        },
                                    )),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.sidebar_scroll.clone())
                .flex_1()
                .min_h_0(),
            )
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
            .into_any_element()
    }
}
