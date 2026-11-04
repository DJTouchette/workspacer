//! Background tasks under design v2: one StatusBadge per task, long
//! commands cut with a tooltip and a Copy button, failed tasks offering
//! "Open log" and "Retry", Stop behind a confirmation, and the log in the
//! v2 card and code surface.
use super::*;
use crate::ui::primitives::{self, ButtonSize, ButtonVariant, StatusBadge, StatusKind};

/// The task's state in the shared status vocabulary.
pub(super) fn task_kind(task: &bg::Task) -> StatusKind {
    match task.status.as_str() {
        "" | "running" => StatusKind::Working,
        "pending" => StatusKind::Queued,
        "completed" => StatusKind::Done,
        "failed" => StatusKind::Failed,
        _ => StatusKind::Idle,
    }
}

/// `Shell · 12m 44s · pid 48213`: the status is the badge's to say.
pub(super) fn meta_v2(task: &bg::Task, now: i64, pid: bool) -> String {
    let mut parts = vec![task.kind.label().to_owned()];
    if let Some(ms) = task.elapsed_ms(now) {
        parts.push(timing::duration_label(ms));
    }
    if let (Some(pid), true, true) = (task.pid, task.running(), pid) {
        parts.push(format!("pid {pid}"));
    }
    parts.join(" · ")
}

/// What Retry drafts for the agent: tasks are the agent's to run, so the
/// composer gets the request and the user sends it.
pub(super) fn retry_prompt(task: &bg::Task) -> String {
    let title = task.title();
    let reason = if task.error.trim().is_empty() {
        String::new()
    } else {
        format!(" It failed: {}", task.error.trim())
    };
    match task.kind {
        Kind::Shell => format!("Run `{title}` again in the background.{reason}"),
        _ => format!("Retry the background task \"{title}\".{reason}"),
    }
}

impl Workspace {
    /// Put a retry request in the composer, focused, for the user to send.
    fn retry_task(&mut self, task: &bg::Task, window: &mut Window, cx: &mut Context<Self>) {
        let text = retry_prompt(task);
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        self.tasks.notice = Some((
            "Retry is in the composer: send it to ask the agent.".into(),
            Tone::Info,
        ));
        cx.notify();
    }

