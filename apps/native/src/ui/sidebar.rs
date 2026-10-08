//! Session navigation, with a compact rail that preserves window-local state.
use super::*;
use gpui_component::tooltip::Tooltip;
use std::collections::BTreeMap;
use wks_native::child_agents::{ChildAgent, ClearMark, clear_key_provider, clear_key_session};

const CLEAR_CHILD: &str = "Clear · hide this finished child here on this device · not archived or stopped · still in Session history";
const CLEAR_SUBAGENT: &str = "Clear · hide this finished subagent here on this device · nothing is stopped · the parent chat keeps its record";
const CLEAR_FINISHED: &str = "Clear finished children · hide them here on this device · running children stay · nothing is archived or stopped";

fn child_limit_note(session: &Session) -> &'static str {
    if session
        .subagents
        .as_array()
        .is_some_and(|children| children.len() == wks_native::child_agents::MAX_PROVIDER_CHILDREN)
    {
        "\nChild list capped at 32. Open the parent conversation for full child activity."
    } else {
        ""
    }
}

fn provider_clear(parent: &str, child: &ChildAgent) -> (String, ClearMark) {
    (
        clear_key_provider(parent, &child.id),
        ClearMark::provider(child, super::timing::now_ms()),
    )
}

fn session_clear(session: &Session) -> (String, ClearMark) {
    (
        clear_key_session(&session.id),
        ClearMark::session(session, super::timing::now_ms()),
    )
}

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
            // Provider-native subagents stay under their parent through every
            // turn and focus change, finished or not, until the user clears
            // them. The one being viewed stays put so its highlight has a home.
            for child in native.unanchored.into_iter().filter(|child| {
                !self.provider_cleared(&parent.id, child)
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
                        })
                        .when(child.settled(), |d| {
                            let (parent, child) = (parent.clone(), child.clone());
                            d.child(
                                self.icon_button(
                                    SharedString::from(format!(
                                        "clear-native-{parent}-{}",
                                        child.id
                                    )),
                                    CLEAR_SUBAGENT,
                                    IconName::EyeOff,
                                    true,
                                )
                                .debug_selector(move || format!("sidebar-clear-provider-{ix}"))
                                .size(px(20.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.clear_children(
                                            vec![provider_clear(&parent, &child)],
                                            cx,
                                        );
                                    },
                                )),
                            )
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
                                    .text_color(rgb(if !self.view.connected {
                                        p.muted
                                    } else if child.failed() {
                                        p.error
                                    } else if child.settled() {
                                        p.success
                                    } else if wks_native::child_agents::provider_active(
                                        &child.status,
                                    ) {
                                        p.warning
                                    } else {
                                        p.muted
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

    fn child_clears(&self) -> Option<&BTreeMap<String, ClearMark>> {
        self.settings.cleared_children.get(&self.project_scope)
    }

    /// A finished provider-native child the user cleared, still on the run
    /// it was cleared after.
    pub(super) fn provider_cleared(&self, parent: &str, child: &ChildAgent) -> bool {
        child.settled()
            && self
                .child_clears()
                .and_then(|marks| marks.get(&clear_key_provider(parent, &child.id)))
                .is_some_and(|mark| mark.covers(&ClearMark::provider(child, 0)))
    }

    /// A finished Workspacer child session the user cleared.
    pub(super) fn session_cleared(&self, session: &Session) -> bool {
        !session.parent_session_id.is_empty()
            && wks_native::child_agents::session_finished(session)
            && self
                .child_clears()
                .and_then(|marks| marks.get(&clear_key_session(&session.id)))
                .is_some_and(|mark| mark.covers(&ClearMark::session(session, 0)))
    }

    /// Cleared on this device (whether or not it has worked since).
    pub(super) fn clear_marked(&self, id: &str) -> bool {
        self.child_clears()
            .is_some_and(|marks| marks.contains_key(&clear_key_session(id)))
    }

    /// The parent's finished, shown children: Workspacer sessions nested
    /// beneath it and its provider-native agents. Running ones never count.
    pub(super) fn clearable_children(&self, parent: &Session) -> Vec<(String, ClearMark)> {
        let now = super::timing::now_ms();
        let sessions = self.view.sessions.iter().filter(|s| {
            s.parent_session_id == parent.id
                && s.id != parent.id
                && !self.archived(&s.id)
                && wks_native::child_agents::session_finished(s)
                && !self.session_cleared(s)
        });
        let native = wks_native::child_agents::project(parent, &[], std::iter::empty());
        sessions
            .map(|s| (clear_key_session(&s.id), ClearMark::session(s, now)))
            .chain(
                native
                    .unanchored
                    .iter()
                    .filter(|c| c.settled() && !self.provider_cleared(&parent.id, c))
                    .map(|c| provider_clear(&parent.id, c)),
            )
            .collect()
    }

    /// Hide finished children from this device's sidebar. A visibility mark
    /// only: no stop, close, archive, forget or selection change is sent, and
    /// the session, transcript and history stay available.
    pub(super) fn clear_children(
        &mut self,
        marks: Vec<(String, ClearMark)>,
        cx: &mut Context<Self>,
    ) {
        // Click handlers can outlive the snapshot they were painted from.
        // Recheck terminal state and work identity before recording a clear.
        let eligible: BTreeMap<_, _> = self
            .view
            .sessions
            .iter()
            .flat_map(|parent| self.clearable_children(parent))
            .collect();
        let marks: Vec<_> = marks
            .into_iter()
            .filter_map(|(key, painted)| {
                eligible
                    .get(&key)
                    .filter(|current| painted.covers(current))
                    .map(|current| (key, current.clone()))
            })
            .collect();
        if marks.is_empty() {
            return;
        }
        let scope = self
            .settings
            .cleared_children
            .entry(self.project_scope.clone())
            .or_default();
        for (key, mark) in marks {
            wks_native::child_agents::remember_clear(scope, key, mark);
        }
        self.save_settings(cx);
    }

    /// Show a cleared child session in the sidebar again.
    pub(super) fn unclear_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = clear_key_session(id);
        if let Some(marks) = self.settings.cleared_children.get_mut(&self.project_scope)
            && marks.remove(&key).is_some()
        {
            self.settings
                .cleared_children
                .retain(|_, marks| !marks.is_empty());
            self.save_settings(cx);
        }
    }

    /// A cleared child that is seen working again comes back and stays back
    /// once it finishes; the clear was for the work it had done.
    pub(super) fn lift_child_clears(&mut self, cx: &mut Context<Self>) {
        if !self.view.connected {
            return;
        }
        let Some(marks) = self.settings.cleared_children.get(&self.project_scope) else {
            return;
        };
        let mut working = std::collections::BTreeSet::new();
        for s in self.view.sessions.iter() {
            let key = clear_key_session(&s.id);
            if s.working()
                || s.subagents.as_array().into_iter().flatten().any(|child| {
                    wks_native::child_agents::provider_active(
                        child["status"].as_str().unwrap_or(""),
                    )
                })
                || s.approval.is_some()
                || s.questions.is_some()
                || matches!(
                    s.state.as_str(),
                    "approval" | "question" | "waiting_approval" | "waiting_input" | "background"
                )
                || marks
                    .get(&key)
                    .is_some_and(|mark| !mark.covers(&ClearMark::session(s, 0)))
            {
                working.insert(key);
            }
            for child in wks_native::child_agents::project(s, &[], std::iter::empty()).unanchored {
                let key = clear_key_provider(&s.id, &child.id);
                if child.running()
                    || matches!(
                        child.status.as_str(),
                        "approval" | "question" | "waiting_approval" | "waiting_input"
                    )
                    || marks
                        .get(&key)
                        .is_some_and(|mark| !mark.covers(&ClearMark::provider(&child, 0)))
                {
                    working.insert(key);
                }
            }
        }
        let active: Vec<String> = marks
            .keys()
            .filter(|key| working.contains(*key))
            .cloned()
            .collect();
        if active.is_empty() {
            return;
        }
        if let Some(marks) = self.settings.cleared_children.get_mut(&self.project_scope) {
            for key in &active {
                marks.remove(key);
            }
        }
        self.settings
            .cleared_children
            .retain(|_, marks| !marks.is_empty());
        self.save_settings(cx);
    }

    /// The footer's connection note. A healthy connection is the norm and
    /// gets none; only the fixture or a lost/paused connection is named.
    pub(super) fn footer_connection_label(&self) -> Option<&'static str> {
        if self.demo {
            Some("Demo")
        } else if !self.view.connected {
            Some(self.connection_copy().label)
        } else {
            None
        }
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
                self.icon_button("new-session-rail", "New agent", IconName::Plus, !self.demo)
                    .debug_selector(|| "new-session-button".into())
                    .when(!self.demo, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.show_new_session(window, cx)),
                        )
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
                                    this.status_of(session, p).0
                                } else {
                                    "Offline"
                                };
                                let details = format!(
                                    "{title}\n{}\n{} · {status}",
                                    session.cwd, session.provider
                                );
                                let details = format!("{details}{}", child_limit_note(session));
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
                self.icon_button("jobs-rail", "Jobs", IconName::Calendar, true)
                    .when(!self.new_session && self.screen == Screen::Jobs, |d| {
                        d.bg(rgb(p.selected)).text_color(rgb(p.text))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Jobs, window, cx)
                    })),
            )
            .child(
                self.icon_button("settings-rail", "Settings", IconName::Settings, true)
                    .when(!self.new_session && self.screen == Screen::Settings, |d| {
                        d.bg(rgb(p.selected)).text_color(rgb(p.text))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_screen(Screen::Settings, window, cx)
                    })),
            )
            .when(!self.view.connected, |d| {
                d.child(div().py_1().child(status_dot(p.warning)))
            })
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
                        self.icon_button("new-session", "New agent", IconName::Plus, !self.demo)
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
                    .text_size(px(chrome::scale::OVERLINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(p.muted))
                    .child("SESSIONS")
                    .child(if filtered {
                        let total = self
                            .view
                            .sessions
                            .iter()
                            .filter(|s| !self.archived(&s.id) && !self.session_cleared(s))
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
                        .child({
                            let path = path.clone();
                            let can_spawn = self.view.connected && !self.demo;
                            self.icon_button(
                                "new-agent-in-filter",
                                "New agent in this project",
                                IconName::Plus,
                                can_spawn,
                            )
                            .debug_selector(|| "new-agent-in-filter".into())
                            .when(can_spawn, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    this.new_agent_in_project(path.clone(), window, cx)
                                }))
                            })
                        })
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
                        } else if self.view.sessions_loading || self.archive_loading() {
                            "Loading sessions…"
                        } else if filtered {
                            "No matching sessions"
                        } else {
                            "No sessions yet"
                        })
                        .child(div().text_color(rgb(p.muted)).text_size(px(11.)).child(
                            if !self.view.connected {
                                self.connection_copy().description
                            } else if self.view.sessions_loading || self.archive_loading() {
                                "Checking your workspace for sessions."
                            } else if filtered {
                                "Try another search or clear the filters."
                            } else {
                                "Start a new agent to begin."
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
                                    this.status_of(session, p)
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
                                // Nested, finished children offer Clear (this
                                // device's child view) in place of Archive.
                                let finished_child = depth > 0
                                    && wks_native::child_agents::session_finished(session);
                                let clearable = this.clearable_children(session);
                                let badge = chrome::model_badge(session, p, 11.);
                                let context = session
                                    .context_window
                                    .map(|tokens| format!(" · {}K context", tokens / 1000))
                                    .unwrap_or_default();
                                let details = format!(
                                    "{title}\n{}\n{model_info}{context} · {status}",
                                    session.cwd
                                );
                                let details = format!("{details}{}", child_limit_note(session));
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
                                                .when(!clearable.is_empty(), |d| {
                                                    d.child(
                                                        this.icon_button(
                                                            SharedString::from(format!("clear-finished-{}", session.id)),
                                                            CLEAR_FINISHED,
                                                            IconName::EyeOff,
                                                            true,
                                                        )
                                                        .debug_selector(move || format!("sidebar-clear-finished-{ix}"))
                                                        .size(px(20.))
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.clear_children(clearable.clone(), cx);
                                                        })),
                                                    )
                                                })
                                                .child(if finished_child {
                                                    this.icon_button(
                                                        SharedString::from(format!("clear-child-{}", session.id)),
                                                        CLEAR_CHILD,
                                                        IconName::EyeOff,
                                                        true,
                                                    )
                                                    .debug_selector(move || format!("sidebar-clear-{ix}"))
                                                    .size(px(20.))
                                                    .on_click(cx.listener({
                                                        let clear = session_clear(session);
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.clear_children(vec![clear.clone()], cx);
                                                        }
                                                    }))
                                                } else {
                                                    this.icon_button(
                                                        SharedString::from(format!("archive-sidebar-{}", session.id)),
                                                        "Archive · hides it in every client, keeps it running · restore from Session history → Archived",
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
                                                    }))
                                                }),
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
                                                    this.project_name(&session.cwd),
                                                ))
                                                .when(!(working && this.view.connected), |d| {
                                                    d.child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .text_color(rgb(color))
                                                            .child(if !child_limit_note(session).is_empty() { format!("{status} · Child list capped") } else { status.to_owned() }),
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
                    .child(
                        self.quiet_button("nav-jobs", "Jobs", IconName::Calendar, true)
                            .debug_selector(|| "nav-jobs".into())
                            .when(!self.new_session && self.screen == Screen::Jobs, |d| {
                                d.bg(rgb(p.selected)).text_color(rgb(p.text))
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_feature(Screen::Jobs, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    // A healthy connection is the norm and needs no label;
                    // only a fixture or a lost/paused connection is called out.
                    .when_some(self.footer_connection_label(), |d, label| {
                        d.child(
                            div()
                                .debug_selector(|| "sidebar-connection-status".into())
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted))
                                .child(label)
                                .when(!self.view.connected, |d| d.child(status_dot(p.warning))),
                        )
                    }),
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
