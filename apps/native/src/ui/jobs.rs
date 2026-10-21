//! The Jobs page: the hub's scheduled jobs and their runs, for reading and
//! approving. There is deliberately no form: agents write jobs (the
//! `scheduled-jobs` skill and `propose_job`), and here the user approves a
//! proposal, pauses, resumes, runs or removes one. The model and the writes
//! live in `wks_native::jobs`.
//!
//! A Runs tab shows every job's runs together, newest first and grouped by
//! day, each opening to its full output; each job row carries a strip of its
//! last runs so a failing job stands out without opening it.
use super::*;
use wks_native::features::{JobAction, Request};
use wks_native::jobs::{self, Job, Run};

pub(super) const JOBS_DESCRIPTION: &str =
    "Work the hub runs on a schedule, even while this window is closed.";
const JOBS_FOOTER: &str = "To add or change a job, ask any agent, for example “every weekday at 7, run the tests and wake an agent if anything fails.” What it proposes waits here, switched off, until you approve it.";
const NO_JOBS: &str = "No jobs yet. Ask any agent for one; it will wait here for you to approve.";
const PROPOSAL_NOTE: &str =
    "An agent proposed this. It does nothing until you approve it, so read it first.";
const CHANGE_NOTE: &str = "An agent proposed this change. The job keeps running as it is until you approve; approving keeps its history and whether it is paused.";
const RUNS_UNSUPPORTED: &str =
    "This hub can’t list every job’s runs together. Open a job to see its own.";
