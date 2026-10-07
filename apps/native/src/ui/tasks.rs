//! Background tasks of the selected session: a chip in the title bar, the
//! task list and a live, auto-following log. The panel takes the right-side
//! slot the file viewer uses — docked beside the conversation when there is
//! room, a modal sheet over it when there is not — and gives the slot back
//! whenever the viewer opens a file.
//!
//! Rows come from the session snapshot (`background_task_list`, Claude stream
//! sessions only). A shell's or workflow's log is read by task id through
//! `sessions.taskOutput`, from the tail and then from where the last read
//! stopped, once a second while the panel shows it. Agents open their own
//! conversation (the provider-native child view). Stop sends the CLI's own
//! `stop_task` request after a confirmation; it never signals a process.
use super::*;
use chrome::{Tone, interactive_control};
use gpui::{AnyElement, ListHorizontalSizingBehavior, ScrollStrategy, ScrollWheelEvent, deferred};
use gpui_component::tooltip::Tooltip;
use wks_native::background_tasks::{self as bg, Kind, Log};
use wks_native::features::Request;

/// Same slide as the file viewer.
const SLIDE: std::time::Duration = std::time::Duration::from_millis(240);
/// Characters of one log line drawn (the rest stays in the copy).
const MAX_LINE_CHARS: usize = 4_000;

#[derive(Default)]
pub(super) struct TasksUi {
    pub(super) open: bool,
    /// The session whose tasks are shown (follows the selection).
    session: String,
    /// The task whose log is shown; `None` shows the list.
    pub(super) log: Option<Log>,
    /// The newest `task-output` / `task-stop` answers already folded in.
    applied: u64,
    stop_seen: u64,
    lines: Arc<Vec<SharedString>>,
    widest: usize,
    scroll: gpui::UniformListScrollHandle,
    /// Keep the newest line in view; scrolling up pauses it.
    following: bool,
    /// The log read failed; polling waits for Try again.
    failed: Option<String>,
    /// A task awaiting the user's confirmation to stop.
    confirm_stop: Option<String>,
    notice: Option<(SharedString, Tone)>,
    opened: u64,
    clock: Option<Task<()>>,
}

fn kind_icon(kind: Kind) -> IconName {
    match kind {
        Kind::Shell => IconName::SquareTerminal,
        Kind::Agent => IconName::Bot,
        Kind::Teammate => IconName::User,
        Kind::Remote => IconName::Globe,
        Kind::Workflow => IconName::GalleryVerticalEnd,
        Kind::Other => IconName::LayoutDashboard,
    }
}

fn status_color(task: &bg::Task, p: Palette) -> u32 {
    if task.running() {
        p.busy
    } else if task.failed() {
        p.error
    } else {
        p.muted
    }
}

/// `Shell · Running · 2m 04s · pid 4242` (the pid only when no process line
/// below repeats it).
fn meta_line(task: &bg::Task, now: i64) -> String {
    meta_line_with(task, now, true)
}

fn meta_line_with(task: &bg::Task, now: i64, pid: bool) -> String {
    let mut parts = vec![task.kind.label().to_owned(), task.status_label().to_owned()];
    if let Some(ms) = task.elapsed_ms(now) {
        parts.push(timing::duration_label(ms));
    }
    if let (Some(pid), true, true) = (task.pid, task.running(), pid) {
        parts.push(format!("pid {pid}"));
    }
    parts.join(" · ")
}

/// `pid 4242 · running · 3.1% CPU · 48.2 MB · 2 processes`
fn process_line(process: &bg::Process) -> String {
    let mut parts = vec![format!("pid {}", process.pid)];
    if !process.alive {
        parts.push("exited".into());
        return parts.join(" · ");
    }
    parts.push("running".into());
    if let Some(cpu) = process.cpu_percent {
        parts.push(format!("{cpu:.1}% CPU"));
    }
    if let Some(rss) = process.rss_bytes {
        parts.push(bg::bytes_label(rss));
    }
    if let Some(n) = process.processes.filter(|n| *n > 1) {
        parts.push(format!("{n} processes"));
    }
    parts.join(" · ")
}

impl Workspace {
    fn task_session(&self) -> Option<&Session> {
        self.view
            .sessions
            .iter()
            .find(|s| s.id == self.tasks.session)
    }

