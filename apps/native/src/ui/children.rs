//! Compact child rows stay with their dispatch; provider transcripts stay parent scoped.
use super::*;
use gpui_component::tooltip::Tooltip;
use wks_native::{
    child_agents::{ChildAgent, ChildAgents},
    features::Request,
    model::Row,
};

#[derive(Default)]
pub(super) struct ChildUi {
    pub agents: ChildAgents,
    clock: Option<Task<()>>,
    pub(super) page: usize,
    pub(super) closed: Option<u64>,
    /// Unanchored children grouped by the transcript row they follow.
    pub(super) overview: std::collections::BTreeMap<usize, Vec<ChildAgent>>,
    /// Row key each untimed child was first seen after, so it stays put.
    pinned: HashMap<String, u64>,
}

pub(super) fn status(child: &ChildAgent, connected: bool) -> &'static str {
    if !connected {
        return "Offline";
    }
    match child.status.as_str() {
        "running" | "working" | "thinking" | "responding" | "streaming" => "Working",
        "complete" | "completed" | "done" => "Completed",
        "stopped" | "ended" => "Ended",
        "failed" | "error" => "Failed",
        "lost" => "Unavailable",
        "approval" | "waiting_approval" => "Needs approval",
        "question" | "waiting_input" => "Needs input",
        "input" | "idle" => "Ready",
        "starting" | "initializing" => "Starting",
        "background" => "Background",
        _ => "Status unavailable",
    }
}
fn count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.)
    } else {
        value.to_string()
    }
}
impl Workspace {
    /// Place each unanchored child after the row that preceded its start (or,
    /// untimed, the row that was newest when it first appeared), snapped to
    /// the end of a work card, so the overview holds its place in the timeline
    /// and later messages arrive below it.
    fn overview_groups(
        &mut self,
        agents: &ChildAgents,
        view: &View,
    ) -> std::collections::BTreeMap<usize, Vec<ChildAgent>> {
        let mut groups = std::collections::BTreeMap::<usize, Vec<ChildAgent>>::new();
        let rows = &view.transcript.rows;
        if view.child.is_some() || rows.is_empty() {
            return groups;
        }
        for child in &agents.unanchored {
            let ix =
                wks_native::child_agents::overview_anchor(child, rows.iter().map(AsRef::as_ref))
                    .or_else(|| {
                        let key = *self
                            .child_ui
                            .pinned
                            .entry(child.id.clone())
                            .or_insert(rows.back().map_or(0, |r| r.key));
                        rows.iter().position(|r| r.key == key)
                    })
                    .unwrap_or(rows.len() - 1);
            let ix = wks_native::tool_preview::group_span(rows, ix, self.settings.merge_turn_tools)
                .map_or(ix, |span| span.end - 1);
            groups.entry(ix).or_default().push(child.clone());
        }
        groups
    }

