//! The Diagnose panel: what the hub's diagnose harness says about an error
//! on screen (see `wks_native::diagnose`). It floats over the window so the
//! error stays visible behind it; Copy and Report issue act on the answer.
use super::*;
use chrome::Tone;
use gpui::{AnyElement, ClipboardItem, FontWeight, relative};
use wks_native::diagnose::{self, Subject};
use wks_native::features::Request;

impl Workspace {
    /// Whether this connection can ask for a diagnosis at all.
    pub(super) fn can_diagnose(&self) -> bool {
        self.view.connected && !self.demo && self.view.may_call("desktop.diagnose")
    }

    /// Open the panel on `subject` and ask the hub. A repeat of the subject
    /// already in flight is not sent again.
    pub(super) fn start_diagnosis(&mut self, subject: Subject, cx: &mut Context<Self>) {
        if !self.can_diagnose() {
            return;
        }
        let in_flight = self.view.requests.get("diagnose").is_some_and(|s| {
            s.loading && matches!(&s.request, Request::Diagnose(sent) if *sent == subject)
        });
        self.extras.diagnosis = Some(subject.clone());
        if !in_flight {
            // Not `request`: that clears the page notice being diagnosed.
            self.command(Command::Request(Request::Diagnose(subject)), cx);
        }
        cx.notify();
    }

    /// An error notice as a subject, with the screen and session it is on.
    pub(super) fn notice_subject(&self, text: &str) -> Subject {
        Subject::notice(
            text,
            &format!("{:?}", self.screen),
            self.selected_session().map(|s| s.provider_id()),
            env!("CARGO_PKG_VERSION"),
        )
    }

    /// The Diagnose action for an error row.
    pub(super) fn diagnose_button(
        &self,
        id: &'static str,
        subject: Subject,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.quiet_button(id, "Diagnose", IconName::Search, self.can_diagnose())
            .debug_selector(move || id.into())
            .flex_shrink_0()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.start_diagnosis(subject.clone(), cx);
                cx.stop_propagation();
            }))
    }

    /// The Diagnose action an island notice carries when it reads as an
    /// error; `None` for every other tone or when the hub cannot diagnose.
    pub(super) fn diagnose_notice(
        &self,
        id: &'static str,
        text: &str,
        tone: Tone,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        (tone == Tone::Error && self.can_diagnose()).then(|| {
            self.diagnose_button(id, self.notice_subject(text), cx)
                .py_1()
                .into_any_element()
        })
    }

    fn close_diagnosis(&mut self, cx: &mut Context<Self>) {
        self.extras.diagnosis = None;
        cx.notify();
    }

    pub(super) fn render_diagnosis(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let subject = self.extras.diagnosis.clone()?;
        let p = self.palette();
        let state = self
            .view
            .requests
            .get("diagnose")
            .filter(|s| matches!(&s.request, Request::Diagnose(sent) if *sent == subject));
        let answer: Option<Result<diagnose::Diagnosis, String>> = match state {
            None => None,
            Some(s) if s.loading => None,
            Some(s) => Some(match &s.error {
                Some(error) => Err(format!("The diagnosis failed: {error}")),
                None => diagnose::read_reply(&s.value),
            }),
        };
        let body: AnyElement = match &answer {
            None => chrome::notice_line(
                "Asking the diagnose model… (this can take up to a minute)",
                Tone::Loading,
                p,
            )
            .debug_selector(|| "diagnosis-loading".into())
            .into_any_element(),
            Some(Err(reason)) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .debug_selector(|| "diagnosis-error".into())
                        .child(chrome::notice_line(reason.clone(), Tone::Error, p)),
                )
                .child({
                    let subject = subject.clone();
                    self.quiet_button("diagnosis-retry", "Try again", IconName::Redo, true)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(Command::Request(Request::Diagnose(subject.clone())), cx)
                        }))
                })
                .into_any_element(),
            Some(Ok(diagnosis)) => self
                .render_markdown("diagnosis", &diagnosis.text, window, cx)
                .into_any_element(),
        };
        let found = answer.and_then(Result::ok);
        let footer = found.map(|diagnosis| {
            let report = diagnose::report(&subject, &diagnosis);
            let link = diagnose::issue_url(&subject, &diagnosis);
            div()
                .pt_2()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(if diagnosis.workspacer_bug {
                            format!(
                                "Answered by {}. This looks like a Workspacer bug: you can file it.",
                                diagnosis.by
                            )
                        } else {
                            format!("Answered by {}.", diagnosis.by)
                        }),
                )
                .child(
                    self.quiet_button("diagnosis-copy", "Copy", IconName::Copy, true)
                        .debug_selector(|| "diagnosis-copy".into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(report.clone()))
                        })),
                )
                .when(diagnosis.workspacer_bug, |d| {
                    d.child(
                        self.primary_button("diagnosis-report", "Report issue…", true)
                            .debug_selector(|| "diagnosis-report".into())
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&link))),
                    )
                })
        });
        let card = div()
            .id("diagnosis-card")
            .debug_selector(|| "diagnosis-card".into())
            .occlude()
            .w_full()
            .max_w(px(640.))
            .max_h(relative(0.8))
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded(px(p.panel_radius))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .shadow_lg()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(scale::BODY))
                                    .child("Diagnosis"),
                            )
                            .child(
                                div()
                                    .text_size(px(scale::META))
                                    .text_color(rgb(p.muted))
                                    .truncate()
                                    .child(subject.title.clone()),
                            ),
                    )
                    .child(
                        self.icon_button("diagnosis-close", "Close", IconName::Close, true)
                            .debug_selector(|| "diagnosis-close".into())
                            .on_click(cx.listener(|this, _, _, cx| this.close_diagnosis(cx))),
                    ),
            )
            .child(
                div()
                    .id("diagnosis-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .children(footer);
        Some(
            gpui::deferred(
                div()
                    .id("diagnosis-scrim")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .bg(chrome::tonal(p, alpha::FAINT))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.close_diagnosis(cx)),
                    )
                    .child(card),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
