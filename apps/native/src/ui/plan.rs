//! The agent's task list: pinned under the title while the conversation is
//! open, and quiet one-line markers in the transcript where it changed.
//!
//! The pinned card shows the newest plan (TodoWrite, Claude's Task tools or
//! Codex's plan tool; see `wks_native::plan`) with progress and the running
//! step, and folds to one line. A finished list starts folded; the reader's
//! own choice holds until the plan's shape (its steps) changes. Transcript
//! markers keep the history of how the list moved, expanding to the list as
//! it was then.
use super::*;
use std::rc::Rc;
use wks_native::plan::{self as task_plan, Status, Step};

/// What the pinned plan was worked out from: session, transcript (by its stamp)
/// and the row's own plan.
type PlanSource = (String, u64, Option<wks_native::plan::Plan>);
/// A worked-out pinned plan and what it came from.
type PinnedPlan = (PlanSource, Option<Rc<Vec<Step>>>);

#[derive(Default)]
pub(super) struct PlanUi {
    /// Per session: the plan's signature and whether the reader opened it.
    open: HashMap<String, (String, bool)>,
    /// The pinned plan, worked out again only when its sources change
    /// rather than by scanning the transcript every frame.
    pinned: std::cell::RefCell<Option<PinnedPlan>>,
}

/// What the reader's open/closed choice belongs to: the steps' text, not
/// their status, so ticking a step off keeps the card as the reader left it.
fn signature(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One step: status icon and label, finished steps dimmed. Only the live
/// list animates its running step; history marks it with a dot.
pub(super) fn step_row(step: &Step, p: Palette, id: SharedString, live: bool) -> Div {
    let icon = match step.status {
        Status::Completed => Icon::new(IconName::CircleCheck)
            .size(px(13.))
            .text_color(rgb(p.success))
            .into_any_element(),
        Status::InProgress if live => brand_spinner(11., p, id).into_any_element(),
        Status::InProgress => status_dot(p.accent).into_any_element(),
        Status::Pending => div()
            .size(px(11.))
            .rounded_full()
            .border_1()
            .border_color(gpui::Hsla::from(rgb(p.muted)).opacity(0.6))
            .into_any_element(),
    };
    div()
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(12.))
        .line_height(gpui::relative(1.4))
        .child(
            div()
                .flex_shrink_0()
                .w(px(13.))
                .h(px(12. * 1.4))
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(match step.status {
                    Status::Completed => p.muted,
                    Status::InProgress => p.text,
                    Status::Pending => p.prose,
                }))
                .when(step.status == Status::InProgress, |d| {
                    d.font_weight(FontWeight::MEDIUM)
                })
                .child(step.label().to_owned()),
        )
}

impl Workspace {
    /// The plan pinned for the selected conversation, if it has one.
    pub(super) fn pinned_plan(&self) -> Option<Rc<Vec<Step>>> {
        if self.view.child.is_some() {
            return None;
        }
        let session = self.selected_session()?;
        let source = (
            session.id.clone(),
            super::transcript::transcript_stamp(&self.view.transcript),
            session.plan.clone(),
        );
        if let Some((seen, steps)) = &*self.plan.pinned.borrow()
            && *seen == source
        {
            return steps.clone();
        }
        let steps = task_plan::current(&self.view.transcript, session.plan.as_ref()).map(Rc::new);
        *self.plan.pinned.borrow_mut() = Some((source, steps.clone()));
        steps
    }

    pub(super) fn plan_open(&self, session: &str, steps: &[Step], compact: bool) -> bool {
        let (done, total) = task_plan::progress(steps);
        match self.plan.open.get(session) {
            Some((seen, open)) if *seen == signature(steps) => *open,
            // A finished list opens nothing new; short windows keep the room.
            _ => done < total && !compact,
        }
    }

    fn toggle_plan(&mut self, compact: bool, cx: &mut Context<Self>) {
        let (Some(session), Some(steps)) = (self.view.selected.clone(), self.pinned_plan()) else {
            return;
        };
        let open = !self.plan_open(&session, &steps, compact);
        if self.plan.open.len() > 256 {
            self.plan.open.clear();
        }
        self.plan.open.insert(session, (signature(&steps), open));
        cx.notify();
    }

