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
        "pending" => StatusKind::Idle,
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

/// r3-transcript: a log line that reports a failure (compiler errors,
/// failed tests, panics), highlighted in the failed task's log.
pub(super) fn failing_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("error")
        || lower.starts_with("fail")
        || lower.starts_with("test result: failed")
        || lower.contains("... failed")
        || lower.contains("panicked at")
        || lower.starts_with("fatal:")
        || (lower.contains("exited with code") && !lower.contains("code 0"))
        || lower.starts_with("✗")
        || lower.starts_with("×")
}

/// How many failing lines "Ask agent to fix" quotes.
const FIX_QUOTE_LINES: usize = 12;

/// What "Ask agent to fix" drafts: the failure, where its log is, and the
/// failing lines quoted, for the agent to fix rather than just re-run.
pub(super) fn fix_prompt(task: &bg::Task, lines: &[SharedString]) -> String {
    let title = task.title();
    let mut text = match task.kind {
        Kind::Shell => format!("The background command `{title}` failed."),
        _ => format!("The background task \"{title}\" failed."),
    };
    if !task.error.trim().is_empty() {
        text.push_str(&format!(" {}", task.error.trim()));
    }
    let failing: Vec<&str> = lines
        .iter()
        .map(|l| l.as_ref())
        .filter(|l| failing_line(l))
        .take(FIX_QUOTE_LINES)
        .collect();
    if !failing.is_empty() {
        text.push_str(&format!("\n\n```\n{}\n```", failing.join("\n")));
    }
    if let Some(file) = &task.redirect_file {
        text.push_str(&format!("\n\nThe full log is in {file}."));
    }
    text.push_str("\n\nFind the cause, fix it, and run it again.");
    text
}

impl Workspace {
    /// "Ask agent to fix": the failure and its failing lines in the
    /// composer, focused, for the user to send.
    fn ask_to_fix(&mut self, task: &bg::Task, window: &mut Window, cx: &mut Context<Self>) {
        let text = fix_prompt(task, &self.tasks.lines);
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        self.tasks.notice = Some((
            "The request is in the composer: send it to ask the agent.".into(),
            Tone::Info,
        ));
        cx.notify();
    }

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
                ButtonVariant::Tertiary,
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
                            ButtonVariant::Tertiary,
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
            // r2-pages: R2.5 failures are loud: an error wash and a red
            // stripe down the row's edge.
            .when(failed, |d| {
                d.relative()
                    .bg(primitives::wash(p.error, t.wash.tone_rest / 2.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(t.space.sm))
                            .bottom(px(t.space.sm))
                            .w(px(t.stroke.focus))
                            .rounded(px(t.radius.full))
                            .bg(rgb(p.error)),
                    )
            })
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
                                .text_color(rgb(if failed {
                                    p.error
                                } else {
                                    p.roles.text_secondary
                                })),
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
                        ButtonVariant::Tertiary,
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
                        ButtonVariant::Destructive,
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
                ButtonVariant::Tertiary,
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
        // r2-pages: a failure turns the header's count red.
        let failed = self
            .task_session()
            .map_or(0, |s| s.tasks.iter().filter(|t| t.failed()).count());
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
                        ButtonVariant::Tertiary,
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
            .when(!in_log && (running > 0 || failed > 0), |d| {
                let badge = match (running, failed) {
                    (0, f) => StatusBadge::new(StatusKind::Failed).label(format!("{f} failed")),
                    (r, 0) => StatusBadge::new(StatusKind::Working).label(format!("{r} running")),
                    (r, f) => StatusBadge::new(StatusKind::Failed)
                        .label(format!("{r} running · {f} failed")),
                };
                d.child(badge.render(&p).debug_selector(|| "tasks-count".into()))
            })
            .child(
                primitives::icon_button(
                    "tasks-close",
                    IconName::Close,
                    "Close",
                    ButtonVariant::Tertiary,
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
                ButtonVariant::Tertiary,
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
            // r3-transcript: a failure offers its recovery where it is read.
            .when_some(task.filter(|t| t.failed()).cloned(), |d, task| {
                let retry = task.clone();
                d.child(
                    primitives::with_tooltip(
                        primitives::button(
                            "task-log-retry-task",
                            "Retry",
                            Some(IconName::Redo),
                            ButtonVariant::Secondary,
                            ButtonSize::Small,
                            true,
                            &p,
                        ),
                        "Ask the agent to run it again (drafted in the composer)",
                        None,
                        &p,
                    )
                    .debug_selector(|| "task-log-retry-task".into())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.retry_task(&retry, window, cx)),
                    ),
                )
                .child(
                    primitives::with_tooltip(
                        primitives::button(
                            "task-log-fix",
                            "Ask agent to fix",
                            None,
                            ButtonVariant::Primary,
                            ButtonSize::Small,
                            true,
                            &p,
                        ),
                        "Draft a request with the failing lines for the agent to fix",
                        None,
                        &p,
                    )
                    .debug_selector(|| "task-log-fix".into())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.ask_to_fix(&task, window, cx)),
                    ),
                )
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
        assert_eq!(task_kind(&task("pending")), StatusKind::Idle);
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

    #[test]
    fn failing_lines_are_found_and_quoted_for_the_fix() {
        for line in [
            "error[E0599]: no function `open_last`",
            "test restores_the_last_workspace ... FAILED",
            "test result: FAILED. 0 passed; 1 failed",
            "thread 'main' panicked at src/main.rs:4:5",
            "FAIL src/app.test.ts",
        ] {
            assert!(failing_line(line), "{line}");
        }
        for line in [
            "test result: ok. 1 passed; 0 failed",
            "   Compiling wks-native v0.9.0",
            "running 1 test",
        ] {
            assert!(!failing_line(line), "{line}");
        }
        let task = bg::Task {
            kind: Kind::Shell,
            status: "failed".into(),
            description: "cargo test".into(),
            ..Default::default()
        };
        let lines: Vec<SharedString> = ["running 1 test", "test x ... FAILED", "ok"]
            .into_iter()
            .map(Into::into)
            .collect();
        let prompt = fix_prompt(&task, &lines);
        assert!(prompt.contains("`cargo test` failed"), "{prompt}");
        assert!(prompt.contains("test x ... FAILED"), "{prompt}");
        assert!(!prompt.contains("running 1 test"), "{prompt}");
    }
}
