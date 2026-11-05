//! The Jobs page: the hub's scheduled jobs and their runs, for reading and
//! approving. There is deliberately no form: agents write jobs (the
//! `scheduled-jobs` skill and `propose_job`), and here the user approves a
//! proposal, pauses, resumes, runs or removes one. The model and the writes
//! live in `wks_native::jobs`.
//!
//! A Runs tab shows every job's runs together, newest first and grouped by
//! day, each opening to its full output; each job row carries a strip of its
//! last runs so a failing job stands out without opening it.
use super::primitives::{self, ButtonSize, ButtonVariant, Col, StatusBadge, StatusKind, tabular};
use super::*;
use wks_native::features::{JobAction, Request};
use wks_native::jobs::{self, Job, Run};
use wks_native::timing::now_ms;

pub(super) const JOBS_DESCRIPTION: &str =
    "Work the hub runs on a schedule, even while this window is closed.";
// r2-pages: agrees with the New job button, which asks an agent to write one.
const JOBS_FOOTER_V2: &str = "New job asks an agent to write one for you. Anything an agent proposes waits here, switched off, until you approve it.";
const JOBS_FOOTER: &str = "To add or change a job, ask any agent. What it proposes waits here, switched off, until you approve it.";
const NO_JOBS_TITLE: &str = "No jobs yet";
const NO_JOBS: &str = "Ask any agent for one, for example “every weekday at 7, run the tests and wake an agent if anything fails.” What it proposes waits here until you approve it.";
const UNAVAILABLE_TITLE: &str = "Couldn’t load jobs";
const UNAVAILABLE: &str =
    "The hub didn’t list its jobs. It may be a version without jobs, or the request failed.";
const PROPOSAL_NOTE: &str =
    "An agent proposed this. It does nothing until you approve it, so read it first.";
const CHANGE_NOTE: &str = "An agent proposed this change. The job keeps running as it is until you approve; approving keeps its history and whether it is paused.";
const RUNS_UNSUPPORTED: &str =
    "This hub can’t list every job’s runs together. Open a job to see its own.";
