//! A session's pending question set: the picker in the dock, its answers
//! and their submission.
use super::*;
use gpui::AnyElement;
use gpui_component::scroll::ScrollableElement;
use serde_json::Value;

/// The questions in a pending set, whether sent bare or under `questions`.
pub(super) fn question_list(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .or_else(|| value.get("questions").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

impl Workspace {
    /// A new question set (or another session's) starts fresh: no picks,
    /// answers or focus carried over from the last one.
    pub(super) fn sync_questions(
        &mut self,
        next: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = next
            .sessions
            .iter()
            .find(|s| Some(&s.id) == next.selected.as_ref())
            .and_then(|s| s.questions.as_ref());
        let signature = format!(
            "{:?}:{}",
            next.selected,
            pending.map(Value::to_string).unwrap_or_default()
        );
        if signature != self.extras.question_signature {
            let questions = pending.map(question_list).unwrap_or_default();
            self.extras.question_signature = signature;
            self.extras.answer_watches.clear();
            self.extras.selected_options = vec![Default::default(); questions.len()];
            self.extras.answers_sent = false;
            self.extras.answer_error = None;
            self.extras
                .question_scroll
                .set_offset(gpui::Point::default());
            self.extras.question_focused.set(None);
            self.extras.option_focus = questions
                .iter()
                .map(|q| {
                    let count = q["options"].as_array().map_or(0, Vec::len);
                    (0..count)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect()
                })
                .collect();
            self.extras.answers = questions
                .iter()
                .map(|q| {
                    let has_options = q["options"].as_array().is_some_and(|o| !o.is_empty());
                    cx.new(|cx| {
                        InputState::new(window, cx).placeholder(if has_options {
                            "Or type a different answer"
                        } else {
                            "Type your answer"
                        })
                    })
                })
                .collect();
        }
        self.extras.answer_watches = self
            .extras
            .answers
            .iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
    }

    pub(super) fn question_answers(&self, cx: &App) -> Vec<String> {
        let qs = self
            .selected_session()
            .and_then(|s| s.questions.as_ref())
            .map(question_list)
            .unwrap_or_default();
        self.extras
            .answers
            .iter()
            .enumerate()
            .map(|(ix, input)| {
                let custom = input.read(cx).value().to_string();
                if !custom.trim().is_empty() {
                    return custom;
                }
                self.extras
                    .selected_options
                    .get(ix)
                    .into_iter()
                    .flatten()
                    .filter_map(|option_ix| {
                        let option = qs.get(ix)?.get("options")?.get(*option_ix)?;
                        option["label"]
                            .as_str()
                            .or(option.as_str())
                            .map(str::to_owned)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect()
    }

    /// Every question has a non-empty answer (a choice or typed text).
    pub(super) fn answers_ready(&self, cx: &App) -> bool {
        !self.extras.answers.is_empty()
            && self
                .question_answers(cx)
                .iter()
                .all(|s| !s.trim().is_empty())
    }

    /// The only way answers leave the picker: the Send button, Ctrl/Cmd+Enter
    /// inside the picker, or Enter in a typed answer. All of them need every
    /// question answered and nothing already sent for this question set.
    pub(super) fn submit_answers(&mut self, cx: &mut Context<Self>) {
        // The same gate as the rendered controls: keyboard sends must not
        // reach a session the user is navigating away from.
        if self.extras.answers_sent
            || self.extras.answer_submission.is_some()
            || self.view.child.is_some()
            || self.view.busy
            || !self.view.connected
            || self.view.loading
            || self
                .navigation_selected
                .as_ref()
                .is_some_and(|id| Some(id) != self.view.selected.as_ref())
            || !self
                .selected_session()
                .is_some_and(|s| s.questions.is_some() && !s.stopped())
            || !self.answers_ready(cx)
        {
            return;
        }
        let answers = self.question_answers(cx);
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        match self.controller.command(Command::Act {
            session,
            action: Action::Answers(answers.clone()),
        }) {
            Ok(()) => {
                self.extras.answer_submission =
                    Some((self.extras.question_signature.clone(), answers));
                self.extras.answer_error = None;
                self.local_notice.clear();
            }
            Err(error) => self.local_notice = error.to_string(),
        }
        cx.notify();
    }

    /// Toggle (multiple choice) or choose (single choice) one option. A choice
    /// replaces typed text for that question, as typing replaces a choice.
    fn pick_option(
        &mut self,
        ix: usize,
        option_ix: usize,
        multiple: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.extras.answers_sent
            || self.extras.answer_submission.is_some()
            || self.view.busy
            || !self.view.connected
        {
            return;
        }
        let Some(input) = self.extras.answers.get(ix).cloned() else {
            return;
        };
        if let Some(selected) = self.extras.selected_options.get_mut(ix) {
            let typed = !input.read(cx).value().is_empty();
            if multiple && !typed && selected.contains(&option_ix) {
                selected.remove(&option_ix);
            } else {
                if !multiple || typed {
                    selected.clear();
                }
                selected.insert(option_ix);
            }
        }
        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Enter in a typed answer: send when every question is answered,
    /// otherwise move to the next unanswered question's answer field. The
    /// key is consumed either way, so it never becomes text in a field;
    /// during an IME composition it is left to the input method.
    pub(super) fn answer_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .extras
            .answers
            .iter()
            .position(|input| input.read(cx).focus_handle(cx).is_focused(window))
        else {
            cx.propagate();
            return;
        };
        let composing = self.extras.answers[ix]
            .update(cx, |input, cx| {
                gpui::EntityInputHandler::marked_text_range(input, window, cx)
            })
            .is_some_and(|range| !range.is_empty());
        if composing {
            cx.propagate();
            return;
        }
        if self.answers_ready(cx) {
            self.submit_answers(cx);
            return;
        }
        let answers = self.question_answers(cx);
        let count = answers.len();
        if let Some(next) = (1..count)
            .map(|step| (ix + step) % count)
            .find(|i| answers[*i].trim().is_empty())
        {
            let handle = self.extras.answers[next].read(cx).focus_handle(cx);
            window.focus(&handle);
        }
    }

    /// Docked AskUserQuestion picker: one themed card above the composer.
    /// Questions scroll inside the card; the header and the Send row stay
    /// visible so a long set can never push the submit control out of view.
    pub(super) fn render_questions(
        &self,
        session: &Session,
        enabled: bool,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let list = session
            .questions
            .as_ref()
            .map(question_list)
            .unwrap_or_default();
        let answers = self.question_answers(cx);
        let total = answers.len();
        let answered = answers.iter().filter(|a| !a.trim().is_empty()).count();
        let ready = self.answers_ready(cx);
        let sent = self.extras.answers_sent;
        let interactive = enabled && !sent && self.extras.answer_submission.is_none();
        let failed = self.extras.answer_error.clone();
        let pad = px(if compact { 8. } else { 12. });
        let body_max = window.viewport_size().height * if compact { 0.4 } else { 0.46 };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .child(status_dot(p.warning))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(p.warning))
                    .child(if total > 1 && !compact {
                        format!("Needs your input · {total} questions")
                    } else {
                        "Needs your input".into()
                    }),
            )
            .when(total > 1, |d| {
                d.child(
                    div()
                        .debug_selector(|| "question-progress".into())
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(if answered == total {
                            p.success
                        } else {
                            p.muted
                        }))
                        .child(format!("{answered} of {total} answered")),
                )
            });
        let card = div()
            .id("question-choices")
            .debug_selector(|| "question-card".into())
            .key_context("QuestionPicker")
            .on_action(cx.listener(|this, _: &SubmitAnswers, _, cx| this.submit_answers(cx)))
            .on_action(
                cx.listener(|this, _: &AnswerEnter, window, cx| this.answer_enter(window, cx)),
            )
            .occlude()
            .w_full()
            .min_h_0()
            .flex_shrink()
            .p(pad)
            .rounded(px(p.panel_radius))
            .shadow(chrome::floating_shadow(p))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap(px(if compact { 6. } else { 10. }));
        if list.is_empty() {
            // A question the hub could not structure: answer from the composer.
            return card
                .child(header)
                .child(
                    div()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(
                            "The agent asked for input. Type your answer in the composer below.",
                        ),
                )
                .child(
                    div().flex().justify_end().child(
                        self.primary_button("answer-text", "Answer with composer", enabled)
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.act(
                                        Action::Answer(this.composer.read(cx).value().to_string()),
                                        cx,
                                    )
                                }))
                            }),
                    ),
                );
        }
        let failed = failed.filter(|_| !sent);
        let status: Option<AnyElement> = if let Some(error) = failed {
            Some(
                chrome::notice_line(error, chrome::Tone::Error, p, "question-error")
                    .into_any_element(),
            )
        } else if sent {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(chrome::notice_line(
                        "Answers sent. Waiting for the agent…",
                        chrome::Tone::Loading,
                        p,
                        "question-sent",
                    )))
                    .child(
                        self.button("edit-answers", "Edit answers", enabled)
                            .debug_selector(|| "edit-answers".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.extras.answers_sent = false;
                                    cx.notify();
                                }))
                            }),
                    )
                    .into_any_element(),
            )
        } else if self.view.busy || self.extras.answer_submission.is_some() {
            Some(
                chrome::notice_line("Sending…", chrome::Tone::Loading, p, "question-sending")
                    .into_any_element(),
            )
        } else if compact {
            // Short windows: the header's progress and Send carry this.
            None
        } else if ready {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(keycap(
                        if cfg!(target_os = "macos") {
                            "⌘ Enter"
                        } else {
                            "Ctrl Enter"
                        },
                        p,
                    ))
                    .child("to send")
                    .into_any_element(),
            )
        } else {
            Some(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(match total - answered {
                        _ if total == 1
                            && list[0]["options"].as_array().is_none_or(Vec::is_empty) =>
                        {
                            "Type an answer to send".to_owned()
                        }
                        _ if total == 1 => "Choose an option or type an answer".to_owned(),
                        1 => "Answer 1 more question to send".to_owned(),
                        left => format!("Answer {left} more questions to send"),
                    })
                    .into_any_element(),
            )
        };
        let submit = self
            .primary_button(
                "submit-answers",
                if total > 1 {
                    "Send answers"
                } else {
                    "Send answer"
                },
                interactive && ready && !self.view.busy,
            )
            .debug_selector(|| "submit-answers".into())
            .flex_shrink_0()
            .when(interactive && ready, |d| {
                d.on_click(cx.listener(|this, _, _, cx| this.submit_answers(cx)))
            });
        // Rows are direct children of the list so the focused one (Tab,
        // arrows, Enter to the next answer) can be scrolled into view.
        let mut children = Vec::new();
        let mut focused = None;
        for (ix, q) in list.iter().enumerate() {
            self.push_question(
                ix,
                q,
                total,
                interactive,
                compact,
                window,
                cx,
                &mut children,
                &mut focused,
            );
        }
        if focused != self.extras.question_focused.get() {
            self.extras.question_focused.set(focused);
            if let Some(child) = focused {
                self.extras.question_scroll.scroll_to_item(child);
            }
        }
        let body = div()
            .id("question-list")
            .debug_selector(|| "question-list".into())
            .relative()
            .min_h_0()
            .flex_shrink()
            .max_h(body_max)
            .pr_2()
            .overflow_y_scroll()
            .track_scroll(&self.extras.question_scroll)
            .flex()
            .flex_col()
            .gap(px(if compact { 3. } else { 5. }))
            .children(children)
            // After the rows: the scrollbar layer is a child too, and
            // scroll_to_item indexes children.
            .vertical_scrollbar(&self.extras.question_scroll);
        if compact {
            // One header row holds progress and Send; any status sits under it.
            return card
                .child(header.child(submit))
                .children(status.map(|status| div().flex_shrink_0().child(status)))
                .child(body);
        }
        card.child(header).child(body).child(
            div()
                .debug_selector(|| "question-footer".into())
                .flex_shrink_0()
                .pt(px(8.))
                .border_t_1()
                .border_color(rgb(p.border))
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().children(status))
                .child(submit),
        )
    }

    /// One question as list rows: overline (position, header, how to answer,
    /// answered mark), the question, its options, then the typed answer.
    #[allow(clippy::too_many_arguments)]
    fn push_question(
        &self,
        ix: usize,
        q: &Value,
        total: usize,
        interactive: bool,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
        children: &mut Vec<AnyElement>,
        focused: &mut Option<usize>,
    ) {
        let p = self.appearance.palette();
        let multiple = q["multiSelect"].as_bool().unwrap_or(false);
        let options = q["options"].as_array().cloned().unwrap_or_default();
        let header = q["header"]
            .as_str()
            .map(str::trim)
            .filter(|h| !h.is_empty());
        let text = q["question"]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .or(header)
            .unwrap_or("Your answer is needed")
            .to_owned();
        let typed = self
            .extras
            .answers
            .get(ix)
            .is_some_and(|input| !input.read(cx).value().trim().is_empty());
        let answered = typed
            || self
                .extras
                .selected_options
                .get(ix)
                .is_some_and(|s| !s.is_empty());
        // Free text needs no instruction: its field's placeholder says it.
        let how = match (options.is_empty(), multiple) {
            (true, _) => "",
            (false, true) => "Choose any",
            (false, false) => "Choose one",
        };
        let mut overline = String::new();
        if total > 1 {
            overline.push_str(&format!("{} of {total}", ix + 1));
        }
        if let Some(header) = header {
            if !overline.is_empty() {
                overline.push_str(" · ");
            }
            overline.push_str(header);
        }
        children.push(
            div()
                .debug_selector(move || format!("question-{ix}"))
                .flex()
                .items_center()
                .gap_2()
                .when(ix > 0, |d| {
                    d.mt(px(if compact { 5. } else { 7. }))
                        .pt(px(if compact { 8. } else { 12. }))
                        .border_t_1()
                        .border_color(rgb(p.border))
                })
                .text_size(px(chrome::scale::OVERLINE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.muted))
                .when(!overline.is_empty(), |d| {
                    d.child(div().min_w_0().truncate().child(overline.to_uppercase()))
                })
                .when(!how.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(px(chrome::scale::CAPTION))
                            .child(how),
                    )
                })
                .child(div().flex_1())
                .when(answered, |d| {
                    d.child(
                        div()
                            .debug_selector(move || format!("question-{ix}-answered"))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(chrome::scale::CAPTION))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(p.success))
                            .child(Icon::new(IconName::CircleCheck).size(px(12.)))
                            .child("Answered"),
                    )
                })
                .into_any_element(),
        );
        children.push(
            div()
                .debug_selector(move || format!("question-{ix}-text"))
                .mb(px(if compact { 1. } else { 3. }))
                .text_size(px(14.))
                .line_height(gpui::relative(1.4))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(text)
                .into_any_element(),
        );
        for (option_ix, option) in options.iter().enumerate() {
            let chosen = !typed
                && self
                    .extras
                    .selected_options
                    .get(ix)
                    .is_some_and(|selected| selected.contains(&option_ix));
            let handle = self
                .extras
                .option_focus
                .get(ix)
                .and_then(|row| row.get(option_ix));
            if handle.is_some_and(|h| h.is_focused(window)) {
                *focused = Some(children.len());
            }
            children.push(
                self.render_option(
                    ix,
                    option_ix,
                    option,
                    options.len(),
                    multiple,
                    chosen,
                    interactive,
                    compact,
                    handle,
                    cx,
                )
                .into_any_element(),
            );
        }
        if let Some(input) = self.extras.answers.get(ix) {
            if input.read(cx).focus_handle(cx).is_focused(window) {
                *focused = Some(children.len());
            }
            children.push(
                div()
                    .debug_selector(move || format!("question-{ix}-answer"))
                    .mt(px(if compact { 1. } else { 3. }))
                    .text_size(px(chrome::scale::BODY))
                    .child(Input::new(input).disabled(!interactive))
                    .into_any_element(),
            );
        }
    }

    /// An option row: a number (single choice) or checkbox (multiple choice)
    /// badge, the literal label and its description. Tab/arrows move between
    /// rows, Enter/Space choose, and 1–9 choose within the same question.
    #[allow(clippy::too_many_arguments)]
    fn render_option(
        &self,
        ix: usize,
        option_ix: usize,
        option: &Value,
        count: usize,
        multiple: bool,
        chosen: bool,
        interactive: bool,
        compact: bool,
        focus: Option<&FocusHandle>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let label = option["label"]
            .as_str()
            .or(option.as_str())
            .unwrap_or("")
            .to_owned();
        let description = option["description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let badge_size = if compact { 16. } else { 18. };
        let badge = div()
            .debug_selector(move || format!("question-{ix}-option-{option_ix}-badge"))
            .flex_shrink_0()
            .mt(px(1.))
            .size(px(badge_size))
            .rounded(px(if multiple { 4. } else { badge_size / 2. }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| {
                if chosen {
                    d.bg(rgb(if interactive { p.primary } else { p.disabled }))
                        .text_color(rgb(p.on_primary))
                } else {
                    d.border_1()
                        .border_color(gpui::Hsla::from(rgb(p.muted)).opacity(0.5))
                        .text_color(rgb(p.muted))
                }
            })
            .map(|d| match (multiple, chosen) {
                (true, true) => d.child(Icon::new(IconName::Check).size(px(11.))),
                (true, false) => d,
                (false, _) => d.child(format!("{}", option_ix + 1)),
            });
        chrome::interactive_control(
            div().id(("question-option", ix * 1000 + option_ix)),
            p,
            interactive,
        )
        .when_some(focus.filter(|_| interactive), |d, focus| {
            d.track_focus(focus)
        })
        .debug_selector(move || format!("question-{ix}-option-{option_ix}"))
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .px(px(if compact { 6. } else { 8. }))
        .py(px(if compact { 3. } else { 5. }))
        .rounded(px(p.control_radius))
        .bg(if chosen {
            gpui::Hsla::from(rgb(p.accent)).opacity(0.16)
        } else {
            rgb(p.base).into()
        })
        .when(interactive && !chosen, |d| {
            d.hover(|s| s.bg(rgb(p.selected)))
        })
        .child(badge)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(chrome::scale::BODY))
                        .line_height(gpui::relative(1.4))
                        .font_weight(if chosen {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(rgb(if interactive { p.text } else { p.muted }))
                        .child(label),
                )
                .when(!description.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(chrome::scale::META))
                            .line_height(gpui::relative(1.4))
                            .text_color(rgb(p.muted))
                            .child(description),
                    )
                }),
        )
        .when(interactive, |d| {
            d.on_click(cx.listener(move |this, _, window, cx| {
                this.pick_option(ix, option_ix, multiple, window, cx)
            }))
            .on_key_down(cx.listener(
                move |this, event: &gpui::KeyDownEvent, window, cx| {
                    let stroke = &event.keystroke;
                    if stroke.modifiers.modified() {
                        return;
                    }
                    match stroke.key.as_str() {
                        "down" => window.focus_next(),
                        "up" => window.focus_prev(),
                        key => match key.parse::<usize>() {
                            Ok(n) if (1..=count.min(9)).contains(&n) => {
                                this.pick_option(ix, n - 1, multiple, window, cx)
                            }
                            _ => return,
                        },
                    }
                    cx.stop_propagation();
                },
            ))
        })
    }
}
