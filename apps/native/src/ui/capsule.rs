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

/// The failed tasks the title's red chip still names: not acknowledged,
/// newest first (by end, then start).
pub(super) fn unacknowledged_failures<'a>(
    tasks: &'a [bg::Task],
    acknowledged: &std::collections::BTreeSet<String>,
) -> Vec<&'a bg::Task> {
    let mut failed: Vec<&bg::Task> = tasks
        .iter()
        .filter(|t| t.failed() && !acknowledged.contains(&t.id))
        .collect();
    failed.sort_by_key(|t| std::cmp::Reverse((t.ended_at, t.started_at)));
    failed
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
        // r2-shell: a 32px fade (`layout.fade`) under the capsule.
        let fade = self.tokens().layout.fade;
        if height <= fade {
            0.6
        } else {
            1. - fade / height
        }
    }

    /// The v2 title: status, the project crumb (it gives way first), the
    /// session title (primary, truncated last), the access mode once it is
    /// known, and background work in words. The model lives in the
    /// composer's chip, the one place it is shown and changed.
    pub(super) fn render_title_bar_v2(
        &self,
        narrow: bool,
        _enabled: bool,
        title: &str,
        session: Option<&Session>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let height = t.layout.control_height + t.space.xs;
        let now = timing::now_ms();
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
            .gap(px(t.space.sm))
            .rounded_full()
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .shadow(primitives::elevation(&p, t.elevation.floating))
            .when_some(session, |d, session| {
                let kind = if !self.view.connected {
                    StatusKind::Offline
                } else if self.paused(session) {
                    StatusKind::Paused
                } else if super::attention::needs_you(session) {
                    StatusKind::NeedsYou
                } else if self.stalled_for(session, now).is_some() {
                    StatusKind::NotResponding
                } else {
                    match super::attention::bucket(session, false) {
                        super::attention::Bucket::Working => StatusKind::Working,
                        _ => StatusKind::Idle,
                    }
                };
                // r3-foundation: the one lexicon, never a provider's word.
                let label = kind.label();
                d.child(
                    div()
                        .id("title-status")
                        .flex_shrink_0()
                        .tooltip(tooltip_view(label, None, &p))
                        .child(status_indicator(kind, &p)),
                )
                .when(!narrow, |d| {
                    let project = self.project_name(&session.cwd);
                    d.child(
                        div()
                            .id("title-crumb")
                            .debug_selector(|| "title-crumb".into())
                            .min_w_0()
                            .max_w(px(t.layout.menu_width / 2.))
                            // Gives way long before the title does.
                            .map(|mut d| {
                                d.style().flex_shrink = Some(1000.);
                                d
                            })
                            .truncate()
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_tertiary))
                            .tooltip(tooltip_view(project.clone(), None, &p))
                            .child(project),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(t.text.body))
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
                        t.layout.dialog_width + t.layout.menu_width / 2.
                    }))
                    .truncate()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.title_reveal.tap(std::time::Instant::now());
                        cx.notify();
                    }))
                    .child(title.to_owned()),
            )
            .when_some(session, |d, session| {
                let (access, mode) = access_word(session);
                let full = mode == Some(Permission::FullAccess);
                // Hidden until the agent reports a mode this app offers:
                // "unknown" is not something to show or choose.
                d.when_some(
                    mode.filter(|_| self.supported_session() && !narrow),
                    |d, mode| {
                        d.child(
                            chrome::interactive_control(
                                div().id("title-access"),
                                p,
                                self.supported_session(),
                            )
                            .debug_selector(|| "title-access".into())
                            .flex_shrink_0()
                            .h(px(t.layout.control_height_sm))
                            .px(px(t.space.md))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .gap(px(t.space.xs))
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(if full {
                                p.warning
                            } else {
                                p.roles.text_secondary
                            }))
                            .hover(move |s| s.bg(rgb(p.roles.hover)))
                            .tooltip(tooltip_view(
                                format!(
                                    "Access: {} · {} · click to change",
                                    mode.label(),
                                    mode.description()
                                ),
                                None,
                                &p,
                            ))
                            .when(full, |d| {
                                d.child(Icon::new(IconName::TriangleAlert).size(px(t.icon.xs)))
                            })
                            .child(access)
                            .on_click(cx.listener(
                                |this, _, window, cx| this.open_model_popover(window, cx),
                            )),
                        )
                    },
                )
            })
            .children(
                session
                    .map(|s| self.status_chips(s, narrow, cx))
                    .unwrap_or_default(),
            )
            .children(session.and_then(|s| self.render_tasks_words(s, cx)))
    }

    /// Background work in the title (R3.1): neutral words for what runs
    /// ("3 subagents", "3 running"), and apart from it a red chip that names
    /// what broke ("cargo test failed"). The red chip opens that task; its ✕
    /// acknowledges it and the chip goes for this run. Healthy work is never
    /// red.
    pub(super) fn render_tasks_words(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if session.tasks.is_empty() && session.background_tasks == 0 {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let failed = unacknowledged_failures(&session.tasks, &self.shell.acknowledged_failures);
        let label = running_words(&session.tasks, session.background_tasks)
            .unwrap_or_else(|| "Background tasks".into());
        let words = chrome::interactive_control(div().id("title-tasks"), p, true)
            .debug_selector(|| "title-tasks".into())
            .flex_shrink_0()
            .h(px(t.layout.control_height_sm))
            .px(px(t.space.md))
            .rounded_full()
            .flex()
            .items_center()
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .map(super::shell::tabular)
            .text_color(rgb(p.roles.text_secondary))
            .when(self.tasks.open, |d| d.bg(rgb(p.selected)))
            .hover(move |s| s.bg(rgb(p.roles.hover)))
            .child(label)
            .tooltip(tooltip_view("Show background tasks", None, &p))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_tasks(window, cx)));
        Some(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(t.space.xs))
                .child(words)
                .children(
                    failed
                        .first()
                        .map(|task| self.render_failed_chip(task, failed.len(), cx)),
                )
                .into_any_element(),
        )
    }

    /// The red chip naming the newest unacknowledged failed task: the name
    /// opens it (its log, or an agent's conversation), the ✕ acknowledges it.
    fn render_failed_chip(
        &self,
        task: &bg::Task,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let name = task.title();
        let mut label = wks_native::format::failed_label(&name, 28);
        if count > 1 {
            label.push_str(&format!(" · +{}", count - 1));
        }
        let error = task.error.trim();
        let tip = if error.is_empty() {
            format!("{name} failed · open to see why")
        } else {
            format!("{name} failed · {error}")
        };
        let open_id = task.id.clone();
        let ack_id = task.id.clone();
        chrome::interactive_control(div().id("title-failed"), p, true)
            .debug_selector(|| "title-failed".into())
            .flex_shrink_0()
            .h(px(t.layout.control_height_sm))
            .pl(px(t.space.md))
            .pr(px(t.space.xxs))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(p.roles.status.fault))
            .bg(primitives::wash(p.roles.status.fault, t.wash.tone_rest))
            .hover(move |s| s.bg(primitives::wash(p.roles.status.fault, t.wash.tone_hover)))
            .children(primitives::status_glyph(StatusKind::Failed, &p))
            .child(label)
            .tooltip(tooltip_view(tip, None, &p))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_task_by_id(&open_id, window, cx)),
            )
            .child(
                chrome::interactive_control(div().id("title-failed-ack"), p, true)
                    .debug_selector(|| "title-failed-ack".into())
                    .size(px(t.layout.control_height_sm - t.space.sm))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .hover(move |s| s.bg(primitives::wash(p.roles.status.fault, t.wash.tone_hover)))
                    .tooltip(tooltip_view("Dismiss · you have seen it", None, &p))
                    .child(Icon::new(IconName::Close).size(px(t.icon.xs)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.shell.acknowledged_failures.insert(ack_id.clone());
                        cx.notify();
                    })),
            )
            .into_any_element()
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
        let vim = self.vim();
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
            primitives::icon_button_with_shortcut(
                id,
                icon,
                tip,
                keys,
                ButtonVariant::Tertiary,
                on,
                &p,
            )
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
        // r2-shell: badges where there is state.
        let changed = self
            .selected_session()
            .and_then(|s| self.changes_count(&s.cwd))
            .filter(|n| *n > 0)
            .map(|n| {
                if n > 99 {
                    "99+".to_owned()
                } else {
                    n.to_string()
                }
            });
        let changes = tool_badge(changes, changed.filter(|_| floating), &p);
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
        let shell_running = self
            .view
            .selected
            .as_ref()
            .is_some_and(|id| self.view.terminals.contains_key(id));
        let terminal = tool_badge(
            terminal,
            (floating && shell_running && !self.terminal.open).then(String::new),
            &p,
        );
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

    /// The session's tools, docked to the conversation column's right edge
    /// just under the title (not floating at the window edge). The column's
    /// measure comes from the transcript (`reading_max`).
    pub(super) fn render_tool_rail_v2(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let column = self.reading_max().min(self.chat_width);
        let rail = t.layout.icon_button + 2. * t.space.xs + 2. * t.stroke.hairline;
        // The rail's left edge sits a step outside the column's text edge.
        let right = ((self.chat_width - column) / 2. + px(t.layout.chat_gutter)
            - px(t.space.lg + rail))
        .max(px(t.space.sm));
        div()
            .absolute()
            .top(self.header_bounds.size.height + px(t.space.md))
            .right(right)
            .child(
                div()
                    .debug_selector(|| "tool-rail".into())
                    .occlude()
                    .py(px(t.space.xs))
                    .px(px(t.space.xs))
                    .rounded(px(t.radius.panel))
                    .bg(rgb(p.surface))
                    .border(px(t.stroke.hairline))
                    .border_color(rgb(p.roles.card_border))
                    .child(self.chat_actions_v2(enabled, true, cx)),
            )
    }
}

