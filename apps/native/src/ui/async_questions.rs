//! Codex's non-blocking questions as cards in the transcript, and the user's
//! replies to them read back as answers rather than raw envelopes.
//!
//! The agent keeps working while a card waits, so nothing here blocks the
//! composer. Answering sends one ordinary `agents.sendMessage` (steering a
//! running turn, else starting one) whose text is Codex's own reply envelope
//! (`wks_native::questions`). A card counts as answered once this window sent
//! its reply, or once the transcript holds a reply naming its item; after a
//! reload with that reply outside the window it shows as unanswered again.
use super::*;
use wks_native::questions::{self as aq, AsyncQuestion, Reply};

/// `(question, answer)` pairs, in question order.
type Answers = Vec<(String, String)>;

#[derive(Default)]
pub(super) struct AsyncQuestionUi {
    /// Cards answered from this window, by card key, with what was sent.
    answered: HashMap<String, Answers>,
    /// The reply in flight: card key, message text and its answers.
    submitting: Option<(String, String, Answers)>,
    errors: HashMap<String, String>,
    /// The option chosen per (card key, question).
    picks: HashMap<(String, usize), usize>,
    /// Typed answers per (card key, question), made as cards render.
    inputs: HashMap<(String, usize), Entity<InputState>>,
    subscriptions: Vec<gpui::Subscription>,
    /// The session the inputs above belong to.
    session: Option<String>,
}

/// Inputs kept before the oldest are dropped (cards scrolled far away).
const MAX_INPUTS: usize = 64;

fn card_key(session: &str, item_id: &str) -> String {
    format!("{session}\0{item_id}")
}

impl Workspace {
    /// A finished reply marks its card answered (or shows why it failed); a
    /// new selection starts with fresh inputs.
    pub(super) fn sync_async_questions(&mut self, next: &View) {
        let ui = &mut self.async_questions;
        if ui.session != next.selected {
            ui.session = next.selected.clone();
            ui.inputs.clear();
            ui.picks.clear();
            ui.subscriptions.clear();
        }
        let Some(receipt) = next
            .receipt
            .as_ref()
            .filter(|r| r.number > self.last_receipt)
        else {
            return;
        };
        let Some((key, text, answers)) = ui.submitting.clone() else {
            return;
        };
        let ours = matches!(&receipt.action, Action::Send(sent) if *sent == text)
            && key.starts_with(&format!("{}\0", receipt.session));
        if ours {
            ui.submitting = None;
            match &receipt.error {
                None => {
                    if ui.answered.len() > 256 {
                        ui.answered.clear();
                    }
                    ui.answered.insert(key, answers);
                }
                Some(error) => {
                    ui.errors.insert(key, error.clone());
                }
            }
        } else if !next.busy {
            // Another control won admission first; allow an explicit retry.
            ui.submitting = None;
        }
    }

    /// What answered this card: this window's reply, or one in the transcript
    /// after it naming the same item.
    fn async_answers(
        &self,
        key: &str,
        item_id: &str,
        after: usize,
    ) -> Option<Vec<(String, String)>> {
        if let Some(sent) = self.async_questions.answered.get(key) {
            return Some(sent.clone());
        }
        let found = self
            .view
            .transcript
            .rows
            .iter()
            .skip(after + 1)
            .filter(|r| r.role == "You")
            .filter_map(|r| aq::parse_reply(&r.text))
            .flatten()
            .filter(|reply| reply.question_item_id == item_id)
            .map(|reply| (reply.question, reply.answer))
            .collect::<Vec<_>>();
        (!found.is_empty()).then_some(found)
    }

