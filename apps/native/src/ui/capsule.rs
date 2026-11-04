//! Design v2's title capsule and session tools: the title with a neutral
//! model chip and the access mode always in view, counts in words, the
//! model popover's anchor, and the tool rail docked to the window edge in
//! labelled groups. The shipped capsule is `chrome::render_title_bar`.
use super::primitives::{self, ButtonVariant, StatusKind, status_indicator, tooltip_view};
use super::*;
use gpui::{AnyElement, ClickEvent};
use wks_native::background_tasks::{self as bg, Kind};

/// "3 subagents", "1 shell task", "2 background tasks": the running work
/// in words. `None` when nothing is running.
pub(super) fn running_words(tasks: &[bg::Task], fallback: u32) -> Option<String> {
    let running: Vec<&bg::Task> = tasks.iter().filter(|t| t.running()).collect();
    let n = if tasks.is_empty() {
        fallback as usize
    } else {
        running.len()
    };
    if n == 0 {
        return None;
    }
    let kind = running.first().map(|t| t.kind);
    let same = kind.is_some() && running.iter().all(|t| Some(t.kind) == kind);
    let (one, many) = match kind.filter(|_| same) {
        Some(Kind::Agent | Kind::Teammate | Kind::Remote) => ("subagent", "subagents"),
        Some(Kind::Shell) => ("shell task", "shell tasks"),
        Some(Kind::Workflow) => ("workflow", "workflows"),
        _ => ("background task", "background tasks"),
    };
    Some(format!("{n} {}", if n == 1 { one } else { many }))
}

/// The session's access mode in a word or two; "Unknown" when the provider
/// has not reported one this app offers.
pub(super) fn access_word(session: &Session) -> (&'static str, Option<Permission>) {
    match Permission::from_wire(session.provider_id(), session.access_mode()) {
        Some(Permission::Ask) => ("Ask", Some(Permission::Ask)),
        Some(Permission::AcceptEdits) => ("Edits", Some(Permission::AcceptEdits)),
        Some(Permission::Plan) => ("Plan", Some(Permission::Plan)),
        Some(Permission::FullAccess) => ("Full access", Some(Permission::FullAccess)),
        None => ("Unknown", None),
    }
}

impl Workspace {
    /// Where the header's fade turns from solid canvas to clear: solid
    /// below the capsule, so nothing shows through or around it, then a
    /// short fade. v1 keeps its gentler 60%.
    pub(super) fn title_fade_stop(&self) -> f32 {
        if !self.design().is_v2() {
            return 0.6;
        }
        let height = f32::from(self.header_bounds.size.height);
        let fade = self.tokens().space.md;
        if height <= fade {
            0.6
        } else {
            1. - fade / height
        }
    }