/// A small count or dot on a tool's top-right corner.
fn tool_badge(control: Stateful<Div>, badge: Option<String>, p: &Palette) -> Stateful<Div> {
    let t = p.tokens;
    control.relative().when_some(badge, |d, badge| {
        d.child(
            div()
                .absolute()
                .top(-px(t.space.xxs))
                .right(-px(t.space.xxs))
                .map(|d| {
                    if badge.is_empty() {
                        d.top(px(t.space.xs))
                            .right(px(t.space.xs))
                            .size(px(t.layout.status_dot))
                            .rounded(px(t.radius.full))
                            .bg(rgb(p.roles.text_secondary))
                    } else {
                        d.child(primitives::count_badge(
                            badge,
                            primitives::CountTone::Neutral,
                            p,
                        ))
                    }
                }),
        )
    })
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

    // r3-foundation
    #[test]
    fn the_red_chip_names_the_newest_unacknowledged_failure() {
        let failed = |id: &str, ended: i64| bg::Task {
            id: id.into(),
            ended_at: Some(ended),
            ..task("local_bash", "failed")
        };
        let tasks = [
            failed("old", 10),
            task("local_bash", "running"),
            failed("new", 20),
        ];
        let mut acked = std::collections::BTreeSet::new();
        let ids = |acked: &std::collections::BTreeSet<String>| {
            unacknowledged_failures(&tasks, acked)
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&acked), ["new", "old"]);
        acked.insert("new".to_owned());
        assert_eq!(ids(&acked), ["old"]);
        acked.insert("old".to_owned());
        assert!(ids(&acked).is_empty());
        // The neutral words never count a failure as running.
        assert_eq!(running_words(&tasks, 0).as_deref(), Some("1 shell task"));
    }
}