    pub(super) fn sync_children(&mut self, view: &View, cx: &mut Context<Self>) -> bool {
        let agents = view
            .sessions
            .iter()
            .find(|s| Some(&s.id) == view.selected.as_ref())
            .map(|parent| {
                wks_native::child_agents::project(
                    parent,
                    &view.sessions,
                    view.transcript.rows.iter().map(AsRef::as_ref),
                )
            })
            .unwrap_or_default();
        let details_changed = view
            .requests
            .get("subagent-history")
            .map(|s| (s.number, s.loading))
            != self
                .view
                .requests
                .get("subagent-history")
                .map(|s| (s.number, s.loading));
        if self.view.selected != view.selected || self.view.child != view.child {
            self.child_ui.pinned.clear();
        }
        let overview = self.overview_groups(&agents, view);
        let changed =
            self.child_ui.agents != agents || details_changed || self.child_ui.overview != overview;
        self.child_ui.agents = agents;
        self.child_ui.overview = overview;
        if self.view.selected != view.selected {
            self.child_ui.page = 0;
            self.child_ui.closed = None;
        }
        let ticking = view.connected
            && self
                .child_ui
                .agents
                .by_tool
                .values()
                .flatten()
                .chain(self.child_ui.agents.unanchored.iter())
                .any(ChildAgent::running);
        if !ticking {
            self.child_ui.clock = None;
        } else if self.child_ui.clock.is_none() {
            self.child_ui.clock = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            if this.screen == Screen::Conversation && !this.new_session {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        changed
    }
    pub(super) fn render_children(
        &mut self,
        parent: &str,
        key: &str,
        children: &[ChildAgent],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let mut body = div()
            .ml_3()
            .pl_3()
            .border_l_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap_2();
        for child in children {
            let child_key = format!("{key}-child-{}", child.id);
            let managed = child.session_id.is_some();
            let provider = self
                .selected_session()
                .map(|s| s.provider.as_str())
                .unwrap_or("");
            let source = if managed {
                "Workspacer child session".to_owned()
            } else {
                format!(
                    "{} native subagent",
                    match provider {
                        "claude" => "Claude",
                        "codex" => "Codex",
                        _ => "Provider",
                    }
                )
            };
            let known = child
                .session_id
                .as_ref()
                .map(|id| {
                    self.view.sessions.iter().any(|s| {
                        &s.id == id
                            && (s.parent_session_id.is_empty() || s.parent_session_id == parent)
                    })
                })
                .unwrap_or_else(|| {
                    self.selected_session()
                        .and_then(|s| s.subagents.as_array())
                        .is_some_and(|items| items.iter().any(|s| s["id"] == child.id))
                });
            let enabled = self.view.connected && known;
            let color =
                if !self.view.connected || matches!(child.status.as_str(), "stopped" | "ended") {
                    p.muted
                } else if child.failed() {
                    p.error
                } else if matches!(
                    child.status.as_str(),
                    "waiting_approval" | "approval" | "waiting_input" | "question"
                ) {
                    p.warning
                } else if child.running() {
                    p.busy
                } else if child.complete() {
                    p.success
                } else {
                    p.muted
                };
            let label = if child.description.is_empty() || child.description == child.label {
                child.label.clone()
            } else {
                format!("{} · {}", child.label, child.description)
            };
            let mut metadata = Vec::new();
            if !child.model.is_empty() {
                metadata.push(child.model.clone());
            }
            if let Some(tools) = child.telemetry.tool_calls {
                metadata.push(format!("{} tools", count(tools)));
            }
            if let Some(tokens) = child.telemetry.tokens {
                metadata.push(format!("{} tokens", count(tokens)));
            }
            if let Some(cost) = child.telemetry.cost_usd {
                metadata.push(format!("${cost:.3}"));
            }
            // Offline clocks freeze at their last observation rather than
            // inventing ongoing work after losing the source connection.
            let at = if self.view.connected {
                Some(timing::now_ms())
            } else {
                child
                    .telemetry
                    .last_activity_ms
                    .or(child.telemetry.completed_at_ms)
            };
            if let Some(duration) = at.and_then(|at| child.duration_ms(at)) {
                metadata.push(timing::duration_label(duration.min(i64::MAX as u64) as i64));
            }
            let owner = parent.to_owned();
            let child_id = child.id.clone();
            let target = child.session_id.clone();
            let label = if label.is_empty() {
                child.id.clone()
            } else {
                label
            };
            let title = label.clone();
            let mut row = div()
                .id(SharedString::from(child_key.clone()))
                .debug_selector({
                    let id = child.id.clone();
                    move || format!("child-agent-{id}")
                })
                .flex()
                .flex_col()
                .gap_1();
            row = row.child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!("{child_key}-open"))),
                    p,
                    enabled,
                )
                .rounded(px(p.control_radius))
                .px_2()
                .py_2()
                .flex()
                .flex_col()
                .gap_1()
                .debug_selector({
                    let managed = child.session_id.is_some();
                    move || {
                        if managed {
                            "open-spawned-session".into()
                        } else {
                            "open-provider-subagent".into()
                        }
                    }
                })
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgb(p.selected)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.view.selected.as_deref() != Some(&owner) || !this.view.connected
                            {
                                return;
                            }
                            if let Some(id) = &target {
                                if this.view.sessions.iter().any(|s| {
                                    &s.id == id
                                        && (s.parent_session_id.is_empty()
                                            || s.parent_session_id == owner)
                                }) {
                                    this.command(Command::Select(id.clone()), cx);
                                }
                            } else {
                                this.child_ui.page = 0;
                                this.child_ui.closed = None;
                                this.request(
                                    Request::SubagentHistory {
                                        session: owner.clone(),
                                        agent: child_id.clone(),
                                    },
                                    cx,
                                );
                            }
                        }))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id(SharedString::from(format!("{child_key}-source")))
                                .debug_selector(move || {
                                    if managed {
                                        "workspacer-child-icon".into()
                                    } else {
                                        "provider-child-icon".into()
                                    }
                                })
                                .tooltip(move |window, cx| {
                                    Tooltip::new(source.clone()).build(window, cx)
                                })
                                .child(
                                    Icon::new(if managed {
                                        IconName::Bot
                                    } else {
                                        IconName::SquareTerminal
                                    })
                                    .size(px(14.))
                                    .text_color(rgb(p.accent)),
                                ),
                        )
                        .child(if self.view.connected && child.running() {
                            brand_spinner(
                                12.,
                                p,
                                SharedString::from(format!("{child_key}-activity")),
                            )
                        } else {
                            status_dot(color)
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(color))
                                .child(status(child, self.view.connected)),
                        )
                        .child(
                            Icon::new(IconName::ArrowRight)
                                .size(px(12.))
                                .text_color(rgb(if enabled { p.muted } else { p.disabled })),
                        ),
                )
                .when(!metadata.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(metadata.join(" · ")),
                    )
                })
                .when(!child.telemetry.last_tool_name.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(format!(
                                "{}{}",
                                child.telemetry.last_tool_name,
                                if child.telemetry.last_tool_summary.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", child.telemetry.last_tool_summary)
                                }
                            )),
                    )
                }),
            );
            if child.session_id.is_none() {
                row = row.child(
                    self.render_child_transcript(parent, &child.id, &title, &child_key, window, cx),
                );
            }
            body = body.child(row);
        }
        body
    }
    fn render_child_transcript(
        &mut self,
        parent: &str,
        agent: &str,
        title: &str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.appearance.palette();
        let Some(state)=self.view.requests.get("subagent-history").filter(|s|matches!(&s.request,Request::SubagentHistory{session,agent:target} if session==parent && target==agent)).cloned() else{return div().into_any_element();};
        if self.child_ui.closed == Some(state.number) {
            return div()
                .debug_selector(|| "closed-child-transcript".into())
                .into_any_element();
        }
        let rows = state.value["rows"].as_array();
        let count = rows.map_or(0, Vec::len);
        let pages = count.div_ceil(50).max(1);
        let page = self.child_ui.page.min(pages - 1);
        let owner = parent.to_owned();
        let target = agent.to_owned();
        let number = state.number;
        let mut body = div()
            .id(SharedString::from(format!("{key}-transcript")))
            .debug_selector(|| "child-transcript-panel".into())
            .p_2()
            .rounded(px(p.control_radius))
            .bg(rgb(p.surface))
            .flex()
            .flex_col()
            .gap_2()
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
                            .text_size(px(12.))
                            .child(format!("{title} · retained transcript")),
                    )
                    .child({
                        let (owner, target) = (owner.clone(), target.clone());
                        self.quiet_button(
                            SharedString::from(format!("{key}-open")),
                            "Open as chat",
                            IconName::Maximize,
                            self.view.connected,
                        )
                        .debug_selector(|| "open-child-chat".into())
                        .when(self.view.connected, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.open_sidebar_child(&owner, &target, cx)
                            }))
                        })
                    })
                    .child(
                        self.quiet_button(
                            SharedString::from(format!("{key}-refresh")),
                            "Refresh",
                            IconName::Redo2,
                            self.view.connected && !state.loading,
                        )
                        .when(self.view.connected && !state.loading, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.request(
                                    Request::SubagentHistory {
                                        session: owner.clone(),
                                        agent: target.clone(),
                                    },
                                    cx,
                                )
                            }))
                        }),
                    )
                    .child(
                        self.icon_button(
                            SharedString::from(format!("{key}-close")),
                            "Close child transcript",
                            IconName::Close,
                            true,
                        )
                        .debug_selector(|| "close-child-transcript".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.child_ui.closed = Some(number);
                            this.invalidate_child_layout(cx);
                        })),
                    ),
            );
        if let Some(error) = &state.error {
            return body
                .child(div().text_color(rgb(p.warning)).child(error.clone()))
                .into_any_element();
        }
        if state.loading {
            return body.child("Loading child transcript…").into_any_element();
        }
        if state.value["omitted"] == true || state.value["first_seq"].as_u64().unwrap_or(0) > 1 {
            body = body.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Earlier child events were trimmed."),
            );
        }
        if count == 0 {
            return body
                .child("No child messages are available yet.")
                .into_any_element();
        }
        let mut content = div()
            .id(SharedString::from(format!("{key}-messages")))
            .max_h(px(280.))
            .overflow_y_scroll();
        for value in rows.into_iter().flatten().skip(page * 50).take(50) {
            if let Ok(row) = serde_json::from_value::<Row>(value.clone()) {
                content =
                    content.child(self.render_message(&row, "child-history", false, window, cx));
            }
        }
        body.child(super::smooth_scroll::scroll_zone(content))
            .when(pages > 1, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            self.button(
                                SharedString::from(format!("{key}-previous")),
                                "Previous",
                                page > 0,
                            )
                            .when(page > 0, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.child_ui.page = page - 1;
                                    this.invalidate_child_layout(cx);
                                }))
                            }),
                        )
                        .child(format!("Page {} of {}", page + 1, pages))
                        .child(
                            self.button(
                                SharedString::from(format!("{key}-next")),
                                "Next",
                                page + 1 < pages,
                            )
                            .when(page + 1 < pages, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.child_ui.page = page + 1;
                                    this.invalidate_child_layout(cx);
                                }))
                            }),
                        ),
                )
            })
            .into_any_element()
    }

    fn invalidate_child_layout(&self, cx: &mut Context<Self>) {
        let anchor = self.scroll_anchor();
        self.list.splice(
            0..self.view.transcript.rows.len(),
            self.view.transcript.rows.len(),
        );
        self.list.scroll_to(anchor);
        cx.notify();
    }

    pub(super) fn render_child_only(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let owner = self.view.selected.clone().unwrap_or_default();
        let children = self.child_ui.agents.unanchored.clone();
        div().flex_1().min_h_0().w_full().child(
            div()
                .id("child-only-list")
                .debug_selector(|| "child-only-list".into())
                .w_full()
                .h_full()
                .overflow_y_scroll()
                .pt(self.header_bounds.size.height + px(16.))
                .pb(self.composer_dock_bounds.size.height + px(12.))
                .child(
                    chrome::chat_column()
                        .child("Child agents")
                        .child(self.render_children(&owner, "unanchored", &children, window, cx)),
                ),
        )
    }
}