    pub(super) fn render_title_bar_v2(
        &self,
        narrow: bool,
        enabled: bool,
        title: &str,
        session: Option<&Session>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let height = t.layout.control_height + t.space.xs;
        div()
            .id("title-bar")
            .debug_selector(|| "title-bar".into())
            .occlude()
            .max_w_full()
            .min_w_0()
            .h(px(height))
            .pl(px(t.space.lg))
            .pr(px(t.space.xs))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .rounded_full()
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .shadow(primitives::elevation(&p, t.elevation.floating))
            .when_some(session, |d, session| {
                let kind = if !self.view.connected {
                    StatusKind::Idle
                } else if self.paused(session) {
                    StatusKind::Paused
                } else {
                    match super::attention::bucket(session, false) {
                        super::attention::Bucket::NeedsYou => StatusKind::NeedsYou,
                        super::attention::Bucket::Working => StatusKind::Working,
                        _ => StatusKind::Idle,
                    }
                };
                let (label, _) = if self.view.connected {
                    self.status_of(session, p)
                } else {
                    ("Offline", p.muted)
                };
                d.child(
                    div()
                        .id("title-status")
                        .flex_shrink_0()
                        .tooltip(tooltip_view(label.to_owned(), None, &p))
                        .child(status_indicator(kind, &p)),
                )
                .when(!narrow, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(t.layout.menu_width / 2.))
                            .truncate()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(self.project_name(&session.cwd)),
                    )
                    .child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child("/"),
                    )
                })
            })
            .child(
                div()
                    .id("title-text")
                    .debug_selector(|| "title-text".into())
                    .min_w_0()
                    .max_w(px(if narrow {
                        t.layout.menu_width
                    } else {
                        t.layout.dialog_width - t.space.xl * 5.
                    }))
                    .truncate()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.title_reveal.tap(std::time::Instant::now());
                        cx.notify();
                    }))
                    .child(title.to_owned()),
            )
            .when_some(session, |d, session| {
                let model_enabled = enabled && self.supported_session();
                let open = self.shell.model_popover;
                let effort = (!session.effort.is_empty())
                    .then(|| wks_native::launch::effort_label(&session.effort));
                let model = session.display_model();
                let model = if model.is_empty() {
                    "Default model".to_owned()
                } else {
                    model
                };
                let (access, mode) = access_word(session);
                let full = mode == Some(Permission::FullAccess);
                let chip = |id: &'static str| {
                    chrome::interactive_control(div().id(id), p, model_enabled)
                        .debug_selector(move || id.into())
                        .relative()
                        .flex_shrink_0()
                        .h(px(t.layout.control_height_sm))
                        .px(px(t.space.md))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .font_weight(FontWeight(t.weight.medium))
                        .when(open, |d| d.bg(rgb(p.selected)))
                        .when(!open && model_enabled, |d| {
                            d.hover(move |s| s.bg(rgb(p.roles.hover)))
                        })
                        .when(model_enabled, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.open_model_popover(window, cx)
                            }))
                        })
                };
                d.when(!narrow, |d| {
                    d.child(
                        chip("title-model")
                            .bg(primitives::tonal(&p, t.wash.rest))
                            .max_w(px(t.layout.menu_width))
                            .tooltip(tooltip_view(
                                "Model, effort and access",
                                self.settings.vim_navigation.then_some("g m"),
                                &p,
                            ))
                            .child(super::shell::neutral_glyph(session.provider_id(), t.icon.xs, p))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(p.roles.text_secondary))
                                    .child(model),
                            )
                            .when_some(effort, |d, effort| {
                                d.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(rgb(p.roles.text_tertiary))
                                        .child(format!("· {effort}")),
                                )
                            })
                            .when(model_enabled, |d| {
                                d.child(
                                    Icon::new(IconName::ChevronDown)
                                        .size(px(t.icon.xs))
                                        .text_color(rgb(p.roles.text_tertiary)),
                                )
                            })
                            .child(chrome::measure(
                                cx.entity().downgrade(),
                                |this: &mut Self, bounds, _| {
                                    let changed = this.shell.model_anchor != bounds;
                                    this.shell.model_anchor = bounds;
                                    changed
                                },
                            )),
                    )
                })
                // The highest-stakes setting is always in view.
                .when(self.supported_session(), |d| {
                    d.child(
                        chip("title-access")
                            .tooltip(tooltip_view(
                                match mode {
                                    Some(mode) => format!("Access: {} · {}", mode.label(), mode.description()),
                                    None => "Access: unknown · the agent has not reported a mode this app offers".into(),
                                },
                                None,
                                &p,
                            ))
                            .text_color(rgb(if full {
                                p.warning
                            } else {
                                p.roles.text_tertiary
                            }))
                            .child(
                                Icon::new(if full {
                                    IconName::TriangleAlert
                                } else {
                                    IconName::Eye
                                })
                                .size(px(t.icon.xs)),
                            )
                            .child(access),
                    )
                })
            })
            .children(
                session
                    .map(|s| self.status_chips(s, narrow, cx))
                    .unwrap_or_default(),
            )
            .children(session.and_then(|s| self.render_tasks_words(s, cx)))
    }

    /// The background-work chip, in words ("3 subagents").
    fn render_tasks_words(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        if session.tasks.is_empty() && session.background_tasks == 0 {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let words = running_words(&session.tasks, session.background_tasks);
        let running = words.is_some();
        let label = words.unwrap_or_else(|| "Background tasks".into());
        Some(
            chrome::interactive_control(div().id("title-tasks"), p, true)
                .debug_selector(|| "title-tasks".into())
                .flex_shrink_0()
                .h(px(t.layout.control_height_sm))
                .px(px(t.space.md))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .text_size(px(t.text.meta))
                .font_weight(FontWeight(t.weight.medium))
                .text_color(rgb(p.roles.text_secondary))
                .when(self.tasks.open, |d| d.bg(rgb(p.selected)))
                .hover(move |s| s.bg(rgb(p.roles.hover)))
                .when(running, |d| {
                    d.child(status_indicator(StatusKind::Working, &p))
                })
                .child(label)
                .tooltip(tooltip_view("Show background tasks", None, &p))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_tasks(window, cx)))
                .into_any_element(),
        )
    }

    /// The session's tools in labelled groups (review · session · agent),
    /// each tooltip naming its shortcut. `floating` stacks them in the rail.
    pub(super) fn chat_actions_v2(
        &self,
        enabled: bool,
        floating: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let selected = self.view.selected.is_some();
        let model_enabled = enabled && self.supported_session();
        let vim = self.settings.vim_navigation;
        let handoff_enabled = self.supported_session() && self.view.connected && !self.demo;
        let handoff_label = match self.selected_session().map(|s| s.provider.as_str()) {
            Some("codex") => "Continue with Claude",
            Some("claude" | "") => "Continue with Codex",
            _ => "Continue with another agent",
        };
        let tool = |id: &'static str,
                    icon: IconName,
                    tip: &'static str,
                    keys: Option<&'static str>,
                    on: bool| {
            primitives::icon_button_with_shortcut(id, icon, tip, keys, ButtonVariant::Ghost, on, &p)
                .debug_selector(move || id.into())
        };
        let page = |screen: Screen| {
            cx.listener(move |this: &mut Self, event: &ClickEvent, window, cx| {
                if floating || this.title_action_allowed(event) {
                    this.open_feature(screen, window, cx)
                }
            })
        };
        let separator = || {
            div().flex_shrink_0().bg(rgb(p.border)).map(|d| {
                if floating {
                    d.h(px(t.stroke.hairline))
                        .w(px(t.icon.lg))
                        .my(px(t.space.xs))
                } else {
                    d.w(px(t.stroke.hairline))
                        .h(px(t.icon.lg))
                        .mx(px(t.space.xs))
                }
            })
        };
        let changes = tool(
            "open-changes",
            IconName::Replace,
            "Changes",
            vim.then_some("g d"),
            selected,
        )
        .when(selected, |d| d.on_click(page(Screen::Changes)));
        let files = tool(
            "open-editor",
            IconName::FolderOpen,
            "Files and editor",
            Some("Ctrl Shift E"),
            selected,
        )
        .when(selected, |d| {
            d.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if floating || this.title_action_allowed(event) {
                    this.open_selected_editor(window, cx)
                }
            }))
        });
        let terminal = tool(
            "open-terminal",
            IconName::SquareTerminal,
            if self.terminal.open {
                "Hide terminal"
            } else {
                "Terminal in this session's folder"
            },
            Some("Ctrl `"),
            selected,
        )
        .when(self.terminal.open, |d| {
            d.bg(rgb(p.selected)).text_color(rgb(p.text))
        })
        .when(selected, |d| {
            d.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if floating || this.title_action_allowed(event) {
                    this.toggle_terminal(window, cx)
                }
            }))
        });
        let history = tool(
            "open-history",
            IconName::BookOpen,
            "Conversation history",
            None,
            selected,
        )
        .when(selected, |d| d.on_click(page(Screen::History)));
        let details = tool(
            "open-session",
            IconName::Info,
            "Session details",
            vim.then_some("g e"),
            selected,
        )
        .when(selected, |d| d.on_click(page(Screen::Session)));
        let model = tool(
            "open-model",
            IconName::Settings2,
            "Model, effort and access",
            vim.then_some("g m"),
            model_enabled,
        )
        .when(model_enabled, |d| {
            d.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if floating || this.title_action_allowed(event) {
                    this.open_model_popover(window, cx)
                }
            }))
        });
        let handoff = tool(
            "open-handoff",
            IconName::ArrowRight,
            handoff_label,
            None,
            handoff_enabled,
        )
        .when(handoff_enabled, |d| d.on_click(page(Screen::Handoff)));
        div()
            .flex()
            .when(floating, |d| d.flex_col())
            .items_center()
            .gap(px(t.space.xxs))
            .flex_shrink_0()
            .child(changes)
            .child(files)
            .child(terminal)
            .child(separator())
            .child(history)
            .child(details)
            .child(separator())
            .child(model)
            .child(handoff)
    }

    /// The tools docked to the window's right edge, between the title and
    /// the composer: a flat strip, not a floating pill.
    pub(super) fn render_tool_rail_v2(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        div()
            .absolute()
            .top(self.header_bounds.size.height)
            .bottom(self.composer_dock_bounds.size.height)
            .right_0()
            .flex()
            .items_center()
            .child(
                div()
                    .debug_selector(|| "tool-rail".into())
                    .occlude()
                    .py(px(t.space.sm))
                    .px(px(t.space.xs))
                    .rounded_l(px(t.radius.panel))
                    .bg(rgb(p.base))
                    .border_l(px(t.stroke.hairline))
                    .border_t(px(t.stroke.hairline))
                    .border_b(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
                    .child(self.chat_actions_v2(enabled, true, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(kind: &str, status: &str) -> bg::Task {
        bg::Task {
            kind: Kind::of(kind),
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn running_work_is_counted_in_words() {
        assert_eq!(running_words(&[], 0), None);
        assert_eq!(running_words(&[], 2).as_deref(), Some("2 background tasks"));
        let agents = [
            task("local_agent", "running"),
            task("local_agent", "running"),
        ];
        assert_eq!(running_words(&agents, 0).as_deref(), Some("2 subagents"));
        let one = [
            task("local_bash", "running"),
            task("local_agent", "completed"),
        ];
        assert_eq!(running_words(&one, 0).as_deref(), Some("1 shell task"));
        let mixed = [
            task("local_bash", "running"),
            task("local_agent", "running"),
        ];
        assert_eq!(
            running_words(&mixed, 0).as_deref(),
            Some("2 background tasks")
        );
        let done = [task("local_bash", "completed")];
        assert_eq!(running_words(&done, 0), None);
    }
}
