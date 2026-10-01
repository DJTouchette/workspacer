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
    page: usize,
    pub(super) closed: Option<u64>,
}

fn status(child: &ChildAgent, connected: bool) -> &'static str {
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
        let changed = self.child_ui.agents != agents || details_changed;
        self.child_ui.agents = agents;
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
            let color = if !self.view.connected {
                p.muted
            } else if matches!(child.status.as_str(), "stopped" | "ended") {
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
        body.child(content)
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