    fn async_input(
        &mut self,
        key: &str,
        question: usize,
        has_options: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let slot = (key.to_owned(), question);
        if let Some(input) = self.async_questions.inputs.get(&slot) {
            return input.clone();
        }
        if self.async_questions.inputs.len() >= MAX_INPUTS {
            self.async_questions.inputs.clear();
            self.async_questions.subscriptions.clear();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if has_options {
                "Or type a different answer"
            } else {
                "Type your answer"
            })
        });
        let owner = key.to_owned();
        self.async_questions.subscriptions.push(cx.subscribe_in(
            &input,
            window,
            move |this, _, event: &gpui_component::input::InputEvent, _, cx| match event {
                gpui_component::input::InputEvent::Change => cx.notify(),
                gpui_component::input::InputEvent::PressEnter { secondary: false } => {
                    this.submit_async_answers(&owner, cx)
                }
                _ => {}
            },
        ));
        self.async_questions.inputs.insert(slot, input.clone());
        input
    }

    /// A card's typed-answer field, once rendered.
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn async_questions_input(
        &self,
        key: &str,
        question: usize,
    ) -> Option<Entity<InputState>> {
        self.async_questions
            .inputs
            .get(&(key.to_owned(), question))
            .cloned()
    }

    /// This window sent the card's reply and the hub accepted it.
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn async_answered(&self, key: &str) -> bool {
        self.async_questions.answered.contains_key(key)
    }

    /// `(question, answer)` for each question the user answered: typed text
    /// wins over a chosen option.
    fn async_draft(
        &self,
        key: &str,
        questions: &[AsyncQuestion],
        cx: &App,
    ) -> Vec<(String, String)> {
        questions
            .iter()
            .enumerate()
            .filter_map(|(ix, q)| {
                let slot = (key.to_owned(), ix);
                let typed = self
                    .async_questions
                    .inputs
                    .get(&slot)
                    .map(|input| input.read(cx).value().trim().to_owned())
                    .filter(|t| !t.is_empty());
                let answer = typed.or_else(|| {
                    let pick = *self.async_questions.picks.get(&slot)?;
                    q.options.get(pick).cloned()
                })?;
                Some((q.title.clone(), answer))
            })
            .collect()
    }

    /// The card's question row, by its key, from the selected transcript.
    fn async_row(&self, key: &str) -> Option<Arc<wks_native::model::Row>> {
        let session = self.view.selected.as_deref()?;
        self.view
            .transcript
            .rows
            .iter()
            .rev()
            .find(|r| r.role == "Questions" && card_key(session, &r.item_id) == key)
            .cloned()
    }

    fn async_enabled(&self) -> bool {
        self.view.connected
            && !self.view.busy
            && self.view.child.is_none()
            && self.async_questions.submitting.is_none()
            && self.selected_session().is_some_and(|s| !s.stopped())
    }

    pub(super) fn submit_async_answers(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.async_row(key) else {
            return;
        };
        if !self.async_enabled() || row.item_id.is_empty() {
            return;
        }
        let answers = self.async_draft(key, &row.questions, cx);
        if answers.is_empty() {
            return;
        }
        let text = aq::reply_text(&row.item_id, &answers);
        self.async_questions.errors.remove(key);
        self.async_questions.submitting = Some((key.to_owned(), text.clone(), answers));
        self.act(Action::Send(text), cx);
    }

    /// A `Questions` row: the questions with option buttons and a typed
    /// answer each, or what was answered. Outside the live chat (History, a
    /// child's transcript) it is read-only.
    pub(super) fn render_async_questions(
        &mut self,
        row: &wks_native::model::Row,
        ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let session = self.view.selected.clone().unwrap_or_default();
        let key = card_key(&session, &row.item_id);
        let live = ix.is_some() && !row.item_id.is_empty() && self.view.child.is_none();
        let answered = ix.and_then(|ix| self.async_answers(&key, &row.item_id, ix));
        let enabled = live && answered.is_none() && self.async_enabled();
        let sending = self
            .async_questions
            .submitting
            .as_ref()
            .is_some_and(|(k, ..)| *k == key);
        let mut card = div()
            .debug_selector(|| "async-question-card".into())
            .w_full()
            .p_3()
            .rounded(px(p.panel_radius))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_dot(if answered.is_some() {
                        p.success
                    } else {
                        p.accent
                    }))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if answered.is_some() {
                                p.success
                            } else {
                                p.text
                            }))
                            .child(if answered.is_some() {
                                "Answered"
                            } else if row.questions.len() > 1 {
                                "Questions for you"
                            } else {
                                "A question for you"
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .when(answered.is_none(), |d| {
                                d.child("The agent keeps working; answer whenever you like.")
                            }),
                    ),
            );
        if let Some(answers) = &answered {
            return card.child(render_answer_list(
                answers.iter().map(|(q, a)| (q.as_str(), a.as_str())),
                p,
            ));
        }
        for (qix, question) in row.questions.iter().enumerate() {
            let slot = (key.clone(), qix);
            let typed = self
                .async_questions
                .inputs
                .get(&slot)
                .is_some_and(|input| !input.read(cx).value().trim().is_empty());
            let picked = (!typed)
                .then(|| self.async_questions.picks.get(&slot).copied())
                .flatten();
            let input =
                live.then(|| self.async_input(&key, qix, !question.options.is_empty(), window, cx));
            let options = question.options.iter().enumerate().map(|(oix, option)| {
                let chosen = picked == Some(oix);
                let key = key.clone();
                chrome::interactive_control(
                    div().id(SharedString::from(format!(
                        "async-option-{key}-{qix}-{oix}"
                    ))),
                    p,
                    enabled,
                )
                .debug_selector(move || format!("async-option-{qix}-{oix}"))
                .px_2()
                .py_1()
                .rounded(px(p.control_radius))
                .text_size(px(12.))
                .text_color(rgb(if enabled { p.text } else { p.muted }))
                .bg(if chosen {
                    gpui::Hsla::from(rgb(p.accent)).opacity(0.16)
                } else {
                    rgb(p.base).into()
                })
                .when(chosen, |d| d.font_weight(FontWeight::SEMIBOLD))
                .when(enabled && !chosen, |d| d.hover(|s| s.bg(rgb(p.selected))))
                .child(option.clone())
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        let slot = (key.clone(), qix);
                        this.async_questions.picks.insert(slot.clone(), oix);
                        if let Some(input) = this.async_questions.inputs.get(&slot).cloned() {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        cx.notify();
                    }))
                })
            });
            let options = options.collect::<Vec<_>>();
            card = card
                .child(
                    div()
                        .debug_selector(move || format!("async-question-{qix}"))
                        .mt_1()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(p.text))
                        .child(question.title.clone()),
                )
                .when(!question.options.is_empty(), |d| {
                    d.child(div().flex().flex_wrap().gap_1().children(options))
                })
                .when_some(input, |d, input| {
                    d.child(
                        div()
                            .debug_selector(move || format!("async-answer-{qix}"))
                            .child(Input::new(&input).disabled(!enabled)),
                    )
                });
        }
        if !live {
            return card.child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child("Answer from the conversation's composer."),
            );
        }
        let ready = !self.async_draft(&key, &row.questions, cx).is_empty();
        let error = self.async_questions.errors.get(&key).cloned();
        let submit_key = key.clone();
        card.children(error.map(|error| {
            chrome::notice_line(error, chrome::Tone::Error, p, "async-question-error")
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .when(sending, |d| {
                    d.child(chrome::notice_line(
                        "Sending…",
                        chrome::Tone::Loading,
                        p,
                        "async-question-sending",
                    ))
                })
                .child(
                    self.primary_button(
                        SharedString::from(format!("async-submit-{key}")),
                        if row.questions.len() > 1 {
                            "Send answers"
                        } else {
                            "Send answer"
                        },
                        enabled && ready,
                    )
                    .debug_selector(|| "async-submit".into())
                    .when(enabled && ready, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.submit_async_answers(&submit_key, cx)
                        }))
                    }),
                ),
        )
    }
}

/// A reply to async questions, read as the answers it gives.
pub(super) fn render_reply(replies: &[Reply], p: Palette) -> Div {
    div()
        .debug_selector(|| "async-reply".into())
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(p.muted))
                .child(if replies.len() > 1 {
                    "Answered the agent's questions"
                } else {
                    "Answered the agent's question"
                }),
        )
        .child(render_answer_list(
            replies
                .iter()
                .map(|r| (r.question.as_str(), r.answer.as_str())),
            p,
        ))
}

fn render_answer_list<'a>(answers: impl Iterator<Item = (&'a str, &'a str)>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(answers.map(|(question, answer)| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(question.to_owned()),
                )
                .child(div().text_color(rgb(p.text)).child(answer.to_owned()))
        }))
}