    pub(super) fn render_task_row_v2(
        &self,
        ix: usize,
        task: &bg::Task,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let opens = task.shows_log() || task.subagent_id.is_some();
        let confirming = self.tasks.confirm_stop.as_deref() == Some(task.id.as_str());
        let kind = task_kind(task);
        let row = task.clone();
        let title = task.title();
        let shell = task.kind == Kind::Shell;
        let failed = task.failed();
        let copy = shell.then(|| {
            let command = title.clone();
            primitives::icon_button(
                ("task-copy", ix),
                IconName::Copy,
                "Copy command",
                ButtonVariant::Ghost,
                true,
                &p,
            )
            .debug_selector({
                let id = task.id.clone();
                move || format!("task-copy-{id}")
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.clone()));
            })
        });
        let failed_actions = failed.then(|| {
            let (log_task, retry) = (task.clone(), task.clone());
            div()
                .pt(px(t.space.xs))
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .when(task.shows_log(), |d| {
                    d.child(
                        primitives::button(
                            ("task-open-log", ix),
                            "Open log",
                            Some(IconName::SquareTerminal),
                            ButtonVariant::Secondary,
                            ButtonSize::Small,
                            true,
                            &p,
                        )
                        .debug_selector({
                            let id = task.id.clone();
                            move || format!("task-open-log-{id}")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_task_log(&log_task.id, log_task.first_source(), cx)
                        })),
                    )
                })
                .child(
                    primitives::with_tooltip(
                        primitives::button(
                            ("task-retry", ix),
                            "Retry",
                            Some(IconName::Redo),
                            ButtonVariant::Ghost,
                            ButtonSize::Small,
                            true,
                            &p,
                        ),
                        "Ask the agent to run it again (drafted in the composer)",
                        None,
                        &p,
                    )
                    .debug_selector({
                        let id = task.id.clone();
                        move || format!("task-retry-{id}")
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.retry_task(&retry, window, cx)
                    })),
                )
        });
        let detail = if failed && !task.error.trim().is_empty() {
            Some(task.error.trim().to_owned())
        } else {
            (!task.summary.is_empty()).then(|| task.summary.clone())
        };
        div()
            .id(("task-row", ix))
            .debug_selector({
                let id = task.id.clone();
                move || format!("task-row-{id}")
            })
            .px(px(t.space.md))
            .py(px(t.space.md))
            .flex()
            .items_start()
            .gap(px(t.space.md))
            .rounded(px(t.radius.control))
            .when(opens, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(rgb(p.roles.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_task(&row, cx)))
            })
            .child(
                div()
                    .h(px(t.text.body * t.text.line_height))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .child(
                        Icon::new(kind_icon(task.kind))
                            .size(px(t.icon.md))
                            .text_color(rgb(p.roles.text_tertiary)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .child(
                        primitives::truncated(("task-title", ix), title, &p)
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.text))
                            .when(shell, |d| d.font_family(mono_font())),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .min_w_0()
                            .child(
                                StatusBadge::new(kind)
                                    .label(task.status_label().to_owned())
                                    .render(&p)
                                    .debug_selector({
                                        let id = task.id.clone();
                                        move || format!("task-status-{id}")
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(t.text.caption))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child(meta_v2(task, now, true)),
                            ),
                    )
                    .when_some(detail, |d, detail| {
                        d.child(
                            primitives::truncated(("task-detail", ix), detail, &p)
                                .text_size(px(t.text.caption))
                                .text_color(rgb(p.roles.text_secondary)),
                        )
                    })
                    .children(failed_actions),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xxs))
                    .children(copy)
                    .when(task.running(), |d| {
                        d.child(self.render_stop_control_v2(
                            &task.id,
                            confirming,
                            "task-stop",
                            ix,
                            cx,
                        ))
                    })
                    .when(opens && !task.running() && !failed, |d| {
                        d.child(
                            div()
                                .size(px(t.layout.icon_button))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::ChevronRight)
                                        .size(px(t.icon.sm))
                                        .text_color(rgb(p.roles.text_tertiary)),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Stop, then "Keep" / "Stop task" in place, so the user sees which
    /// task they are stopping.
    pub(super) fn render_stop_control_v2(
        &self,
        task: &str,
        confirming: bool,
        prefix: &'static str,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let id = task.to_owned();
        if confirming {
            let stop_id = id.clone();
            return div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .child(
                    primitives::button(
                        (prefix, ix * 3 + 2),
                        "Keep",
                        None,
                        ButtonVariant::Ghost,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.tasks.confirm_stop = None;
                        cx.notify();
                    })),
                )
                .child(
                    primitives::button(
                        (prefix, ix * 3 + 1),
                        "Stop task",
                        None,
                        ButtonVariant::DangerGhost,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector({
                        let id = id.clone();
                        move || format!("{prefix}-confirm-{id}")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.stop_task(&stop_id, cx)
                    })),
                )
                .into_any_element();
        }
        primitives::with_tooltip(
            primitives::button(
                (prefix, ix * 3),
                "Stop",
                None,
                ButtonVariant::Ghost,
                ButtonSize::Small,
                true,
                &p,
            ),
            "Stop this background task",
            None,
            &p,
        )
        .debug_selector({
            let id = id.clone();
            move || format!("{prefix}-{id}")
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.tasks.confirm_stop = Some(id.clone());
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn render_tasks_header_v2(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let log_task = self.tasks.log.as_ref().and_then(|log| {
            self.task_session()
                .and_then(|s| s.tasks.iter().find(|t| t.id == log.task))
                .cloned()
        });
        let in_log = self.tasks.log.is_some();
        let running = self.task_session().map_or(0, |s| bg::running(&s.tasks));
        let title: SharedString = match &log_task {
            Some(task) => task.title().into(),
            None if in_log => "Task log".into(),
            None => "Background tasks".into(),
        };
        div()
            .flex_shrink_0()
            .h(px(t.panels.header_height))
            .pl(px(if in_log { t.space.sm } else { t.space.lg }))
            .pr(px(t.space.sm))
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .border_b(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .when(in_log, |d| {
                d.child(
                    primitives::icon_button(
                        "tasks-back",
                        IconName::ArrowLeft,
                        "All background tasks",
                        ButtonVariant::Ghost,
                        true,
                        &p,
                    )
                    .debug_selector(|| "tasks-back".into())
                    .on_click(cx.listener(|this, _, _, cx| this.show_task_list(cx))),
                )
            })
            .child(
                primitives::truncated("tasks-title", title, &p)
                    .flex_1()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .when(
                        log_task.as_ref().is_some_and(|t| t.kind == Kind::Shell),
                        |d| d.font_family(mono_font()),
                    ),
            )
            .when(!in_log && running > 0, |d| {
                d.child(
                    StatusBadge::new(StatusKind::Working)
                        .label(format!("{running} running"))
                        .render(&p),
                )
            })
            .child(
                primitives::icon_button(
                    "tasks-close",
                    IconName::Close,
                    "Close",
                    ButtonVariant::Ghost,
                    true,
                    &p,
                )
                .debug_selector(|| "tasks-close".into())
                .on_click(cx.listener(|this, _, _, cx| this.close_tasks(cx))),
            )
    }

    /// The log's status line under v2: badge, meta, process, Stop.
    pub(super) fn render_task_log_meta_v2(
        &self,
        log: &Log,
        task: Option<&bg::Task>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let now = timing::now_ms();
        let running = task.is_some_and(bg::Task::running);
        let confirming = self.tasks.confirm_stop.as_deref() == Some(log.task.as_str());
        let copy = task.filter(|t| t.kind == Kind::Shell).map(|task| {
            let command = task.title();
            primitives::icon_button(
                "task-log-copy",
                IconName::Copy,
                "Copy command",
                ButtonVariant::Ghost,
                true,
                &p,
            )
            .debug_selector(|| "task-log-copy".into())
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.clone()))
            })
        });
        div()
            .px(px(t.space.lg))
            .py(px(t.space.md))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .when_some(task, |d, task| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(t.space.md))
                                .min_w_0()
                                .child(
                                    StatusBadge::new(task_kind(task))
                                        .label(task.status_label().to_owned())
                                        .render(&p),
                                )
                                .child(
                                    div()
                                        .debug_selector(|| "task-log-meta".into())
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(t.text.caption))
                                        .text_color(rgb(p.roles.text_tertiary))
                                        .child(meta_v2(
                                            task,
                                            now,
                                            log.process.is_none() || !running,
                                        )),
                                ),
                        )
                    })
                    .when_some(log.process.filter(|_| running), |d, process| {
                        d.child(
                            div()
                                .debug_selector(|| "task-log-process".into())
                                .truncate()
                                .text_size(px(t.text.caption))
                                .text_color(rgb(p.roles.text_tertiary))
                                .child(process_line(&process)),
                        )
                    }),
            )
            .children(copy)
            .when(running, |d| {
                d.child(self.render_stop_control_v2(&log.task, confirming, "task-log-stop", 0, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_speak_the_status_vocabulary() {
        let task = |status: &str| bg::Task {
            status: status.into(),
            ..Default::default()
        };
        assert_eq!(task_kind(&task("running")), StatusKind::Working);
        assert_eq!(task_kind(&task("")), StatusKind::Working);
        assert_eq!(task_kind(&task("pending")), StatusKind::Queued);
        assert_eq!(task_kind(&task("completed")), StatusKind::Done);
        assert_eq!(task_kind(&task("failed")), StatusKind::Failed);
        assert_eq!(task_kind(&task("killed")), StatusKind::Idle);
    }

    #[test]
    fn retry_asks_the_agent_with_the_reason() {
        let failed = bg::Task {
            kind: Kind::Shell,
            status: "failed".into(),
            description: "cargo test --locked".into(),
            error: "exit code 101".into(),
            ..Default::default()
        };
        let prompt = retry_prompt(&failed);
        assert!(prompt.contains(&failed.title()), "{prompt}");
        assert!(prompt.ends_with("It failed: exit code 101"), "{prompt}");
        let meta = meta_v2(
            &bg::Task {
                started_at: Some(0),
                pid: Some(7),
                ..failed.clone()
            },
            1_000,
            true,
        );
        assert!(!meta.contains("Failed"), "the badge says it: {meta}");
    }
}