    /// The chip opens and closes the panel for the selected session.
    pub(super) fn toggle_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tasks.open {
            self.close_tasks(cx);
            return;
        }
        let Some(id) = self.view.selected.clone() else {
            return;
        };
        // One panel at a time: the editor gives up the slot first (it asks
        // about unsaved edits, and keeps it if the user keeps editing).
        if self.viewer_in_slot() {
            self.close_file_viewer(window, cx);
            if self.viewer_in_slot() {
                return;
            }
        }
        self.tasks.open = true;
        self.tasks.session = id;
        self.tasks.log = None;
        self.tasks.confirm_stop = None;
        self.tasks.notice = None;
        self.tasks.failed = None;
        // Answers that arrived before this opening are not news.
        self.tasks.applied = self
            .view
            .requests
            .get("task-output")
            .map_or(0, |s| s.number);
        self.tasks.stop_seen = self.view.requests.get("task-stop").map_or(0, |s| s.number);
        self.tasks.opened += 1;
        self.ensure_task_clock(cx);
        cx.notify();
    }

    pub(super) fn close_tasks(&mut self, cx: &mut Context<Self>) {
        self.tasks.open = false;
        self.tasks.log = None;
        self.tasks.confirm_stop = None;
        self.tasks.clock = None;
        cx.notify();
    }

    /// A row: a shell/workflow opens its log here, an agent its conversation.
    fn open_task(&mut self, task: &bg::Task, cx: &mut Context<Self>) {
        if task.shows_log() {
            self.tasks.log = Some(Log::new(&self.tasks.session, &task.id));
            self.tasks.lines = Arc::default();
            self.tasks.widest = 0;
            self.tasks.following = true;
            self.tasks.failed = None;
            self.tasks.notice = None;
            self.poll_task_log(cx);
        } else if let Some(agent) = task.subagent_id.clone() {
            let parent = self.tasks.session.clone();
            self.open_sidebar_child(&parent, &agent, cx);
        }
        cx.notify();
    }

    fn show_task_list(&mut self, cx: &mut Context<Self>) {
        self.tasks.log = None;
        self.tasks.failed = None;
        cx.notify();
    }

    /// Read the open log onward from where it stands (the tail at first).
    pub(super) fn poll_task_log(&mut self, cx: &mut Context<Self>) {
        let Some(log) = &self.tasks.log else {
            return;
        };
        if log.done || self.tasks.failed.is_some() || !self.view.connected {
            return;
        }
        let request = Request::TaskOutput {
            session: log.session.clone(),
            task: log.task.clone(),
            offset: log.next_offset,
        };
        // Not `request()`: a once-a-second read must not clear notices.
        self.command(Command::Request(request), cx);
    }

    fn ensure_task_clock(&mut self, cx: &mut Context<Self>) {
        if self.tasks.clock.is_some() {
            return;
        }
        self.tasks.clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    if !this.tasks.open {
                        return false;
                    }
                    let reading = this
                        .view
                        .requests
                        .get("task-output")
                        .is_some_and(|s| s.loading);
                    if !reading {
                        this.poll_task_log(cx);
                    }
                    // Elapsed times tick.
                    cx.notify();
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Fold the controller's answers into the panel.
    pub(super) fn sync_tasks(&mut self, view: &View, cx: &mut Context<Self>) {
        if !self.tasks.open {
            return;
        }
        match &view.selected {
            None => {
                self.close_tasks(cx);
                return;
            }
            Some(id) if *id != self.tasks.session => {
                self.tasks.session = id.clone();
                self.tasks.log = None;
                self.tasks.confirm_stop = None;
                self.tasks.notice = None;
                self.tasks.failed = None;
            }
            Some(_) => {}
        }
        let mut read_more = false;
        if let Some(state) = view.requests.get("task-output")
            && !state.loading
            && state.number > self.tasks.applied
        {
            self.tasks.applied = state.number;
            if let (Some(log), Request::TaskOutput { session, task, .. }) =
                (&mut self.tasks.log, &state.request)
                && log.session == *session
                && log.task == *task
            {
                match &state.error {
                    Some(error) => self.tasks.failed = Some(error.clone()),
                    None => {
                        let changed = log.apply(&state.value);
                        read_more = log.behind();
                        if changed {
                            self.refresh_log_lines();
                        }
                    }
                }
            }
        }
        if let Some(state) = view.requests.get("task-stop")
            && !state.loading
            && state.number > self.tasks.stop_seen
        {
            self.tasks.stop_seen = state.number;
            if matches!(&state.request, Request::TaskStop { session, .. } if *session == self.tasks.session)
            {
                self.tasks.notice = Some(match &state.error {
                    Some(error) => (
                        format!("Could not stop the task: {error}").into(),
                        Tone::Error,
                    ),
                    None => (
                        "Stop requested. The task list updates when it ends.".into(),
                        Tone::Success,
                    ),
                });
            }
        }
        if read_more {
            self.poll_task_log(cx);
        }
    }

    fn refresh_log_lines(&mut self) {
        let Some(log) = &self.tasks.log else {
            return;
        };
        let lines: Vec<SharedString> = log
            .lines()
            .into_iter()
            .map(|l| SharedString::from(wks_native::transcript::head(l, MAX_LINE_CHARS)))
            .collect();
        self.tasks.widest = lines
            .iter()
            .enumerate()
            .max_by_key(|(_, l)| l.len())
            .map_or(0, |(ix, _)| ix);
        let count = lines.len();
        self.tasks.lines = Arc::new(lines);
        if self.tasks.following && count > 0 {
            self.tasks
                .scroll
                .scroll_to_item(count - 1, ScrollStrategy::Bottom);
        }
    }

    fn follow_log(&mut self, cx: &mut Context<Self>) {
        self.tasks.following = true;
        let count = self.tasks.lines.len();
        if count > 0 {
            self.tasks
                .scroll
                .scroll_to_item(count - 1, ScrollStrategy::Bottom);
        }
        cx.notify();
    }

    fn stop_task(&mut self, task: &str, cx: &mut Context<Self>) {
        self.tasks.confirm_stop = None;
        self.tasks.notice = Some(("Stopping…".into(), Tone::Loading));
        self.command(
            Command::Request(Request::TaskStop {
                session: self.tasks.session.clone(),
                task: task.to_owned(),
            }),
            cx,
        );
        cx.notify();
    }

    // ── Rendering ──────────────────────────────────────────────────────

    /// The title-bar chip: running count, or nothing when the session has no
    /// background work to show.
    pub(super) fn render_tasks_chip(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if session.tasks.is_empty() && session.background_tasks == 0 {
            return None;
        }
        let p = self.appearance.palette();
        let running = if session.tasks.is_empty() {
            session.background_tasks as usize
        } else {
            bg::running(&session.tasks)
        };
        let open = self.tasks.open;
        let tip: SharedString = match running {
            0 => "Background tasks".into(),
            1 => "1 background task running".into(),
            n => format!("{n} background tasks running").into(),
        };
        Some(
            interactive_control(div().id("title-tasks"), p, true)
                .debug_selector(|| "title-tasks".into())
                .flex_shrink_0()
                .px_2()
                .py(px(3.))
                .rounded_full()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(chrome::scale::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .bg(rgb(if open { p.selected } else { p.surface }))
                .text_color(rgb(if running > 0 { p.busy } else { p.muted }))
                .hover(|s| s.bg(rgb(p.border)))
                .child(Icon::new(IconName::LayoutDashboard).size(px(12.)))
                .child(running.to_string())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_tasks(window, cx)))
                .into_any_element(),
        )
    }

    fn render_task_row(
        &self,
        ix: usize,
        task: &bg::Task,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.appearance.palette();
        let opens = task.shows_log() || task.subagent_id.is_some();
        let confirming = self.tasks.confirm_stop.as_deref() == Some(task.id.as_str());
        let id = task.id.clone();
        let row = task.clone();
        div()
            .id(("task-row", ix))
            .debug_selector({
                let id = task.id.clone();
                move || format!("task-row-{id}")
            })
            .px_3()
            .py_2()
            .flex()
            .items_start()
            .gap_2()
            .rounded(px(p.control_radius))
            .when(opens, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(rgb(p.selected)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_task(&row, cx)))
            })
            .child(
                div().pt(px(2.)).flex_shrink_0().child(
                    Icon::new(kind_icon(task.kind))
                        .size(px(14.))
                        .text_color(rgb(status_color(task, p))),
                ),
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
                            .truncate()
                            .text_size(px(chrome::scale::BODY))
                            .text_color(rgb(p.text))
                            .child(task.title()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(meta_line(task, now)),
                    )
                    .when(!task.summary.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child(task.summary.clone()),
                        )
                    }),
            )
            .when(task.running(), |d| {
                d.child(self.render_stop_control(&id, confirming, "task-stop", ix, cx))
            })
            .when(opens && !task.running(), |d| {
                d.child(
                    div().pt(px(2.)).flex_shrink_0().child(
                        Icon::new(IconName::ChevronRight)
                            .size(px(12.))
                            .text_color(rgb(p.disabled)),
                    ),
                )
            })
            .into_any_element()
    }

    /// Stop, then a confirmation in place: the user sees which task.
    fn render_stop_control(
        &self,
        task: &str,
        confirming: bool,
        prefix: &'static str,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.appearance.palette();
        let small = |id: gpui::ElementId, label: &'static str, color: u32| {
            interactive_control(div().id(id), p, true)
                .px_2()
                .py(px(2.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(color))
                .hover(|s| s.bg(rgb(p.border)))
                .child(label)
        };
        let id = task.to_owned();
        if confirming {
            let stop_id = id.clone();
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    small((prefix, ix * 3 + 1).into(), "Stop task", p.error)
                        .debug_selector({
                            let id = id.clone();
                            move || format!("{prefix}-confirm-{id}")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.stop_task(&stop_id, cx)
                        })),
                )
                .child(
                    small((prefix, ix * 3 + 2).into(), "Keep", p.muted).on_click(cx.listener(
                        |this, _, _, cx| {
                            cx.stop_propagation();
                            this.tasks.confirm_stop = None;
                            cx.notify();
                        },
                    )),
                )
                .into_any_element()
        } else {
            small((prefix, ix * 3).into(), "Stop", p.muted)
                .flex_shrink_0()
                .debug_selector({
                    let id = id.clone();
                    move || format!("{prefix}-{id}")
                })
                .tooltip(|window, cx| Tooltip::new("Stop this background task").build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.tasks.confirm_stop = Some(id.clone());
                    cx.notify();
                }))
                .into_any_element()
        }
    }

    fn render_tasks_header(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let log_title = self.tasks.log.as_ref().map(|log| {
            self.task_session()
                .and_then(|s| s.tasks.iter().find(|t| t.id == log.task))
                .map(bg::Task::title)
                .unwrap_or_else(|| "Task log".into())
        });
        div()
            .h(px(44.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(rgb(p.border))
            .when(log_title.is_some(), |d| {
                d.child(
                    self.icon_button(
                        "tasks-back",
                        "All background tasks",
                        IconName::ArrowLeft,
                        true,
                    )
                    .debug_selector(|| "tasks-back".into())
                    .on_click(cx.listener(|this, _, _, cx| this.show_task_list(cx))),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl_1()
                    .truncate()
                    .text_size(px(chrome::scale::HEADING))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(p.text))
                    .child(log_title.unwrap_or_else(|| "Background tasks".into())),
            )
            .child(
                self.icon_button("tasks-close", "Close", IconName::Close, true)
                    .debug_selector(|| "tasks-close".into())
                    .on_click(cx.listener(|this, _, _, cx| this.close_tasks(cx))),
            )
    }

    fn render_task_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.appearance.palette();
        let Some(session) = self.task_session() else {
            return div().into_any_element();
        };
        let now = timing::now_ms();
        if session.tasks.is_empty() {
            let text: SharedString = if session.background_tasks > 0 {
                format!(
                    "{} background task{} running. This session reports only a count: task \
                     details and logs need a Claude session on the stream transport.",
                    session.background_tasks,
                    if session.background_tasks == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
                .into()
            } else {
                "No background tasks. Shells, subagents and workflows the agent runs in the \
                 background appear here."
                    .into()
            };
            return div()
                .id("tasks-empty")
                .debug_selector(|| "tasks-empty".into())
                .p_4()
                .text_size(px(chrome::scale::META))
                .text_color(rgb(p.muted))
                .child(text)
                .into_any_element();
        }
        let rows: Vec<AnyElement> = session
            .tasks
            .clone()
            .iter()
            .enumerate()
            .map(|(ix, task)| self.render_task_row(ix, task, now, cx))
            .collect();
        div()
            .id("tasks-list")
            .debug_selector(|| "tasks-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_1()
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }

    fn render_task_log(&self, log: &Log, cx: &mut Context<Self>) -> AnyElement {
        let p = self.appearance.palette();
        let now = timing::now_ms();
        let task = self
            .task_session()
            .and_then(|s| s.tasks.iter().find(|t| t.id == log.task))
            .cloned();
        let running = task.as_ref().is_some_and(bg::Task::running);
        let confirming = self.tasks.confirm_stop.as_deref() == Some(log.task.as_str());
        let lines = self.tasks.lines.clone();
        let count = lines.len();
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let reading = self
            .view
            .requests
            .get("task-output")
            .is_some_and(|s| s.loading);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .when_some(task.as_ref(), |d, task| {
                                d.child(
                                    div()
                                        .debug_selector(|| "task-log-meta".into())
                                        .truncate()
                                        .child(meta_line_with(
                                            task,
                                            now,
                                            log.process.is_none() || !running,
                                        )),
                                )
                            })
                            .when_some(log.process.filter(|_| running), |d, process| {
                                d.child(
                                    div()
                                        .debug_selector(|| "task-log-process".into())
                                        .truncate()
                                        .child(process_line(&process)),
                                )
                            }),
                    )
                    .when(running, |d| {
                        d.child(self.render_stop_control(
                            &log.task,
                            confirming,
                            "task-log-stop",
                            0,
                            cx,
                        ))
                    }),
            )
            .when(log.truncated, |d| {
                d.child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.disabled))
                        .child("Earlier output is not shown."),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .mx_2()
                    .mb_2()
                    .rounded(px(p.control_radius))
                    .bg(rgb(p.code_block))
                    .overflow_hidden()
                    .child(if count == 0 {
                        div()
                            .p_3()
                            .debug_selector(|| "task-log-empty".into())
                            .text_size(px(chrome::scale::META))
                            .text_color(rgb(p.muted))
                            .child(if self.tasks.failed.is_some() {
                                ""
                            } else if reading || log.next_offset.is_none() {
                                "Reading the log…"
                            } else {
                                "No output yet."
                            })
                            .into_any_element()
                    } else {
                        uniform_list(
                            "task-log-lines",
                            count,
                            cx.processor(move |_this, range: std::ops::Range<usize>, _, _| {
                                range
                                    .map(|ix| {
                                        div()
                                            .id(ix)
                                            .px_3()
                                            .whitespace_nowrap()
                                            .text_color(rgb(p.prose))
                                            .child(lines[ix].clone())
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .with_horizontal_sizing_behavior(
                            ListHorizontalSizingBehavior::Unconstrained,
                        )
                        .with_width_from_item(Some(self.tasks.widest))
                        .track_scroll(self.tasks.scroll.clone())
                        .debug_selector(|| "task-log-lines".into())
                        .size_full()
                        .py_2()
                        .font_family(mono)
                        .text_size(px(chrome::scale::META))
                        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                            // Reading back up pauses following; Follow resumes.
                            if event.delta.pixel_delta(px(16.)).y > px(0.) && this.tasks.following {
                                this.tasks.following = false;
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                    })
                    .when(count > 0 && !self.tasks.following, |d| {
                        d.child(
                            div().absolute().bottom_2().right_2().child(
                                self.quiet_button(
                                    "task-log-follow",
                                    "Follow",
                                    IconName::ArrowDown,
                                    true,
                                )
                                .debug_selector(|| "task-log-follow".into())
                                .bg(rgb(p.surface))
                                .on_click(cx.listener(|this, _, _, cx| this.follow_log(cx))),
                            ),
                        )
                    }),
            )
            .when_some(self.tasks.failed.clone(), |d, error| {
                d.child(
                    div()
                        .px_3()
                        .pb_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(chrome::notice_line(
                            error,
                            Tone::Error,
                            p,
                            "task-log-error",
                        )))
                        .child(
                            self.quiet_button("task-log-retry", "Try again", IconName::Redo, true)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.tasks.failed = None;
                                    this.poll_task_log(cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_tasks_panel(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.appearance.palette();
        let body = match &self.tasks.log {
            Some(log) => self.render_task_log(log, cx),
            None => self.render_task_list(cx),
        };
        div()
            .id("tasks-panel-card")
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(p.panel_radius))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .child(self.render_tasks_header(cx))
            .when_some(self.tasks.notice.clone(), |d, (text, tone)| {
                d.child(
                    div()
                        .debug_selector(|| "tasks-notice".into())
                        .px_3()
                        .pt_2()
                        .child(chrome::notice_line(text, tone, p, "tasks-notice")),
                )
            })
            .child(body)
    }

    /// Whether the panel takes the slot this frame (the file viewer wins it).
    fn tasks_showing(&self) -> bool {
        self.tasks.open && !self.viewer_in_slot() && self.task_session().is_some()
    }

    /// Wide windows: the panel docks beside the conversation.
    pub(super) fn render_docked_tasks(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.tasks_showing() {
            return None;
        }
        let width = self.viewer_dock_width(window)?;
        Some(
            div()
                .id("tasks-panel")
                .debug_selector(|| "tasks-panel".into())
                .relative()
                .h_full()
                .w(width)
                .flex_shrink_0()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(width)
                        .py_2()
                        .pr_2()
                        .when(chrome::custom_caption(), |d| {
                            d.pt(px(chrome::CAPTION_HEIGHT + 4.))
                        })
                        .child(self.render_tasks_panel(cx)),
                )
                .map(|slot| {
                    self.slide_in(slot, "tasks-slide", move |panel, progress| {
                        panel.w(width * progress)
                    })
                }),
        )
    }

    /// Slide a panel in over [`SLIDE`], or show it at once under reduced
    /// motion.
    fn slide_in(
        &self,
        element: Stateful<Div>,
        key: &'static str,
        animator: impl Fn(Stateful<Div>, f32) -> Stateful<Div> + 'static,
    ) -> AnyElement {
        if self.settings.reduce_motion {
            return element.into_any_element();
        }
        element
            .with_animation(
                (key, self.tasks.opened),
                Animation::new(SLIDE).with_easing(|t| 1. - (1. - t).powi(3)),
                animator,
            )
            .into_any_element()
    }

    /// Narrow windows: the panel slides over the conversation as a sheet.
    pub(super) fn render_tasks_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.tasks_showing() || self.viewer_dock_width(window).is_some() {
            return None;
        }
        let p = self.appearance.palette();
        let width = (window.viewport_size().width - px(24.))
            .min(px(640.))
            .max(px(280.));
        Some(
            deferred(
                div()
                    .id("tasks-backdrop")
                    .debug_selector(|| "tasks-backdrop".into())
                    .absolute()
                    .inset_0()
                    .when(chrome::custom_caption(), |d| {
                        d.top(px(chrome::CAPTION_HEIGHT))
                    })
                    .occlude()
                    .bg(gpui::Hsla::from(rgb(p.shadow)).opacity(0.45))
                    .flex()
                    .justify_end()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.close_tasks(cx)),
                    )
                    .child(
                        div()
                            .id("tasks-sheet")
                            .debug_selector(|| "tasks-sheet".into())
                            .occlude()
                            .relative()
                            .w(width)
                            .h_full()
                            .p_2()
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(self.render_tasks_panel(cx))
                            .map(|sheet| {
                                self.slide_in(sheet, "tasks-sheet-slide", move |sheet, progress| {
                                    sheet.left(width * (1. - progress))
                                })
                            }),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_read_plainly() {
        let task = bg::Task {
            kind: Kind::Shell,
            status: "running".into(),
            started_at: Some(1_000),
            pid: Some(4242),
            ..Default::default()
        };
        assert_eq!(
            meta_line(&task, 125_000),
            "Shell · Running · 2m 04s · pid 4242"
        );
        let done = bg::Task {
            status: "completed".into(),
            ended_at: Some(2_000),
            ..task
        };
        assert_eq!(meta_line(&done, 9_000), "Shell · Done · 1s");
        let process = bg::Process {
            pid: 7,
            alive: true,
            cpu_percent: Some(3.06),
            rss_bytes: Some(2 * 1024 * 1024),
            processes: Some(2),
        };
        assert_eq!(
            process_line(&process),
            "pid 7 · running · 3.1% CPU · 2.0 MB · 2 processes"
        );
        assert_eq!(
            process_line(&bg::Process {
                alive: false,
                ..process
            }),
            "pid 7 · exited"
        );
    }
}