    /// The pinned task card under the title: a one-line summary that opens
    /// to the list.
    pub(super) fn render_pinned_plan(
        &self,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let steps = self.pinned_plan()?;
        let session = self.view.selected.clone()?;
        let p = self.appearance.palette();
        let (done, total) = task_plan::progress(&steps);
        let open = self.plan_open(&session, &steps, compact);
        let active = task_plan::active(&steps);
        let summary = if done == total {
            "All tasks done".to_owned()
        } else {
            active
                .map(|step| step.label().to_owned())
                .unwrap_or_else(|| format!("{} to do", total - done))
        };
        let header = chrome::interactive_control(div().id("pinned-plan-toggle"), p, true)
            .debug_selector(|| "pinned-plan-toggle".into())
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py(px(3.))
            .rounded(px(p.control_radius))
            .text_size(px(12.))
            .hover(|s| s.bg(rgb(p.selected)))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.))
                .text_color(rgb(p.muted)),
            )
            .child(match (active, done == total) {
                (Some(_), _) => brand_spinner(11., p, "pinned-plan-active").into_any_element(),
                (None, true) => Icon::new(IconName::CircleCheck)
                    .size(px(13.))
                    .text_color(rgb(p.success))
                    .into_any_element(),
                (None, false) => status_dot(p.muted).into_any_element(),
            })
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(p.text))
                    .child("Tasks"),
            )
            .child(
                div()
                    .debug_selector(|| "pinned-plan-progress".into())
                    .flex_shrink_0()
                    .text_color(rgb(if done == total { p.success } else { p.muted }))
                    .child(format!("{done}/{total} done")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(p.muted))
                    .when(!open, |d| d.child(summary)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_plan(compact, cx)));
        let list_max = (window.viewport_size().height * 0.3).min(px(220.));
        Some(
            div()
                .debug_selector(|| "pinned-plan".into())
                .occlude()
                .w_full()
                .max_w(px(CHAT_WIDTH * 0.8))
                .p_1()
                .rounded(px(p.panel_radius))
                .shadow(chrome::floating_shadow(p))
                .bg(rgb(p.surface))
                .flex()
                .flex_col()
                .child(header)
                .when(open, |d| {
                    d.child(
                        div()
                            .id("pinned-plan-steps")
                            .debug_selector(|| "pinned-plan-steps".into())
                            .max_h(list_max)
                            .overflow_y_scroll()
                            .px_2()
                            .pt_1()
                            .pb_2()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .children(steps.iter().enumerate().map(|(ix, step)| {
                                step_row(
                                    step,
                                    p,
                                    SharedString::from(format!("pinned-plan-step-{ix}")),
                                    true,
                                )
                            })),
                    )
                }),
        )
    }

    /// A `Plan` row in the transcript: where the list changed, folded to a
    /// line until opened.
    pub(super) fn render_plan_marker(
        &self,
        key: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let steps = task_plan::parse(&serde_json::from_str::<serde_json::Value>(text).ok()?);
        if steps.is_empty() {
            return None;
        }
        let p = self.appearance.palette();
        let (done, total) = task_plan::progress(&steps);
        let open_key = format!("{key}-plan");
        let open = self.chat.open.get(&open_key).copied().unwrap_or(false);
        let toggle_key = open_key.clone();
        Some(
            div()
                .debug_selector(|| "plan-marker".into())
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    chrome::interactive_control(
                        div().id(SharedString::from(format!("{open_key}-toggle"))),
                        p,
                        true,
                    )
                    .debug_selector(|| "plan-marker-toggle".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .rounded(px(p.control_radius))
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .hover(|s| s.text_color(rgb(p.text)))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(11.)),
                    )
                    .child(format!("Task list updated · {done}/{total} done"))
                    .when_some(task_plan::active(&steps).filter(|_| !open), |d, step| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(p.disabled))
                                .child(step.label().to_owned()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle_key.clone(), false, cx)
                    })),
                )
                .when(open, |d| {
                    d.child(div().pl(px(18.)).flex().flex_col().gap(px(3.)).children(
                        steps.iter().enumerate().map(|(ix, step)| {
                            step_row(
                                step,
                                p,
                                SharedString::from(format!("{open_key}-{ix}")),
                                false,
                            )
                        }),
                    ))
                }),
        )
    }
}