/// Runs a job row's strip shows, oldest to newest.
const STRIP_RUNS: usize = 10;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Workspace {
    /// The newest list the hub returned, from a read or from a write (which
    /// answers with the refreshed list).
    pub(super) fn job_rows(&self) -> Option<Vec<Job>> {
        ["jobs", "job-action"]
            .iter()
            .filter_map(|key| self.view.requests.get(*key))
            .filter(|s| !s.loading && s.error.is_none() && s.value["jobs"].is_array())
            .max_by_key(|s| s.number)
            .map(|s| jobs::parse_list(&s.value))
    }

    fn job_busy(&self) -> bool {
        self.view
            .requests
            .get("job-action")
            .is_some_and(|s| s.loading)
    }

    fn job_action(&mut self, action: JobAction, cx: &mut Context<Self>) {
        self.extras.job_confirm_remove = None;
        self.request(Request::JobAction(action), cx);
    }

    fn toggle_job(&mut self, id: &str, cx: &mut Context<Self>) {
        self.extras.job_confirm_remove = None;
        if self.extras.job_expanded.as_deref() == Some(id) {
            self.extras.job_expanded = None;
        } else {
            self.extras.job_expanded = Some(id.to_owned());
            self.request(Request::JobHistory { id: id.to_owned() }, cx);
        }
        cx.notify();
    }

    /// Every job's runs, newest first, once the hub has answered them.
    pub(super) fn job_runs(&self) -> Option<Vec<Run>> {
        self.view
            .requests
            .get("job-runs")
            .filter(|s| !s.loading && s.error.is_none())
            .map(|s| jobs::parse_runs(&s.value))
    }

    pub(super) fn render_jobs(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let rows = self.job_rows();
        let loading = self.view.requests.get("jobs").is_some_and(|s| s.loading);
        let list = rows.clone().unwrap_or_default();
        let runs = self.job_runs();
        let page = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.job_tabs(&list, runs.as_deref(), cx))
            .child(self.feature_message("jobs"))
            .child(self.feature_message("job-action"));
        if self.extras.job_runs_tab {
            return page.child(self.render_run_history(&list, runs, cx));
        }
        page.when(rows.as_ref().is_some_and(Vec::is_empty) && !loading, |d| {
            d.child(empty_jobs(p))
        })
        .children(
            list.iter()
                .enumerate()
                .map(|(ix, job)| self.render_job(ix, job, &list, runs.as_deref(), cx)),
        )
        .child(
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(JOBS_FOOTER),
        )
    }

    /// Jobs / Runs, a segmented control; Runs counts today's failures.
    fn job_tabs(&self, list: &[Job], runs: Option<&[Run]>, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let now = now_ms();
        let failed_today = runs.map_or(0, |runs| {
            runs.iter()
                .filter(|r| r.status == "error" && jobs::day_label(r.started_at, now) == "Today")
                .count()
        });
        let segment = |id: &'static str, label: String, runs_tab: bool, alert: bool| {
            let active = self.extras.job_runs_tab == runs_tab;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_3()
                .py(px(3.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .flex()
                .items_center()
                .gap_1()
                .when(active, |d| {
                    d.bg(rgb(p.surface))
                        .text_color(rgb(p.text))
                        .shadow(chrome::floating_shadow(p))
                })
                .when(!active, |d| {
                    d.text_color(rgb(p.muted))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(p.text)))
                })
                .child(label)
                .when(alert, |d| {
                    d.child(
                        div()
                            .text_color(rgb(p.error))
                            .child(format!("· {failed_today} failed today")),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.extras.job_runs_tab != runs_tab {
                        this.extras.job_runs_tab = runs_tab;
                        if runs_tab {
                            this.request(Request::JobRuns, cx);
                        }
                        cx.notify();
                    }
                }))
        };
        let jobs_label = if list.is_empty() {
            "Jobs".to_owned()
        } else {
            format!("Jobs ({})", list.iter().filter(|j| !j.proposal()).count())
        };
        div().flex().child(
            div()
                .p(px(2.))
                .flex()
                .gap(px(2.))
                .rounded(px(p.control_radius + 2.))
                .bg(rgb(p.selected))
                .child(segment("jobs-tab-jobs", jobs_label, false, false))
                .child(segment(
                    "jobs-tab-runs",
                    "Runs".to_owned(),
                    true,
                    failed_today > 0,
                )),
        )
    }

    /// Every job's runs, grouped by day; a run opens to its full output.
    fn render_run_history(
        &self,
        list: &[Job],
        runs: Option<Vec<Run>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let caption = |text: String, color: u32| {
            div()
                .py_6()
                .text_center()
                .text_size(px(chrome::scale::META))
                .text_color(rgb(color))
                .child(text)
        };
        let state = self.view.requests.get("job-runs");
        let Some(runs) = runs else {
            return match state {
                Some(s) if s.error.is_some() => caption(RUNS_UNSUPPORTED.into(), p.muted),
                _ => caption("Loading runs…".into(), p.muted),
            };
        };
        let failed_only = self.extras.job_failed_only;
        let failed = runs.iter().filter(|r| r.status == "error").count();
        let filter = |id: &'static str, label: String, value: bool| {
            let active = failed_only == value;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_2()
                .py(px(2.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .when(active, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
                .when(!active, |d| {
                    d.text_color(rgb(p.muted))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(p.text)))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.extras.job_failed_only = value;
                    cx.notify();
                }))
        };
        let shown: Vec<Run> = runs
            .into_iter()
            .filter(|r| !failed_only || r.status == "error")
            .collect();
        let now = now_ms();
        let mut page = div().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(filter("job-runs-all", "All".into(), false))
                .child(filter(
                    "job-runs-failed",
                    format!("Failed ({failed})"),
                    true,
                )),
        );
        if shown.is_empty() {
            return page.child(caption(
                if failed_only {
                    "No failed runs."
                } else {
                    "No runs yet. Runs show here as jobs fire."
                }
                .into(),
                p.muted,
            ));
        }
        // Runs are newest first, so a day's runs are adjacent: label each
        // run once and start a group where the label changes.
        let days: Vec<String> = shown
            .iter()
            .map(|run| jobs::day_label(run.started_at, now))
            .collect();
        let mut ix = 0;
        while ix < shown.len() {
            let day = days[ix].clone();
            let mut card = chrome::card(p).flex().flex_col().overflow_hidden();
            let start = ix;
            while ix < shown.len() && days[ix] == day {
                card = card.child(self.run_row(ix, ix > start, &shown[ix], list, cx));
                ix += 1;
            }
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_1()
                            .text_size(px(chrome::scale::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(p.muted))
                            .child(day),
                    )
                    .child(card),
            );
        }
        page
    }

    /// One run in the history: when, which job, how it went, how long, and
    /// its first line; open, its whole output and where to go next.
    fn run_row(
        &self,
        ix: usize,
        divided: bool,
        run: &Run,
        list: &[Job],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let key = (run.job_id.clone(), run.started_at);
        let open = self.extras.job_run_open.as_ref() == Some(&key);
        let job = list.iter().find(|j| j.id == run.job_id);
        let color = run_color(run, p);
        let toggle = key.clone();
        // A spawn run names the session it started, by its title, with a way
        // straight to it; the bare id stays in the opened detail.
        let session = run
            .spawned()
            .and_then(|id| self.view.sessions.iter().find(|s| s.id == id));
        let summary = match session {
            Some(session) => format!("→ {}", self.session_title(session)),
            None => run.summary().to_owned(),
        };
        let session = session.map(|s| s.id.clone());
        let header = div()
            .id(("job-run", ix))
            .debug_selector(move || format!("job-run-{ix}"))
            .px_4()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(p.selected)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.extras.job_run_open = if this.extras.job_run_open.as_ref() == Some(&toggle) {
                    None
                } else {
                    Some(toggle.clone())
                };
                cx.notify();
            }))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.))
                .text_color(rgb(p.muted)),
            )
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(rgb(color)),
            )
            .child(
                div()
                    .w(px(64.))
                    .flex_shrink_0()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(jobs::clock(run.started_at, self.settings.twelve_hour_clock)),
            )
            .child(
                div()
                    .w(px(180.))
                    .flex_shrink_0()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::META))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(job.is_none(), |d| d.text_color(rgb(p.muted)))
                    .child(job.map_or("Removed job".to_owned(), |j| j.name.clone())),
            )
            .child(
                div()
                    .w(px(56.))
                    .flex_shrink_0()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(color))
                    .child(run.label()),
            )
            .child(
                div()
                    .w(px(56.))
                    .flex_shrink_0()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(run.duration()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(summary),
            )
            .when_some(session, |d, id| {
                d.child(
                    self.button(
                        SharedString::from(format!("job-run-row-session-{ix}")),
                        "Open",
                        true,
                    )
                    .debug_selector(move || format!("job-run-row-session-{ix}"))
                    .flex_shrink_0()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_alerted_session(id.clone(), window, cx)
                    })),
                )
            });
        div()
            .flex()
            .flex_col()
            .when(divided, |d| d.border_t_1().border_color(rgb(p.border)))
            .child(header)
            .when(open, |d| d.child(self.run_detail(ix, run, job, cx)))
    }

    fn run_detail(&self, ix: usize, run: &Run, job: Option<&Job>, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let now = now_ms();
        let mut when = format!(
            "Started {} ({})",
            timing::timestamp_label(Some(run.started_at), now, self.settings.twelve_hour_clock),
            jobs::since(run.started_at, now)
        );
        if !run.duration().is_empty() {
            when = format!("{when} · took {}", run.duration());
        }
        let session = run
            .spawned()
            .filter(|id| self.view.sessions.iter().any(|s| s.id == *id))
            .map(str::to_owned);
        let job_id = job.map(|j| j.id.clone());
        div()
            .debug_selector(move || format!("job-run-detail-{ix}"))
            .px_4()
            .pb_3()
            .pl(px(44.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(when),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(p.control_radius))
                    .bg(rgb(p.code_block))
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(if run.status == "error" {
                        p.error
                    } else {
                        p.prose
                    }))
                    .child(if run.detail.trim().is_empty() {
                        "No output.".to_owned()
                    } else {
                        run.detail.trim_end().to_owned()
                    }),
            )
            .when(session.is_some() || job_id.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .gap_1()
                        .when_some(session, |d, id| {
                            d.child(
                                self.button(
                                    SharedString::from(format!("job-run-session-{ix}")),
                                    "Open session",
                                    true,
                                )
                                .debug_selector(move || format!("job-run-session-{ix}"))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_alerted_session(id.clone(), window, cx)
                                    },
                                )),
                            )
                        })
                        .when_some(job_id, |d, id| {
                            d.child(
                                self.button(
                                    SharedString::from(format!("job-run-job-{ix}")),
                                    "Show job",
                                    true,
                                )
                                .debug_selector(move || format!("job-run-job-{ix}"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.extras.job_runs_tab = false;
                                        this.extras.job_expanded = None;
                                        this.toggle_job(&id, cx);
                                    },
                                )),
                            )
                        }),
                )
            })
    }

    /// A job's last runs as small bars, oldest to newest.
    fn run_strip(&self, id: &str, runs: Option<&[Run]>) -> Option<Div> {
        let p = self.appearance.palette();
        let mut last: Vec<&Run> = runs?
            .iter()
            .filter(|r| r.job_id == id)
            .take(STRIP_RUNS)
            .collect();
        if last.is_empty() {
            return None;
        }
        last.reverse();
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .items_end()
                .gap(px(2.))
                .children(last.into_iter().map(|run| {
                    div()
                        .w(px(4.))
                        .h(px(if run.status == "error" { 12. } else { 8. }))
                        .rounded(px(1.))
                        .bg(rgb(run_color(run, p)))
                })),
        )
    }

    fn render_job(
        &self,
        ix: usize,
        job: &Job,
        all: &[Job],
        runs: Option<&[Run]>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let expanded = self.extras.job_expanded.as_deref() == Some(job.id.as_str());
        let target = job
            .change()
            .then(|| all.iter().find(|j| j.id == job.replaces))
            .flatten();
        let now = now_ms();
        let mut when = job.trigger_summary();
        if job.enabled
            && let Some(at) = job.next_run_at
        {
            when = format!("{when} ({})", jobs::until(at, now));
        }
        let summary = format!("{when} · {}", job.action_summary());
        let toggle = job.id.clone();
        chrome::card(p)
            .debug_selector(move || format!("job-row-{ix}"))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("job-row", ix))
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_job(&toggle, cx)))
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(chrome::scale::BODY))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(!job.enabled && !job.proposal(), |d| {
                                                d.text_color(rgb(p.muted))
                                            })
                                            .child(job.name.clone()),
                                    )
                                    .child(self.job_status(job, target, now))
                                    .children(self.run_strip(&job.id, runs)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(mono_font())
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(summary),
                            ),
                    )
                    .child(self.job_actions(ix, job, target, cx)),
            )
            .when(expanded, |d| d.child(self.job_detail(job, target)))
    }

    /// The badge beside a job's name: what it is waiting on, or how its
    /// last run went.
    fn job_status(&self, job: &Job, target: Option<&Job>, now: i64) -> Div {
        let p = self.appearance.palette();
        let (text, color) = if job.change() {
            let name = target.map_or("a removed job", |t| t.name.as_str());
            (
                format!("change to {name} · by {}", job.proposed_by),
                p.warning,
            )
        } else if job.proposal() {
            (format!("proposed by {}", job.proposed_by), p.warning)
        } else if job.running {
            ("running".to_owned(), p.busy)
        } else if let Some(run) = &job.last_run {
            let at = run.finished_at.unwrap_or(run.started_at);
            let color = match run.status.as_str() {
                "ok" => p.success,
                "error" => p.error,
                _ => p.muted,
            };
            (format!("{} {}", run.label(), jobs::since(at, now)), color)
        } else if !job.enabled {
            ("paused".to_owned(), p.muted)
        } else {
            return div();
        };
        div()
            .flex_shrink_0()
            .text_size(px(chrome::scale::CAPTION))
            .text_color(rgb(color))
            .child(text)
    }

    fn job_actions(
        &self,
        ix: usize,
        job: &Job,
        target: Option<&Job>,
        cx: &mut Context<Self>,
    ) -> Div {
        let enabled = self.view.connected && !self.demo && !self.job_busy();
        let id = job.id.clone();
        let proposal = job.proposal();
        let confirming = self.extras.job_confirm_remove.as_deref() == Some(id.as_str());
        let remove_label = match (proposal, confirming) {
            (true, _) => "Reject",
            (false, false) => "Remove",
            (false, true) => "Remove job and history",
        };
        let remove_id = id.clone();
        let remove = self
            .danger_button(job_control("remove", ix), remove_label, enabled)
            .debug_selector(move || format!("job-remove-{ix}"))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.remove_job(&remove_id, proposal, cx);
                }))
            });
        let row = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .child(remove);
        if proposal {
            // A change whose job is gone has nothing left to change.
            let can_approve = enabled && (!job.change() || target.is_some());
            let approval = job.approval();
            let approve = self
                .primary_button(job_control("approve", ix), "Approve", can_approve)
                .debug_selector(move || format!("job-approve-{ix}"))
                .when(can_approve, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.job_action(JobAction::Save(approval.clone()), cx);
                    }))
                });
            return row.child(approve);
        }
        let paused = job.with_enabled(!job.enabled);
        let pause_label = if job.enabled { "Pause" } else { "Resume" };
        let pause = self
            .button(job_control("pause", ix), pause_label, enabled)
            .debug_selector(move || format!("job-pause-{ix}"))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.job_action(JobAction::Save(paused.clone()), cx);
                }))
            });
        let can_run = enabled && !job.running;
        let run = self
            .button(job_control("run", ix), "Run now", can_run)
            .debug_selector(move || format!("job-run-{ix}"))
            .when(can_run, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.job_action(JobAction::Run(id.clone()), cx);
                }))
            });
        row.child(pause).child(run)
    }

    /// A proposal goes on one click; a job carries history, so removing it
    /// takes a second.
    fn remove_job(&mut self, id: &str, proposal: bool, cx: &mut Context<Self>) {
        if proposal || self.extras.job_confirm_remove.as_deref() == Some(id) {
            self.job_action(JobAction::Remove(id.to_owned()), cx);
        } else {
            self.extras.job_confirm_remove = Some(id.to_owned());
            cx.notify();
        }
    }

    /// The whole spec (beside the job a change would replace) and, for an
    /// approved job, its recent runs.
    fn job_detail(&self, job: &Job, target: Option<&Job>) -> Div {
        let p = self.appearance.palette();
        let note = if job.change() {
            Some(CHANGE_NOTE)
        } else if job.proposal() {
            Some(PROPOSAL_NOTE)
        } else {
            None
        };
        div()
            .debug_selector(|| "job-detail".into())
            .px_4()
            .pb_3()
            .pl(px(44.))
            .flex()
            .flex_col()
            .gap_3()
            .children(note.map(|note| {
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.warning))
                    .child(note)
            }))
            .map(|d| match target {
                Some(current) => d
                    .child(self.spec_block(Some("Now"), current, None))
                    .child(self.spec_block(Some("Proposed"), job, Some(current))),
                None => d.child(self.spec_block(None, job, None)),
            })
            .when(!job.proposal(), |d| d.child(self.job_history(&job.id)))
    }

    fn spec_block(&self, title: Option<&'static str>, job: &Job, base: Option<&Job>) -> Div {
        let p = self.appearance.palette();
        let before = base.map(Job::details);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(title.map(|title| {
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(p.muted))
                    .child(title)
            }))
            .children(job.details().into_iter().map(|(label, value)| {
                let changed = before
                    .as_ref()
                    .is_some_and(|rows| !rows.iter().any(|row| row.0 == label && row.1 == value));
                div()
                    .flex()
                    .gap_3()
                    .min_w_0()
                    .child(
                        div()
                            .w(px(72.))
                            .flex_shrink_0()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(if changed { p.warning } else { p.muted }))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(mono_font())
                            .text_size(px(chrome::scale::CAPTION))
                            .child(value),
                    )
            }))
    }

    fn job_history(&self, id: &str) -> Div {
        let p = self.appearance.palette();
        let state = self
            .view
            .requests
            .get("job-history")
            .filter(|s| matches!(&s.request, Request::JobHistory { id: asked } if asked == id));
        let caption = |text: &'static str| {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(text)
        };
        let Some(state) = state.filter(|s| !s.loading) else {
            return caption("Loading runs…");
        };
        if let Some(error) = &state.error {
            return div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.error))
                .child(error.clone());
        }
        let runs = jobs::parse_runs(&state.value);
        if runs.is_empty() {
            return caption("No runs yet.");
        }
        let now = now_ms();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(p.muted))
                    .child("Recent runs"),
            )
            .children(runs.into_iter().take(8).map(|run| {
                let color = run_color(&run, p);
                let at = run.finished_at.unwrap_or(run.started_at);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .text_size(px(chrome::scale::CAPTION))
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(rgb(color)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(p.muted))
                            .child(format!("{} {}", run.label(), jobs::since(at, now))),
                    )
                    .when(!run.duration().is_empty(), |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_color(rgb(p.muted))
                                .child(run.duration()),
                        )
                    })
                    .when(!run.detail.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font())
                                .text_color(rgb(if run.status == "error" {
                                    p.error
                                } else {
                                    p.muted
                                }))
                                .child(run.detail.clone()),
                        )
                    })
            }))
    }
}

fn run_color(run: &Run, p: Palette) -> u32 {
    match run.status.as_str() {
        "ok" => p.success,
        "error" => p.error,
        _ => p.muted,
    }
}

fn job_control(action: &str, ix: usize) -> SharedString {
    format!("job-{action}-{ix}").into()
}

fn empty_jobs(p: Palette) -> Div {
    div()
        .py_6()
        .text_center()
        .text_size(px(chrome::scale::META))
        .text_color(rgb(p.muted))
        .child(NO_JOBS)
}