impl Workspace {
    /// The provider-native subagent shown as the chat, with its parent and
    /// projected details (title, model, status) when the fleet still has them.
    pub(super) fn viewed_child(&self) -> Option<(Session, Option<ChildAgent>)> {
        let target = self.view.child.as_ref()?;
        let parent = self
            .view
            .sessions
            .iter()
            .find(|s| s.id == target.parent)?
            .clone();
        let projected = wks_native::child_agents::project(&parent, &[], std::iter::empty());
        let child = projected
            .unanchored
            .into_iter()
            .chain(projected.by_tool.into_values().flatten())
            .find(|c| c.id == target.agent);
        Some((parent, child))
    }

    fn child_title(child: Option<&ChildAgent>) -> String {
        child
            .map(|c| {
                if c.description.is_empty() {
                    c.label.clone()
                } else {
                    c.description.clone()
                }
            })
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Subagent".into())
    }

    fn leave_child(&mut self, cx: &mut Context<Self>) {
        self.command(Command::ViewChild(None), cx);
    }

    /// Title pill while a subagent is the chat: back to the parent, the
    /// subagent's title and model, and refresh.
    pub(super) fn render_child_title_bar(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (parent, child) = self.viewed_child()?;
        let p = self.appearance.palette();
        let title = Self::child_title(child.as_ref());
        let status = child
            .as_ref()
            .map(|c| super::children::status(c, self.view.connected))
            .unwrap_or("Subagent");
        let running = child.as_ref().is_some_and(ChildAgent::running);
        let model = child.as_ref().map(|c| c.model.clone()).unwrap_or_default();
        let parent_title = self.session_title(&parent);
        Some(
            div()
                .id("title-bar")
                .debug_selector(|| "child-title-bar".into())
                .occlude()
                .max_w_full()
                .min_w_0()
                .h(px(40.))
                .pl_1()
                .pr_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded_full()
                .bg(rgb(p.surface))
                .border_1()
                .border_color(rgb(p.border))
                .shadow(chrome::floating_shadow(p))
                .child(
                    self.icon_button("child-back", "Back to parent", IconName::ArrowLeft, true)
                        .debug_selector(|| "child-back".into())
                        .rounded_full()
                        .on_click(cx.listener(|this, _, _, cx| this.leave_child(cx))),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(140.))
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child(parent_title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.disabled))
                        .child("/"),
                )
                .child(status_dot(if running { p.busy } else { p.success }))
                .child(
                    div()
                        .min_w_0()
                        .max_w(px(320.))
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py(px(3.))
                        .rounded_full()
                        .bg(rgb(p.selected))
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(chrome::brand_badge(
                            wks_native::model::model_provider(&model).unwrap_or(&parent.provider),
                            wks_native::model::model_display_name(&model),
                            p,
                            12.,
                        )),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .pr_2()
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(status),
                )
                .child(
                    self.icon_button(
                        "refresh",
                        "Refresh subagent",
                        IconName::Redo,
                        self.view.connected && !self.view.loading,
                    )
                    .when(self.view.connected && !self.view.loading, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                    }),
                ),
        )
    }

    /// Stands in for the composer: provider-native subagents take no input.
    pub(super) fn render_child_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (_, child) = self.viewed_child()?;
        let p = self.appearance.palette();
        let running = child.as_ref().is_some_and(ChildAgent::running);
        Some(
            div()
                .debug_selector(|| "child-read-only-bar".into())
                .occlude()
                .w_full()
                .px_4()
                .py_3()
                .rounded(px(p.composer_radius))
                .border_1()
                .border_color(gpui::Hsla::from(rgb(p.border)).opacity(0.55))
                .bg(rgb(p.surface))
                .shadow(chrome::floating_shadow(p))
                .flex()
                .items_center()
                .gap_3()
                .child(
                    Icon::new(IconName::EyeOff)
                        .size(px(16.))
                        .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Read-only subagent"),
                        )
                        .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                            if running {
                                "Its parent drives it; this view updates as it works."
                            } else {
                                "Its parent drove it. Message the parent to follow up."
                            },
                        )),
                )
                .child(
                    self.quiet_button(
                        "child-back-parent",
                        "Back to parent",
                        IconName::ArrowLeft,
                        true,
                    )
                    .debug_selector(|| "child-back-parent".into())
                    .on_click(cx.listener(|this, _, _, cx| this.leave_child(cx))),
                ),
        )
    }
}

