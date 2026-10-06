//! The Jobs page: the hub's scheduled jobs and their runs, for reading and
//! approving. There is deliberately no form: agents write jobs (the
//! `scheduled-jobs` skill and `propose_job`), and here the user approves a
//! proposal, pauses, resumes, runs or removes one. The model and the writes
//! live in `wks_native::jobs`.
use super::*;
use wks_native::features::{JobAction, Request};
use wks_native::jobs::{self, Job};

pub(super) const JOBS_DESCRIPTION: &str =
    "Work the hub runs on a schedule, even while this window is closed.";
const JOBS_FOOTER: &str = "To add or change a job, ask any agent, for example “every weekday at 7, run the tests and wake an agent if anything fails.” What it proposes waits here, switched off, until you approve it.";
const NO_JOBS: &str = "No jobs yet. Ask any agent for one; it will wait here for you to approve.";
const PROPOSAL_NOTE: &str =
    "An agent proposed this. It does nothing until you approve it, so read it first.";
const CHANGE_NOTE: &str = "An agent proposed this change. The job keeps running as it is until you approve; approving keeps its history and whether it is paused.";

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

    pub(super) fn render_jobs(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let rows = self.job_rows();
        let loading = self.view.requests.get("jobs").is_some_and(|s| s.loading);
        let list = rows.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.feature_message("jobs"))
            .child(self.feature_message("job-action"))
            .when(rows.as_ref().is_some_and(Vec::is_empty) && !loading, |d| {
                d.child(empty_jobs(p))
            })
            .children(
                list.iter()
                    .enumerate()
                    .map(|(ix, job)| self.render_job(ix, job, &list, cx)),
            )
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(JOBS_FOOTER),
            )
    }

    fn render_job(&self, ix: usize, job: &Job, all: &[Job], cx: &mut Context<Self>) -> Div {
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
                                    .child(self.job_status(job, target, now)),
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
            .when(!job.proposal(), |d| d.child(self.job_runs(&job.id)))
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

    fn job_runs(&self, id: &str) -> Div {
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
                let color = match run.status.as_str() {
                    "ok" => p.success,
                    "error" => p.error,
                    _ => p.muted,
                };
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