/// Runs a job row's strip shows, oldest to newest.
const STRIP_RUNS: usize = 10;

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
        if self.design().is_v2() {
            return self.render_jobs_v2(cx);
        }
        let p = self.palette();
        let rows = self.job_rows();
        let list = rows.clone().unwrap_or_default();
        let runs = self.job_runs();
        let read = self.view.requests.get("jobs");
        // A failed read matters on its own only while there is nothing to
        // show; over a listed page it is a line above the (older) rows.
        let read_error = read
            .filter(|s| !s.loading)
            .and_then(|s| s.error.clone())
            .filter(|e| !e.is_empty());
        let page = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.job_tabs(&list, runs.as_deref(), cx))
            .when(rows.is_some(), |d| {
                d.children(
                    read_error
                        .clone()
                        .map(|e| chrome::notice_line(e, chrome::Tone::Error, p)),
                )
            })
            .child(self.feature_message("job-action"));
        if self.extras.job_runs_tab {
            return page.child(self.render_run_history(&list, runs, cx));
        }
        let Some(rows) = rows else {
            return page.child(match read_error {
                Some(error) => self.jobs_unavailable(error, cx),
                None => div().py_2().child(chrome::notice_line(
                    "Loading jobs…",
                    chrome::Tone::Loading,
                    p,
                )),
            });
        };
        if rows.is_empty() {
            return page.child(jobs_state(
                IconName::Calendar,
                NO_JOBS_TITLE,
                NO_JOBS,
                p.muted,
                p,
            ));
        }
        // Proposals sort first; they are what the page needs from the user.
        let waiting = rows.iter().filter(|j| j.proposal()).count();
        let mut page = page;
        for (ix, job) in rows.iter().enumerate() {
            if waiting > 0 && ix == 0 {
                page = page.child(group_label("Needs your approval", p.warning));
            } else if waiting > 0 && ix == waiting {
                page = page.child(group_label("Scheduled", p.muted).pt_2());
            }
            page = page.child(self.render_job(ix, job, &rows, runs.as_deref(), cx));
        }
        page.child(
            div()
                .pt_1()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(JOBS_FOOTER),
        )
    }

    /// The hub did not answer `jobs.list`: say so plainly, with its reason
    /// and a way to ask again, instead of opening on a bare error.
    fn jobs_unavailable(&self, error: String, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let connected = self.view.connected;
        jobs_state(
            IconName::TriangleAlert,
            UNAVAILABLE_TITLE,
            UNAVAILABLE,
            p.warning,
            p,
        )
        .debug_selector(|| "jobs-unavailable".into())
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .font_family(mono_font())
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(error),
        )
        .child(
            div().pt_1().child(
                self.button("jobs-retry", "Try again", connected)
                    .debug_selector(|| "jobs-retry".into())
                    .when(connected, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.request(Request::Jobs, cx);
                            this.request(Request::JobRuns, cx);
                        }))
                    }),
            ),
        )
    }

    /// Jobs / Runs, a segmented control; Runs counts today's failures.
    fn job_tabs(&self, list: &[Job], runs: Option<&[Run]>, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let now = now_ms();
        let failed_today = runs.map_or(0, |runs| {
            runs.iter()
                .filter(|r| r.status == "error" && jobs::day_label(r.started_at, now) == "Today")
                .count()
        });
        let waiting = list.iter().filter(|j| j.proposal()).count();
        let segment =
            |id: &'static str, label: String, runs_tab: bool, alert: Option<(String, u32)>| {
                let active = self.extras.job_runs_tab == runs_tab;
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .px_3()
                    .py(px(4.))
                    .rounded(px(p.control_radius))
                    .text_size(px(chrome::scale::META))
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .gap(px(6.))
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
                    .when_some(alert, |d, (text, color)| {
                        d.child(
                            div()
                                .px(px(6.))
                                .rounded_full()
                                .bg(tint(color, 0.14))
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(color))
                                .child(text),
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
        let scheduled = list.len() - waiting;
        let jobs_label = if scheduled == 0 {
            "Jobs".to_owned()
        } else {
            format!("Jobs {scheduled}")
        };
        div().flex().child(
            div()
                .p(px(2.))
                .flex()
                .gap(px(2.))
                .rounded(px(p.control_radius + 2.))
                .bg(rgb(p.selected))
                .child(segment(
                    "jobs-tab-jobs",
                    jobs_label,
                    false,
                    (waiting > 0).then(|| (format!("{waiting} to approve"), p.warning)),
                ))
                .child(segment(
                    "jobs-tab-runs",
                    "Runs".to_owned(),
                    true,
                    (failed_today > 0).then(|| (format!("{failed_today} failed today"), p.error)),
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
        let p = self.palette();
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
            return div().py_2().child(match state {
                Some(s) if s.error.is_some() => {
                    chrome::notice_line(RUNS_UNSUPPORTED, chrome::Tone::Info, p)
                }
                _ => chrome::notice_line("Loading runs…", chrome::Tone::Loading, p),
            });
        };
        let failed_only = self.extras.job_failed_only;
        let failed = runs.iter().filter(|r| r.status == "error").count();
        let filter = |id: &'static str, label: String, value: bool| {
            let active = failed_only == value;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_2()
                .py(px(3.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .font_weight(FontWeight::MEDIUM)
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
            let mut card = self.page_card().flex().flex_col().overflow_hidden();
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

    /// One run in the history: how it went, which job, when and how long,
    /// then its first line of output; open, its whole output and where to
    /// go next.
    fn run_row(
        &self,
        ix: usize,
        divided: bool,
        run: &Run,
        list: &[Job],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
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
        let meta = [run_word(run).to_owned(), run.duration()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let header = div()
            .id(("job-run", ix))
            .debug_selector(move || format!("job-run-{ix}"))
            .px_4()
            .py(px(10.))
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .cursor_pointer()
            .hover(|s| s.bg(chrome::tonal(p, 0.04)))
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
                .flex_shrink_0()
                .text_color(rgb(p.muted)),
            )
            .child(if self.design().is_v2() {
                primitives::status_indicator(run_kind(run), &p)
            } else {
                status_dot(color, 8.)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(chrome::scale::META))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if job.is_some() { p.text } else { p.muted }))
                                    .child(
                                        job.map_or("Removed job".to_owned(), |j| j.name.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(chrome::scale::CAPTION))
                                    // Failures carry the colour; the rest
                                    // leave it to the dot.
                                    .text_color(rgb(if run.status == "error" {
                                        color
                                    } else {
                                        p.muted
                                    }))
                                    .child(meta),
                            ),
                    )
                    .when(!summary.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font())
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child(summary),
                        )
                    }),
            )
            .when_some(session, |d, id| {
                d.child(
                    self.quiet_button(
                        SharedString::from(format!("job-run-row-session-{ix}")),
                        "Open",
                        IconName::ArrowRight,
                        true,
                    )
                    .debug_selector(move || format!("job-run-row-session-{ix}"))
                    .py_1()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_alerted_session(id.clone(), window, cx)
                    })),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(jobs::clock(run.started_at, self.settings.twelve_hour_clock)),
            );
        div()
            .flex()
            .flex_col()
            .when(divided, |d| d.border_t_1().border_color(rgb(p.border)))
            .child(header)
            .when(open, |d| d.child(self.run_detail(ix, run, job, cx)))
    }

    fn run_detail(&self, ix: usize, run: &Run, job: Option<&Job>, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
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
    fn run_strip(&self, id: &str, runs: Option<&[Run]>) -> Option<gpui::AnyElement> {
        let p = self.palette();
        let mut last: Vec<&Run> = runs?
            .iter()
            .filter(|r| r.job_id == id)
            .take(STRIP_RUNS)
            .collect();
        if last.is_empty() {
            return None;
        }
        last.reverse();
        if self.design().is_v2() {
            // Taller bars for failures; a tooltip says what the strip shows.
            let t = p.tokens;
            let summary = strip_summary(&last);
            return Some(
                div()
                    .id(SharedString::from(format!("job-strip-{id}")))
                    .debug_selector({
                        let id = id.to_owned();
                        move || format!("job-strip-{id}")
                    })
                    .flex_shrink_0()
                    .h(px(t.icon.xs))
                    .flex()
                    .items_end()
                    .gap(px(t.space.xxs))
                    .tooltip(primitives::tooltip_view(summary, None, &p))
                    .children(last.into_iter().map(|run| {
                        div()
                            .w(px(t.space.xs))
                            .h(px(if run.status == "error" {
                                t.icon.xs
                            } else {
                                t.space.md
                            }))
                            .rounded(px(t.stroke.hairline))
                            .bg(rgb(run_color(run, p)))
                    }))
                    .into_any_element(),
            );
        }
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
                }))
                .into_any_element(),
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
        if self.design().is_v2() {
            return self.job_row_v2(ix, job, all, runs, cx);
        }
        let p = self.palette();
        let expanded = self.extras.job_expanded.as_deref() == Some(job.id.as_str());
        let target = job
            .change()
            .then(|| all.iter().find(|j| j.id == job.replaces))
            .flatten();
        let now = now_ms();
        let proposal = job.proposal();
        let paused = !job.enabled && !proposal;
        let mut when = schedule(&job.spec["trigger"], self.settings.twelve_hour_clock)
            .unwrap_or_else(|| job.trigger_summary());
        if job.enabled
            && let Some(at) = job.next_run_at
        {
            when = format!("{when} · next {}", jobs::until(at, now));
        }
        if job.change() {
            let name = target.map_or("a removed job", |t| t.name.as_str());
            when = format!("{when} · changes {name}, proposed by {}", job.proposed_by);
        } else if proposal {
            when = format!("{when} · proposed by {}", job.proposed_by);
        }
        let toggle = job.id.clone();
        // r2-pages: v2 reads as a sentence, the command in an inset.
        let header = {
            div()
                .id(("job-row", ix))
                .min_w_0()
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
                    .flex_shrink_0()
                    .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(220.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(chrome::scale::BODY))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(if paused { p.muted } else { p.text }))
                                        .child(job.name.clone()),
                                )
                                .children(self.job_chip(job, p))
                                .child(div().flex_1())
                                .child(self.job_status(job, now))
                                .children(
                                    (!proposal).then(|| self.run_strip(&job.id, runs)).flatten(),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(if paused { p.muted } else { p.prose }))
                                .child(when),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font())
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .debug_selector(move || format!("job-action-{ix}"))
                                .child(if job.host().is_empty() {
                                    job.action_summary()
                                } else {
                                    // Queued work says where; inline is the default
                                    // and stays unsaid (Details shows it).
                                    format!("{} · {}", job.action_summary(), job.host_label())
                                }),
                        ),
                )
                .child(self.job_actions(ix, job, target, cx))
        };
        let card = chrome::card(p)
            .debug_selector(move || format!("job-row-{ix}"))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header)
            .when(expanded, |d| d.child(self.job_detail(job, target)));
        if !proposal {
            return card;
        }
        // What waits on the user: a warning rule down its edge and a faint
        // warning wash, so it reads as "needs you" before it is read.
        card.border_color(tint(p.warning, 0.4))
            .bg(tint(p.warning, 0.05))
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.))
                    .bg(rgb(p.warning)),
            )
    }

    /// The state beside a job's name: waiting for approval, running, or
    /// switched off.
    fn job_chip(&self, job: &Job, p: Palette) -> Option<Div> {
        if self.design().is_v2() {
            let badge = if job.change() {
                StatusBadge::new(StatusKind::Idle).label("Proposed change")
            } else if job.proposal() {
                return None;
            } else if job.running {
                StatusBadge::new(StatusKind::Working)
            } else if !job.enabled {
                StatusBadge::new(StatusKind::Paused)
            } else {
                return None;
            };
            return Some(badge.render(&p));
        }
        let (text, color) = if job.change() {
            ("Proposed change", p.warning)
        } else if job.proposal() {
            // Its group, rule and wash already say it waits on the user.
            return None;
        } else if job.running {
            ("Running", p.busy)
        } else if !job.enabled {
            ("Paused", p.muted)
        } else {
            return None;
        };
        Some(chip(text, color, p))
    }

    /// Right of a job's name: how its last run went.
    fn job_status(&self, job: &Job, now: i64) -> Div {
        let p = self.palette();
        let caption = |text: String, color: u32| {
            div()
                .flex_shrink_0()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(color))
                .child(text)
        };
        if job.proposal() {
            return div();
        }
        if self.design().is_v2() {
            let badge = match &job.last_run {
                None => StatusBadge::new(StatusKind::Idle).label("Not run yet"),
                Some(run) => {
                    let at = run.finished_at.unwrap_or(run.started_at);
                    StatusBadge::new(run_kind(run)).label(format!(
                        "{} {}",
                        run_word(run),
                        jobs::since(at, now)
                    ))
                }
            };
            return badge.render(&p);
        }
        let Some(run) = &job.last_run else {
            return caption("Not run yet".into(), p.muted);
        };
        let at = run.finished_at.unwrap_or(run.started_at);
        let color = run_color(run, p);
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(status_dot(color, 6.))
            .child(caption(
                format!("{} {}", run_word(run), jobs::since(at, now)),
                if run.status == "ok" { p.muted } else { color },
            ))
    }

    fn job_actions(
        &self,
        ix: usize,
        job: &Job,
        target: Option<&Job>,
        cx: &mut Context<Self>,
    ) -> Div {
        if self.design().is_v2() {
            return self.job_actions_v2(ix, job, target, cx);
        }
        let enabled = self.view.connected && !self.demo && !self.job_busy();
        let id = job.id.clone();
        let proposal = job.proposal();
        let confirming = self.extras.job_confirm_remove.as_deref() == Some(id.as_str());
        let remove_id = id.clone();
        // Removing is quiet until it is asked for; the second click, which
        // deletes the history too, is the one that looks destructive.
        let v2 = self.design().is_v2().then(|| self.palette());
        // V2: one quiet ghost until asked, then a ghost-danger confirm.
        let v2_button = |id: SharedString, label: &'static str, variant, enabled: bool| {
            v2.map(|p| primitives::button(id, label, None, variant, ButtonSize::Small, enabled, &p))
        };
        let remove = if let Some(button) = v2_button(
            job_control("remove", ix),
            match (confirming, proposal) {
                (true, _) => "Remove job and history",
                (false, true) => "Reject",
                (false, false) => "Remove",
            },
            if confirming {
                ButtonVariant::Destructive
            } else {
                ButtonVariant::Tertiary
            },
            enabled,
        ) {
            button
        } else if confirming {
            self.danger_button(job_control("remove", ix), "Remove job and history", enabled)
        } else {
            self.job_quiet(
                job_control("remove", ix),
                if proposal { "Reject" } else { "Remove" },
                enabled,
            )
        }
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
            // Proposals sort first: the first is the page's one primary
            // action, the rest stay tonal.
            let approve = v2_button(
                job_control("approve", ix),
                "Approve",
                if ix == 0 {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
                can_approve,
            )
            .unwrap_or_else(|| {
                self.button_style(job_control("approve", ix), "Approve", can_approve, ix == 0)
            })
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
        let pause = v2_button(
            job_control("pause", ix),
            pause_label,
            ButtonVariant::Secondary,
            enabled,
        )
        .unwrap_or_else(|| self.button(job_control("pause", ix), pause_label, enabled))
        .debug_selector(move || format!("job-pause-{ix}"))
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.job_action(JobAction::Save(paused.clone()), cx);
            }))
        });
        let can_run = enabled && !job.running;
        let run = v2_button(
            job_control("run", ix),
            "Run now",
            ButtonVariant::Secondary,
            can_run,
        )
        .unwrap_or_else(|| self.button(job_control("run", ix), "Run now", can_run))
        .debug_selector(move || format!("job-run-{ix}"))
        .when(can_run, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.job_action(JobAction::Run(id.clone()), cx);
            }))
        });
        row.child(pause).child(run)
    }

    /// A text-only, low-emphasis row action.
    fn job_quiet(&self, id: SharedString, label: &'static str, enabled: bool) -> Stateful<Div> {
        let p = self.palette();
        chrome::interactive_control(div().id(id), p, enabled)
            .px_3()
            .py_2()
            .rounded(px(p.control_radius))
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(chrome::scale::META))
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover_text_style(|s| s.bg(chrome::tonal(p, 0.07)).text_color(rgb(p.text)))
            })
            .child(label)
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
        let p = self.palette();
        let note = if job.change() {
            Some(CHANGE_NOTE)
        } else if job.proposal() {
            Some(PROPOSAL_NOTE)
        } else {
            None
        };
        div()
            .debug_selector(|| "job-detail".into())
            .border_t_1()
            .border_color(rgb(p.border))
            .pr_4()
            .pt_3()
            .pb_4()
            .pl(px(44.))
            .flex()
            .flex_col()
            .gap_3()
            .children(note.map(|note| chrome::notice_line(note, chrome::Tone::Warning, p)))
            .map(|d| match target {
                Some(current) => d
                    .child(self.spec_block(Some("Now"), current, None))
                    .child(self.spec_block(Some("Proposed"), job, Some(current))),
                None => d.child(self.spec_block(None, job, None)),
            })
            .when(!job.proposal(), |d| d.child(self.job_history(&job.id)))
    }

    /// The spec field by field, its schedule in the same plain words as
    /// the row.
    fn job_details(&self, job: &Job) -> Vec<(String, String)> {
        let plain = schedule(&job.spec["trigger"], self.settings.twelve_hour_clock);
        job.details()
            .into_iter()
            .map(|(label, value)| match (&plain, label.as_str()) {
                (Some(plain), "When") => (label, plain.clone()),
                _ => (label, value),
            })
            .collect()
    }

    fn spec_block(&self, title: Option<&'static str>, job: &Job, base: Option<&Job>) -> Div {
        let p = self.palette();
        let before = base.map(|base| self.job_details(base));
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
            .children(self.job_details(job).into_iter().map(|(label, value)| {
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
        let p = self.palette();
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
                            .child(format!("{} {}", run_word(&run), jobs::since(at, now))),
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

/// A run's state in the shared status vocabulary (v2).
fn run_kind(run: &Run) -> StatusKind {
    match run.status.as_str() {
        "ok" => StatusKind::Done,
        "error" => StatusKind::Failed,
        _ => StatusKind::Idle,
    }
}

fn job_control(action: &str, ix: usize) -> SharedString {
    format!("job-{action}-{ix}").into()
}

/// How a run went, as a word a row can lead with.
fn run_word(run: &Run) -> &'static str {
    match run.status.as_str() {
        "ok" => "Succeeded",
        "skipped" => "Skipped",
        _ => "Failed",
    }
}

fn tint(color: u32, alpha: f32) -> gpui::Hsla {
    gpui::Hsla::from(rgb(color)).opacity(alpha)
}

fn status_dot(color: u32, size: f32) -> Div {
    div()
        .size(px(size))
        .rounded_full()
        .flex_shrink_0()
        .bg(rgb(color))
}

/// A small state pill: its tone's text on a faint wash of the same tone.
fn chip(text: &'static str, color: u32, p: Palette) -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(p.control_radius))
        .bg(if color == p.muted {
            chrome::tonal(p, 0.08)
        } else {
            tint(color, 0.14)
        })
        .text_size(px(chrome::scale::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(text)
}

/// The overline above a group of job rows.
pub(super) fn group_label(text: &'static str, color: u32) -> Div {
    div()
        .px_1()
        .text_size(px(chrome::scale::OVERLINE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(color))
        .child(text.to_uppercase())
}

/// A centred page state (nothing yet, or not available): an icon on a
/// tonal disc, a heading and one short paragraph.
pub(super) fn jobs_state(
    icon: IconName,
    title: &'static str,
    body: &'static str,
    tone: u32,
    p: Palette,
) -> Div {
    div()
        .py_8()
        .px_4()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(40.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if tone == p.muted {
                    chrome::tonal(p, 0.07)
                } else {
                    tint(tone, 0.14)
                })
                .child(Icon::new(icon).size(px(18.)).text_color(rgb(tone))),
        )
        .child(
            div()
                .pt_1()
                .text_size(px(chrome::scale::HEADING))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_size(px(chrome::scale::META))
                .line_height(gpui::relative(1.45))
                .text_color(rgb(p.muted))
                .child(body),
        )
}

/// A job's trigger in plain words: "Every 4 hours", "Weekdays at 7:00 AM".
/// `None` for a kind this client does not know, which then reads as the
/// hub's own summary.
fn schedule(trigger: &serde_json::Value, twelve_hour: bool) -> Option<String> {
    let time = |at: &str| -> String {
        match chrono::NaiveTime::parse_from_str(at, "%H:%M") {
            Ok(t) if twelve_hour => t.format("%-I:%M %p").to_string(),
            Ok(t) => t.format("%H:%M").to_string(),
            Err(_) => at.to_owned(),
        }
    };
    Some(match trigger["kind"].as_str()? {
        "interval" => match trigger["everyMinutes"].as_i64()? {
            1 => "Every minute".into(),
            60 => "Every hour".into(),
            1440 => "Every day".into(),
            m if m > 0 && m % 1440 == 0 => format!("Every {} days", m / 1440),
            m if m > 0 && m % 60 == 0 => format!("Every {} hours", m / 60),
            m => format!("Every {m} minutes"),
        },
        "daily" => {
            let at = time(trigger["at"].as_str()?);
            let mut days: Vec<u64> = trigger["days"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_u64().filter(|d| *d < 7))
                .collect();
            days.sort_unstable();
            days.dedup();
            const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            const PLURAL: [&str; 7] = [
                "Sundays",
                "Mondays",
                "Tuesdays",
                "Wednesdays",
                "Thursdays",
                "Fridays",
                "Saturdays",
            ];
            match days.as_slice() {
                [] | [0, 1, 2, 3, 4, 5, 6] => format!("Every day at {at}"),
                [1, 2, 3, 4, 5] => format!("Weekdays at {at}"),
                [0, 6] => format!("Weekends at {at}"),
                [day] => format!("{} at {at}", PLURAL[*day as usize]),
                days => format!(
                    "{} at {at}",
                    days.iter()
                        .map(|d| NAMES[*d as usize])
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        "once" => {
            let at = trigger["once"].as_str()?;
            match chrono::DateTime::parse_from_rfc3339(at) {
                Ok(t) => t
                    .with_timezone(&chrono::Local)
                    .format(if twelve_hour {
                        "Once, %a %b %-d at %-I:%M %p"
                    } else {
                        "Once, %a %b %-d at %H:%M"
                    })
                    .to_string(),
                Err(_) => format!("Once, {at}"),
            }
        }
        "manual" => "Only when run by hand".into(),
        _ => return None,
    })
}

// area-pages: the Jobs page under design v2.
/// What the New job button prefills in a new session's task.
pub(super) const NEW_JOB_PROMPT: &str = "Create a scheduled job that ";

impl Workspace {
    /// The page's one primary: a new session, its task prefilled to ask the
    /// agent for a job (agents write jobs; this page approves them).
    pub(super) fn new_job_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let enabled = self.view.connected && !self.demo;
        // A proposal waiting for approval owns the page's one primary.
        let waiting = self
            .job_rows()
            .is_some_and(|rows| rows.iter().any(Job::proposal));
        primitives::button(
            "jobs-new",
            "New job",
            Some(IconName::Plus),
            if waiting {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Primary
            },
            ButtonSize::Small,
            enabled,
            &p,
        )
        .debug_selector(|| "jobs-new".into())
        .tooltip(primitives::tooltip_view(
            "Ask an agent to write one; it waits here for your approval",
            None,
            &p,
        ))
        .when(enabled, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.start_new_job(window, cx)))
        })
    }

    /// Open New session with the task reading "Create a scheduled job that "
    /// and the caret after it.
    pub(super) fn start_new_job(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo || self.spawn_pending || self.view.creating {
            return;
        }
        self.show_new_session(window, cx);
        self.prompt.update(cx, |input, cx| {
            input.set_value(NEW_JOB_PROMPT, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn render_jobs_v2(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let rows = self.job_rows();
        let list = rows.clone().unwrap_or_default();
        let runs = self.job_runs();
        let read_error = self
            .view
            .requests
            .get("jobs")
            .filter(|s| !s.loading)
            .and_then(|s| s.error.clone())
            .filter(|e| !e.is_empty());
        let page = div()
            .flex()
            .flex_col()
            .gap(px(t.space.lg))
            .child(self.job_tabs_v2(&list, runs.as_deref(), cx))
            .when(rows.is_some(), |d| {
                d.children(
                    read_error
                        .clone()
                        .map(|e| chrome::notice_line(e, chrome::Tone::Error, p)),
                )
            })
            .child(self.feature_message("job-action"));
        if self.extras.job_runs_tab {
            return page.child(self.render_run_history(&list, runs, cx));
        }
        let Some(rows) = rows else {
            return page.child(match read_error {
                Some(error) => self.jobs_unavailable(error, cx),
                None => chrome::notice_line("Loading jobs…", chrome::Tone::Loading, p),
            });
        };
        if rows.is_empty() {
            return page.child(
                primitives::empty_state(
                    IconName::Calendar,
                    NO_JOBS_TITLE,
                    Some(NO_JOBS.into()),
                    &p,
                )
                .debug_selector(|| "jobs-empty".into())
                .child(self.new_job_button(cx)),
            );
        }
        // r3-pages (R3.6): one dense table of scheduled jobs, a job per row:
        // name, schedule, last runs, next run and actions. R4.3: proposals
        // wait above it as decisions, the same component as a tool approval
        // (the whole command, where it runs, Approve primary, Reject
        // secondary, Details for the full spec): they wait on you.
        let waiting = rows.iter().filter(|j| j.proposal()).count();
        let mut page = page;
        if waiting > 0 {
            page = page
                .child(primitives::section_header(
                    "Waiting for your approval",
                    Some(waiting.to_string().into()),
                    &p,
                ))
                .children(
                    rows.iter()
                        .enumerate()
                        .take(waiting)
                        .map(|(ix, job)| self.job_proposal_v2(ix, job, &rows, cx)),
                );
        }
        let mut table: Option<Div> = None;
        for (ix, job) in rows.iter().enumerate().skip(waiting) {
            if ix == waiting {
                page = page.child(
                    primitives::section_header(
                        "Scheduled",
                        Some((rows.len() - waiting).to_string().into()),
                        &p,
                    )
                    .when(ix > 0, |d| d.pt(px(t.space.md))),
                );
                table = Some(primitives::page_table(&p).child(self.job_table_head(false)));
            }
            table = table.map(|tb| tb.child(self.render_job(ix, job, &rows, runs.as_deref(), cx)));
        }
        page = page.children(table);
        page.child(
            div()
                .pt(px(t.space.xs))
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_tertiary))
                .child(JOBS_FOOTER_V2),
        )
    }

    /// A job an agent proposed, as the one decision component (R4.3): the
    /// question, what it does and when, who proposed it, the whole command
    /// it runs (wrapped, never cut beside Approve), its folder and host,
    /// Approve primary, Reject secondary, and Details for the full spec
    /// (and, for a change, the job as it is now).
    fn job_proposal_v2(&self, ix: usize, job: &Job, all: &[Job], cx: &mut Context<Self>) -> Div {
        use super::decision::{Choice, Decision, Details, Frame};
        let p = self.palette();
        let enabled = self.view.connected && !self.demo && !self.job_busy();
        let target = job
            .change()
            .then(|| all.iter().find(|j| j.id == job.replaces))
            .flatten();
        let can_approve = enabled && (!job.change() || target.is_some());
        let quoted = |name: &str| format!("\u{201c}{name}\u{201d}");
        let headline = match (job.change(), target) {
            (true, Some(current)) => format!("Change the job {}?", quoted(&current.name)),
            (true, None) => format!("Change a removed job to {}?", quoted(&job.name)),
            (false, _) => format!("Schedule the job {}?", quoted(&job.name)),
        };
        let when = schedule(&job.spec["trigger"], self.settings.twelve_hour_clock)
            .unwrap_or_else(|| job.trigger_summary());
        let (sentence, command) = job.describe();
        let by = Some(job.proposed_by.as_str())
            .filter(|b| !b.is_empty())
            .map(|b| format!("proposed by {b}"));
        let reason = [Some(sentence), Some(when), by]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let action = &job.spec["action"];
        let folder = [&action["spawn"]["cwd"], &action["shell"]["cwd"]]
            .into_iter()
            .find_map(|v| v.as_str().map(str::trim).filter(|c| !c.is_empty()))
            .map(str::to_owned);
        let host = match job.host() {
            "" | "local" => self.decision_host(),
            "any" => "whichever machine is free".to_owned(),
            peer => peer.to_owned(),
        };
        let approval = job.approval();
        let allow = Choice::new(format!("job-approve-{ix}"), "Approve")
            .tooltip("Approve: it runs on its schedule from now on")
            .enabled(can_approve)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.job_action(JobAction::Save(approval.clone()), cx);
            }));
        let remove_id = job.id.clone();
        let reject = Choice::new(format!("job-remove-{ix}"), "Reject")
            .tooltip("Reject the proposal; nothing runs")
            .enabled(enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.remove_job(&remove_id, true, cx);
            }));
        let open = self.extras.job_expanded.as_deref() == Some(job.id.as_str());
        let toggle = job.id.clone();
        let mut decision = Decision::new(
            format!("job-row-{ix}"),
            allow,
            Frame::Card { compact: false },
        );
        decision.headline = vec![(headline, false)];
        decision.headline_selector = Some(format!("job-headline-{ix}").into());
        decision.reason = Some(reason);
        decision.request = command.map(|c| format!("$ {c}"));
        decision.request_selector = format!("job-action-{ix}").into();
        decision.folder = folder;
        decision.host = Some(host);
        decision.deny = Some(reject);
        decision.details = Some(Details {
            toggle: Choice::new(format!("job-details-{ix}"), "Details")
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_job(&toggle, cx))),
            open,
            content: open.then(|| {
                let t = p.tokens;
                primitives::inset(&p)
                    .debug_selector(|| "job-detail".into())
                    .gap(px(t.space.md))
                    .child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(if job.change() {
                                CHANGE_NOTE
                            } else {
                                PROPOSAL_NOTE
                            }),
                    )
                    .map(|d| match target {
                        Some(current) => d
                            .child(self.spec_block(Some("Now"), current, None))
                            .child(self.spec_block(Some("Proposed"), job, Some(current))),
                        None => d.child(self.spec_block(None, job, None)),
                    })
                    .into_any_element()
            }),
        });
        decision.render(&p)
    }

    /// The column labels over a jobs table.
    fn job_table_head(&self, proposals: bool) -> Div {
        let p = self.palette();
        let pg = p.tokens.pages();
        primitives::page_table_head(
            &[
                ("", Col::Fixed(p.tokens.icon.sm)),
                ("Job", Col::Flex),
                ("Schedule", Col::Fixed(pg.when)),
                (
                    if proposals {
                        "Proposed by"
                    } else {
                        "Last runs"
                    },
                    Col::Fixed(pg.runs),
                ),
                (if proposals { "" } else { "Next" }, Col::Fixed(pg.age)),
                ("", Col::Keep(pg.actions)),
            ],
            &p,
        )
    }

    /// A v2 job as one table row (R3.6): its name and what it does, the
    /// schedule, the last runs (strip and the newest result), the next run,
    /// and actions; the row opens to the whole spec and its runs.
    fn job_row_v2(
        &self,
        ix: usize,
        job: &Job,
        all: &[Job],
        runs: Option<&[Run]>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let pg = t.pages();
        let now = now_ms();
        let expanded = self.extras.job_expanded.as_deref() == Some(job.id.as_str());
        let target = job
            .change()
            .then(|| all.iter().find(|j| j.id == job.replaces))
            .flatten();
        let proposal = job.proposal();
        let paused = !job.enabled && !proposal;
        let dim = |c: u32| if paused { p.roles.text_tertiary } else { c };
        let when = schedule(&job.spec["trigger"], self.settings.twelve_hour_clock)
            .unwrap_or_else(|| job.trigger_summary());
        let next = if proposal {
            String::new()
        } else if paused {
            "—".to_owned()
        } else {
            job.next_run_at
                .map_or_else(|| "—".to_owned(), |at| jobs::until(at, now))
        };
        let (sentence, command) = job.describe();
        let what = if job.change() {
            let name = target.map_or("a removed job", |t| t.name.as_str());
            format!("Changes {name} · {sentence}")
        } else {
            sentence
        };
        let toggle = job.id.clone();
        let secondary = |text: String| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(t.text.meta))
                .text_color(rgb(dim(p.roles.text_secondary)))
                .child(text)
        };
        let row = primitives::page_table_row(&p)
            .id(("job-row", ix))
            .py(px(t.space.sm))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(p.roles.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_job(&toggle, cx)))
            .child(
                primitives::page_cell(Col::Fixed(t.icon.sm)).child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(t.icon.sm))
                    .text_color(rgb(p.roles.text_tertiary)),
                ),
            )
            .child(
                primitives::page_cell(Col::Flex)
                    .flex_col()
                    .items_start()
                    .gap(px(t.space.xxs))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight(t.weight.semibold))
                                    .text_color(rgb(dim(p.text)))
                                    .child(job.name.clone()),
                            )
                            .children(self.job_chip(job, p)),
                    )
                    .child(match command {
                        Some(command) => div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .debug_selector(move || format!("job-action-{ix}"))
                            .font_family(mono_font())
                            .text_size(px(t.text.meta))
                            .text_color(rgb(dim(p.roles.text_secondary)))
                            .child(format!("$ {command}")),
                        None => secondary(what.clone()).w_full(),
                    }),
            )
            .child(
                primitives::page_cell(Col::Fixed(pg.when))
                    .child(secondary(when).text_size(px(t.text.body))),
            )
            .child(if proposal {
                primitives::page_cell(Col::Fixed(pg.runs)).child(
                    primitives::truncated(
                        SharedString::from(format!("job-proposer-{ix}")),
                        job.proposed_by.clone(),
                        &p,
                    )
                    .text_color(rgb(p.roles.text_secondary)),
                )
            } else {
                primitives::page_cell(Col::Fixed(pg.runs))
                    .flex_col()
                    .items_start()
                    .gap(px(t.space.xxs))
                    .child(self.job_status(job, now))
                    .children(self.run_strip(&job.id, runs))
            })
            .child(primitives::page_cell(Col::Fixed(pg.age)).child(tabular(
                div().text_color(rgb(p.roles.text_secondary)).child(next),
            )))
            .child(
                primitives::page_cell(Col::Keep(pg.actions))
                    .justify_end()
                    .child(self.job_actions(ix, job, target, cx)),
            );
        div()
            .debug_selector(move || format!("job-row-{ix}"))
            .w_full()
            .flex()
            .flex_col()
            .child(row)
            .when(expanded, |d| d.child(self.job_detail(job, target)))
    }

    /// Jobs / Runs as the shared segmented control, each with its count.
    fn job_tabs_v2(&self, list: &[Job], runs: Option<&[Run]>, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let now = now_ms();
        let failed_today = runs.map_or(0, |runs| {
            runs.iter()
                .filter(|r| r.status == "error" && jobs::day_label(r.started_at, now) == "Today")
                .count()
        });
        let waiting = list.iter().filter(|j| j.proposal()).count();
        let scheduled = list.len() - waiting;
        let tab = |id: &'static str, label: &'static str, runs_tab: bool, alert: Option<Div>| {
            primitives::segment(id, label, self.extras.job_runs_tab == runs_tab, &p)
                .debug_selector(move || id.into())
                .gap(px(t.space.sm))
                .children(alert)
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
        let count = |n: usize| {
            div()
                .text_color(rgb(p.roles.text_tertiary))
                .child(n.to_string())
        };
        div().flex().child(
            primitives::segmented(&p)
                .child(tab(
                    "jobs-tab-jobs",
                    "Jobs",
                    false,
                    Some(if waiting > 0 {
                        primitives::pill(
                            format!("{waiting} to approve"),
                            primitives::Intent::Neutral,
                            &p,
                        )
                    } else {
                        count(scheduled)
                    }),
                ))
                .child(tab(
                    "jobs-tab-runs",
                    "Runs",
                    true,
                    (failed_today > 0).then(|| {
                        primitives::pill(
                            format!("{failed_today} failed today"),
                            primitives::Intent::Danger,
                            &p,
                        )
                    }),
                )),
        )
    }

    /// A v2 job row's actions: Approve (the first proposal is the page's
    /// primary) and Reject for proposals; Pause/Resume and Run now for jobs,
    /// with Remove in the overflow menu behind a confirm step.
    fn job_actions_v2(
        &self,
        ix: usize,
        job: &Job,
        target: Option<&Job>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let enabled = self.view.connected && !self.demo && !self.job_busy();
        let id = job.id.clone();
        let proposal = job.proposal();
        let confirming = self.extras.job_confirm_remove.as_deref() == Some(id.as_str());
        let button = |id: SharedString, label: &'static str, variant, enabled: bool| {
            primitives::button(id, label, None, variant, ButtonSize::Small, enabled, &p)
        };
        let row = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(t.space.xs));
        if proposal {
            let remove_id = id.clone();
            let reject = button(
                job_control("remove", ix),
                "Reject",
                ButtonVariant::Tertiary,
                enabled,
            )
            .debug_selector(move || format!("job-remove-{ix}"))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.remove_job(&remove_id, true, cx);
                }))
            });
            let can_approve = enabled && (!job.change() || target.is_some());
            let approval = job.approval();
            let approve = button(
                job_control("approve", ix),
                "Approve",
                if ix == 0 {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
                can_approve,
            )
            .debug_selector(move || format!("job-approve-{ix}"))
            .when(can_approve, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.job_action(JobAction::Save(approval.clone()), cx);
                }))
            });
            // r4-pages (R4.5): fixed slots, so every row's actions align.
            let slot = |b: Stateful<Div>| {
                primitives::action_slot(
                    t.pages().action_slot,
                    Some(primitives::slot_button(b, &p).into_any_element()),
                )
            };
            return primitives::action_slots(vec![slot(reject), slot(approve)], &p);
        }
        if confirming {
            // The second step names what goes: the job and its history.
            let remove_id = id.clone();
            return row
                .child(
                    button(
                        job_control("keep", ix),
                        "Keep",
                        ButtonVariant::Tertiary,
                        true,
                    )
                    .debug_selector(move || format!("job-keep-{ix}"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.extras.job_confirm_remove = None;
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        job_control("remove", ix),
                        "Remove job and history",
                        ButtonVariant::Destructive,
                        enabled,
                    )
                    .debug_selector(move || format!("job-remove-{ix}"))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_job(&remove_id, false, cx);
                        }))
                    }),
                );
        }
        let paused = job.with_enabled(!job.enabled);
        let pause_label = if job.enabled { "Pause" } else { "Resume" };
        let pause = button(
            job_control("pause", ix),
            pause_label,
            ButtonVariant::Secondary,
            enabled,
        )
        .debug_selector(move || format!("job-pause-{ix}"))
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.job_action(JobAction::Save(paused.clone()), cx);
            }))
        });
        let can_run = enabled && !job.running;
        let run_id = id.clone();
        let run = button(
            job_control("run", ix),
            "Run now",
            ButtonVariant::Secondary,
            can_run,
        )
        .debug_selector(move || format!("job-run-{ix}"))
        .when(can_run, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.job_action(JobAction::Run(run_id.clone()), cx);
            }))
        });
        let remove_id = id.clone();
        let remove = primitives::menu_item(
            job_control("remove", ix),
            "Remove…",
            Some(IconName::Delete),
            true,
            enabled,
            &p,
        )
        .debug_selector(move || format!("job-remove-{ix}"))
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.close_page_menu(cx);
                this.remove_job(&remove_id, false, cx);
            }))
        });
        // r4-pages (R4.5): Pause and Resume share one slot width, so Run
        // now and the overflow sit at the same x on every row.
        let slot = |b: Stateful<Div>| {
            primitives::action_slot(
                t.pages().action_slot,
                Some(primitives::slot_button(b, &p).into_any_element()),
            )
        };
        primitives::action_slots(
            vec![
                slot(pause),
                slot(run),
                div().flex_shrink_0().child(self.page_overflow(
                    format!("job-{ix}"),
                    vec![remove.into_any_element()],
                    cx,
                )),
            ],
            &p,
        )
    }
}

/// The run strip's tooltip: "Last 10 runs: 8 succeeded, 2 failed".
pub(super) fn strip_summary(runs: &[&Run]) -> String {
    let ok = runs.iter().filter(|r| r.status == "ok").count();
    let failed = runs.iter().filter(|r| r.status == "error").count();
    let skipped = runs.len() - ok - failed;
    let mut parts = Vec::new();
    if ok > 0 {
        parts.push(format!("{ok} succeeded"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    let head = match runs.len() {
        1 => "Last run".to_owned(),
        n => format!("Last {n} runs"),
    };
    format!("{head}: {}", parts.join(", "))
}