impl Workspace {
    /// A collapsible card for a batch of subagents. Open while any works;
    /// once every one has settled it becomes a collapsed "finished" summary
    /// (the open state is keyed by settledness, so the change re-collapses it).
    pub(super) fn render_overview(
        &mut self,
        session: &str,
        children: &[ChildAgent],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let total = children.len();
        let settled = children.iter().all(ChildAgent::settled);
        let working = children.iter().filter(|c| c.running()).count();
        let failed = children.iter().filter(|c| c.failed()).count();
        let done = children
            .iter()
            .filter(|c| c.settled() && !c.failed())
            .count();
        let waiting = total.saturating_sub(working + failed + done);
        let first = children.first().map(|c| c.id.as_str()).unwrap_or_default();
        let key = format!("overview:{session}:{first}:{settled}");
        let open = self.chat.open.get(&key).copied().unwrap_or(!settled);
        let tools: u64 = children.iter().filter_map(|c| c.telemetry.tool_calls).sum();
        let span = || {
            let start = children
                .iter()
                .filter_map(|c| c.telemetry.started_at_ms)
                .min()?;
            let end = children
                .iter()
                .filter_map(|c| c.telemetry.completed_at_ms.or(c.telemetry.last_activity_ms))
                .max()?;
            (end > start).then(|| timing::duration_label(end - start))
        };
        let noun = if total == 1 { "subagent" } else { "subagents" };
        let title = if settled {
            format!("{total} {noun} finished")
        } else {
            format!("{total} {noun}")
        };
        let radius = px((p.panel_radius - 1.).max(0.));
        let (icon, tone) = if failed > 0 && settled {
            (IconName::TriangleAlert, p.error)
        } else if settled {
            (IconName::CircleCheck, p.success)
        } else {
            (IconName::Bot, p.accent)
        };
        let chip = |text: String, color: u32| {
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(color))
                .child(text)
        };
        let toggle = key.clone();
        div()
            .id(SharedString::from(format!("{key}-card")))
            .debug_selector(|| "subagent-overview".into())
            .mt_2()
            .w_full()
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(gpui::Hsla::from(rgb(p.surface)).opacity(0.6))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("{key}-toggle")))
                    .debug_selector(|| "subagent-overview-toggle".into())
                    .rounded_t(radius)
                    .when(!open, |d| d.rounded_b(radius))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.selected)))
                    .child(Icon::new(icon).size(px(14.)).text_color(rgb(tone)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(div().flex_1())
                    .when(working > 0, |d| {
                        d.child(brand_spinner(
                            11.,
                            p,
                            SharedString::from(format!("{key}-spin")),
                        ))
                        .child(chip(format!("{working} working"), p.busy))
                    })
                    .when(waiting > 0, |d| {
                        d.child(chip(format!("{waiting} waiting"), p.warning))
                    })
                    .when(done > 0 && !settled, |d| {
                        d.child(chip(format!("{done} done"), p.success))
                    })
                    .when(failed > 0, |d| {
                        d.child(chip(format!("{failed} failed"), p.error))
                    })
                    .when(settled && tools > 0, |d| {
                        d.child(chip(format!("{} tools", count(tools)), p.muted))
                    })
                    .when_some(settled.then(span).flatten(), |d, label| {
                        d.child(chip(label, p.disabled))
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(rgb(p.disabled)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle.clone(), !settled, cx)
                    })),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .px_3()
                        .pt_1()
                        .pb_3()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .child(self.render_children(session, &key, children, window, cx)),
                )
            })
    }
}
