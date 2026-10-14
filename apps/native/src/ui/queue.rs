//! The selected session's message queue, docked above the composer: what the
//! hub will send when the turn ends, with edit, reorder, remove and send now,
//! the paused state and its reason, and a send that may have gone out.
//!
//! The hub owns the queue (see `wks_native::queue`); every control here is
//! one `sessions.queue*` call, and the panel redraws from the next snapshot
//! row. Nothing is ever resent on the user's behalf: a may-have-been-sent
//! message offers Send again (or, when the hub's preview may be cut, copies
//! it to the composer) and Dismiss.
//!
//! An edit belongs to its session and survives switching away. It saves
//! against the text it started from, so a change made elsewhere meanwhile is
//! a conflict the reader resolves (Overwrite / Discard mine), never a silent
//! overwrite. The editor closes only once the hub accepts the save; a refused
//! save, or an entry that left the queue while being edited, keeps the typed
//! text with Queue as new / Copy to composer.
use super::*;
use wks_native::queue::{Entry, Op, Queue};

/// Characters of an entry shown before it needs expanding.
const PREVIEW_CHARS: usize = 140;
/// Transcript rows looked through for an edited entry that was sent.
const SENT_LOOKBACK: usize = 50;

/// Why an edit could not simply be saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    /// Changed elsewhere since the edit began; the newer text.
    Conflict(String),
    /// The entry left the queue (sent, or removed elsewhere).
    Gone,
    /// Refused for another reason.
    Refused(String),
}

/// One session's edit of one queued entry.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub(super) id: String,
    /// The entry's text when the edit began, or the newer text the reader
    /// chose to overwrite: what the hub must still hold for the save to apply.
    pub(super) start: String,
    /// The editor's text, kept while another session is shown.
    text: String,
    /// The change in flight for this edit.
    pub(super) pending: Option<Op>,
    pub(super) problem: Option<Problem>,
}

pub(super) struct QueueUi {
    /// The inline editor; it shows the selected session's edit.
    pub(super) editor: Entity<InputState>,
    /// Edits by session.
    pub(super) edits: HashMap<String, Edit>,
    /// The session whose edit the editor holds now.
    shown: Option<String>,
    /// Entries shown in full.
    expanded: std::collections::BTreeSet<String>,
}

impl QueueUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            editor: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(1, 6)
                    .placeholder("Queued message")
            }),
            edits: HashMap::new(),
            shown: None,
            expanded: Default::default(),
        }
    }
}

/// Long or multi-line text folds behind Show all.
fn long(text: &str) -> bool {
    text.lines().nth(1).is_some() || text.chars().count() > PREVIEW_CHARS
}

impl Workspace {
    /// Keep each session's edit across switches, settle the edit's change
    /// when its receipt arrives, and notice an entry that left the queue.
    pub(super) fn sync_queue(&mut self, next: &View, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue.shown != next.selected {
            let typed = self.queue.editor.read(cx).value().to_string();
            if let Some(edit) = self
                .queue
                .shown
                .as_ref()
                .and_then(|s| self.queue.edits.get_mut(s))
            {
                edit.text = typed;
            }
            self.queue.shown = next.selected.clone();
            if let Some(text) = next
                .selected
                .as_ref()
                .and_then(|s| self.queue.edits.get(s))
                .map(|edit| edit.text.clone())
            {
                self.queue
                    .editor
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        let receipt = next
            .receipt
            .as_ref()
            .filter(|r| r.number > self.last_receipt);
        let mut done = Vec::new();
        for (session, edit) in self.queue.edits.iter_mut() {
            let row = next.sessions.iter().find(|s| s.id == *session);
            let current = row.and_then(|s| s.queue.entries.iter().find(|e| e.id == edit.id));
            if let Some(pending) = edit.pending.clone() {
                let ours = receipt.filter(|r| {
                    r.session == *session
                        && matches!(&r.action, Action::Queue(op) if *op == pending)
                });
                match ours {
                    // Accepted: the save (or re-queue) ends the edit.
                    Some(receipt) if receipt.error.is_none() => done.push(session.clone()),
                    // Refused: the text stays, with why.
                    Some(receipt) => {
                        edit.pending = None;
                        edit.problem = Some(match current {
                            None => Problem::Gone,
                            Some(entry) if entry.text != edit.start => {
                                Problem::Conflict(entry.text.clone())
                            }
                            Some(_) => Problem::Refused(receipt.error.clone().unwrap_or_default()),
                        });
                    }
                    // Another control won admission first: nothing was sent.
                    None if receipt.is_some() && !next.busy => {
                        edit.pending = None;
                        edit.problem = Some(Problem::Refused(
                            "The change was not sent; try again.".into(),
                        ));
                    }
                    None => {}
                }
                continue;
            }
            // Only a row that reports the queue can say the entry left it.
            if row.is_some() && current.is_none() && edit.problem.is_none() {
                edit.problem = Some(Problem::Gone);
            }
        }
        for session in done {
            self.queue.edits.remove(&session);
        }
    }

    pub(super) fn queue_op(&mut self, op: Op, cx: &mut Context<Self>) {
        self.act(Action::Queue(op), cx);
    }

    /// The selected session's edit.
    pub(super) fn queue_edit(&self) -> Option<&Edit> {
        self.queue.edits.get(self.view.selected.as_ref()?)
    }

    fn start_queue_edit(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        self.queue.edits.insert(
            session.clone(),
            Edit {
                id: entry.id.clone(),
                start: entry.text.clone(),
                text: entry.text.clone(),
                pending: None,
                problem: None,
            },
        );
        self.queue.shown = Some(session);
        let text = entry.text.clone();
        self.queue.editor.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Send this edit's change, keeping the editor open until the hub says.
    fn send_edit(&mut self, op: Op, cx: &mut Context<Self>) {
        let Some(edit) = self
            .view
            .selected
            .clone()
            .and_then(|s| self.queue.edits.get_mut(&s))
        else {
            return;
        };
        edit.pending = Some(op.clone());
        edit.problem = None;
        self.queue_op(op, cx);
        cx.notify();
    }

    /// Save the inline edit against the text it started from. Untouched,
    /// nothing is sent; empty, nothing is saved.
    pub(super) fn save_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self
            .queue
            .editor
            .update(cx, |input, cx| {
                gpui::EntityInputHandler::marked_text_range(input, window, cx)
            })
            .is_some_and(|range| !range.is_empty());
        if composing {
            cx.propagate();
            return;
        }
        let Some(edit) = self.queue_edit().cloned() else {
            return;
        };
        if edit.pending.is_some() || edit.problem == Some(Problem::Gone) {
            return;
        }
        let text = self.queue.editor.read(cx).value().to_string();
        if text == edit.start && edit.problem.is_none() {
            self.close_queue_edit(window, cx);
            return;
        }
        if text.trim().is_empty() {
            self.queue
                .editor
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        self.send_edit(
            Op::Update {
                id: edit.id,
                text,
                expected: Some(edit.start),
            },
            cx,
        );
    }

    /// Conflict: save mine over the newer text the hub now holds.
    fn overwrite_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        if let Some(edit) = self.queue.edits.get_mut(&session)
            && let Some(Problem::Conflict(current)) = edit.problem.take()
        {
            edit.start = current;
        }
        self.save_queue_edit(window, cx);
    }

    /// The entry is gone or the save was refused: queue the text anew.
    fn requeue_edit(&mut self, cx: &mut Context<Self>) {
        let text = self.queue.editor.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.send_edit(Op::Add(text), cx);
    }

    fn copy_edit_to_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.queue.editor.read(cx).value().to_string();
        self.close_queue_edit(window, cx);
        if !text.trim().is_empty() {
            self.prefill(&text, window, cx);
        }
    }

    /// Cancel / Discard mine: the edit ends and nothing is sent.
    pub(super) fn close_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.view.selected.clone() {
            self.queue.edits.remove(&session);
        }
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Whether `text` reached the transcript as the user's message lately:
    /// how a vanished entry is told sent from removed.
    fn queue_text_sent(&self, text: &str) -> bool {
        self.view
            .transcript
            .rows
            .iter()
            .rev()
            .take(SENT_LOOKBACK)
            .any(|r| r.role == "You" && r.text == text)
    }

    /// The queue panel, when there is anything to show. `enabled` is the
    /// composer's: adding, editing and sending need a live session, while
    /// tidying (remove, reorder, clear, dismiss, resume) works on a stopped
    /// one too, whose queue the hub keeps.
    pub(super) fn render_queue_panel(
        &self,
        session: &Session,
        enabled: bool,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let queue = &session.queue;
        let edit = self.queue_edit();
        if !(queue.visible() || edit.is_some()) || self.view.child.is_some() {
            return None;
        }
        let p = self.appearance.palette();
        let live = enabled && !session.stopped();
        let tidy = self.view.connected && !self.view.busy && !self.view.loading;
        let count = queue.entries.len();
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .child(status_dot(if queue.paused { p.warning } else { p.accent }))
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(if queue.paused { p.warning } else { p.text }))
                    .child(match (count, queue.paused) {
                        (0, true) => "Queue · paused".to_owned(),
                        (0, false) => "Queue".to_owned(),
                        (n, true) => format!("Queued · {n} · paused"),
                        (n, false) => format!("Queued · {n}"),
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .when(count > 0 && !queue.paused && !compact, |d| {
                        d.child(if count == 1 {
                            "Sends when this turn ends"
                        } else {
                            "Sends one per turn, in order"
                        })
                    }),
            )
            .when(count > 0, |d| {
                d.child(
                    self.button("queue-clear", "Clear all", tidy)
                        .debug_selector(|| "queue-clear".into())
                        .when(tidy, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.queue_op(Op::Clear, cx)))
                        }),
                )
            });
        let mut card = div()
            .debug_selector(|| "queue-panel".into())
            .occlude()
            .w_full()
            .p(px(if compact { 8. } else { 10. }))
            .rounded(px(p.panel_radius))
            .shadow(chrome::floating_shadow(p))
            .bg(rgb(p.surface))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(if compact { 4. } else { 6. }))
            .text_size(px(12.))
            .child(header);
        if queue.paused {
            card = card.child(self.render_queue_paused(queue, tidy, cx));
        }
        if let Some(uncertain) = &queue.uncertain {
            card = card.child(self.render_queue_uncertain(uncertain, tidy, live, cx));
        }
        // An edit whose entry left the queue keeps its text here.
        if let Some(edit) = edit.filter(|e| !queue.entries.iter().any(|x| x.id == e.id)) {
            card = card.child(self.render_queue_editor(edit, live, cx));
        }
        for (ix, entry) in queue.entries.iter().enumerate() {
            card = card.child(self.render_queue_entry(ix, count, entry, tidy, live, window, cx));
        }
        Some(card)
    }

    fn render_queue_paused(&self, queue: &Queue, tidy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let reason = queue
            .error
            .clone()
            .unwrap_or_else(|| "Sending is paused.".into());
        div()
            .debug_selector(|| "queue-paused".into())
            .flex()
            .items_start()
            .gap_2()
            .child(div().flex_1().min_w_0().child(chrome::notice_line(
                format!("Paused: {reason}"),
                chrome::Tone::Warning,
                p,
                "queue-paused-line",
            )))
            // Only an explicit resume (or a send now) lifts a pause.
            .child(
                self.button("queue-resume", "Resume", tidy)
                    .debug_selector(|| "queue-resume".into())
                    .when(tidy, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.queue_op(Op::Resume, cx)))
                    }),
            )
    }

    fn render_queue_uncertain(
        &self,
        uncertain: &wks_native::queue::Uncertain,
        tidy: bool,
        live: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let id = uncertain.id.clone();
        let text = uncertain.text.clone();
        let complete = uncertain.complete();
        let preview = if complete {
            text.clone()
        } else {
            format!("{text}…")
        };
        div()
            .debug_selector(|| "queue-uncertain".into())
            .p_2()
            .rounded(px(p.control_radius))
            .bg(gpui::Hsla::from(rgb(p.warning)).opacity(0.08))
            .flex()
            .flex_col()
            .gap_1()
            .child(chrome::notice_line(
                "May have been sent — check the conversation before sending it again.",
                chrome::Tone::Warning,
                p,
                "queue-uncertain-line",
            ))
            .child(
                div()
                    .pl(px(19.))
                    .text_color(rgb(p.prose))
                    .max_h(px(60.))
                    .overflow_hidden()
                    .child(preview),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_2()
                    .map(|d| {
                        if complete {
                            let (id, text) = (id.clone(), text.clone());
                            d.child(
                                self.button("queue-uncertain-resend", "Send again", live)
                                    .debug_selector(|| "queue-uncertain-resend".into())
                                    .when(live, |d| {
                                        d.on_click(cx.listener(move |this, _, _, cx| {
                                            this.queue_op(
                                                Op::Resend {
                                                    uncertain: id.clone(),
                                                    text: text.clone(),
                                                },
                                                cx,
                                            )
                                        }))
                                    }),
                            )
                        } else {
                            // The hub keeps only the first 200 characters: never
                            // resend a message that may have been cut.
                            let text = text.clone();
                            d.child(
                                self.button("queue-uncertain-copy", "Copy to composer", tidy)
                                    .debug_selector(|| "queue-uncertain-copy".into())
                                    .when(tidy, |d| {
                                        d.on_click(cx.listener(move |this, _, window, cx| {
                                            this.prefill(&text, window, cx)
                                        }))
                                    }),
                            )
                        }
                    })
                    .child(
                        self.button("queue-uncertain-dismiss", "Dismiss", tidy)
                            .debug_selector(|| "queue-uncertain-dismiss".into())
                            .when(tidy, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.queue_op(Op::Remove(id.clone()), cx)
                                }))
                            }),
                    ),
            )
    }

    /// The inline editor with what stands in the way of saving, if anything.
    fn render_queue_editor(&self, edit: &Edit, live: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let saving = edit.pending.is_some();
        let action = |id: &'static str, label: &'static str, primary: bool, enabled: bool| {
            if primary {
                self.primary_button(id, label, enabled)
            } else {
                self.button(id, label, enabled)
            }
            .debug_selector(move || id.into())
        };
        let problem: Option<(String, Div)> = match &edit.problem {
            Some(Problem::Conflict(current)) => Some((
                "Changed elsewhere since you started editing. Now it reads:".into(),
                div()
                    .debug_selector(|| "queue-edit-conflict".into())
                    .pl(px(19.))
                    .max_h(px(80.))
                    .overflow_hidden()
                    .text_color(rgb(p.prose))
                    .child(current.clone()),
            )),
            Some(Problem::Gone) => Some((
                if self.queue_text_sent(&edit.start) {
                    "That message was sent while you were editing; your text is kept here.".into()
                } else {
                    "That message was removed elsewhere while you were editing; your text is kept here."
                        .into()
                },
                div().debug_selector(|| "queue-edit-gone".into()),
            )),
            Some(Problem::Refused(error)) => Some((
                format!("Not saved: {error}"),
                div().debug_selector(|| "queue-edit-refused".into()),
            )),
            None => None,
        };
        let buttons = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(if saving {
                        "Saving…"
                    } else {
                        "Enter saves · Shift Enter for a new line · Esc cancels"
                    }),
            )
            .map(|d| match &edit.problem {
                Some(Problem::Conflict(_)) => d
                    .child(
                        action("queue-edit-discard", "Discard mine", false, !saving).on_click(
                            cx.listener(|this, _, window, cx| this.close_queue_edit(window, cx)),
                        ),
                    )
                    .child(
                        action("queue-edit-overwrite", "Overwrite", true, live && !saving).when(
                            live && !saving,
                            |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.overwrite_queue_edit(window, cx)
                                }))
                            },
                        ),
                    ),
                Some(Problem::Gone | Problem::Refused(_)) => d
                    .child(
                        action("queue-edit-discard", "Discard", false, !saving).on_click(
                            cx.listener(|this, _, window, cx| this.close_queue_edit(window, cx)),
                        ),
                    )
                    .child(
                        action("queue-edit-copy", "Copy to composer", false, !saving).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.copy_edit_to_composer(window, cx)
                            }),
                        ),
                    )
                    .child(
                        action("queue-edit-requeue", "Queue as new", true, live && !saving)
                            .when(live && !saving, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.requeue_edit(cx)))
                            }),
                    ),
                None => d
                    .child(action("queue-edit-cancel", "Cancel", false, true).on_click(
                        cx.listener(|this, _, window, cx| this.close_queue_edit(window, cx)),
                    ))
                    .child(
                        action("queue-edit-save", "Save", true, live && !saving).when(
                            live && !saving,
                            |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.save_queue_edit(window, cx)
                                }))
                            },
                        ),
                    ),
            });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .when_some(problem, |d, (line, detail)| {
                d.child(chrome::notice_line(
                    line,
                    chrome::Tone::Warning,
                    p,
                    "queue-edit-problem",
                ))
                .child(detail)
            })
            .child(
                div()
                    .debug_selector(|| "queue-editor".into())
                    .key_context("QueueEditor")
                    .on_action(cx.listener(|this, _: &QueueEditSave, window, cx| {
                        this.save_queue_edit(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &QueueEditCancel, window, cx| {
                        this.close_queue_edit(window, cx)
                    }))
                    .child(Input::new(&self.queue.editor)),
            )
            .child(buttons)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_queue_entry(
        &self,
        ix: usize,
        count: usize,
        entry: &Entry,
        tidy: bool,
        live: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let id = entry.id.clone();
        let open = self.queue.expanded.contains(&id);
        let foldable = long(&entry.text);
        let number = div()
            .flex_shrink_0()
            .w(px(16.))
            .pt(px(2.))
            .text_size(px(chrome::scale::CAPTION))
            .text_color(rgb(p.muted))
            .child(format!("{}", ix + 1));
        let row = div()
            .debug_selector(move || format!("queue-entry-{ix}"))
            .flex()
            .items_start()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(p.control_radius))
            .bg(rgb(p.base))
            .child(number);
        if let Some(edit) = self.queue_edit().filter(|e| e.id == id) {
            return row.child(self.render_queue_editor(edit, live, cx));
        }
        let text = div()
            .debug_selector(move || format!("queue-entry-{ix}-text"))
            .flex_1()
            .min_w_0()
            .text_color(rgb(p.prose))
            .map(|d| {
                if open {
                    d.child(
                        div()
                            .id(SharedString::from(format!("queue-full-{id}")))
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .child(super::transcript::literal(
                                format!("queue-literal-{id}"),
                                &entry.text,
                                window,
                                cx,
                            )),
                    )
                } else {
                    d.truncate().child(
                        entry
                            .text
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(PREVIEW_CHARS)
                            .collect::<String>(),
                    )
                }
            });
        let move_to = |target: usize| {
            let id = id.clone();
            cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, _, cx| {
                this.queue_op(
                    Op::Move {
                        id: id.clone(),
                        index: target,
                    },
                    cx,
                )
            })
        };
        let (up, down) = (ix > 0 && tidy, ix + 1 < count && tidy);
        let edit_entry = entry.clone();
        let (send_id, remove_id) = (id.clone(), id.clone());
        row.child(text)
            .when(foldable, |d| {
                let toggle_id = id.clone();
                d.child(
                    self.button(
                        SharedString::from(format!("queue-expand-{id}")),
                        if open { "Less" } else { "More" },
                        true,
                    )
                    .debug_selector(move || format!("queue-entry-{ix}-expand"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.queue.expanded.remove(&toggle_id) {
                            if this.queue.expanded.len() > 64 {
                                this.queue.expanded.clear();
                            }
                            this.queue.expanded.insert(toggle_id.clone());
                        }
                        cx.notify();
                    })),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        self.icon_button(
                            SharedString::from(format!("queue-up-{id}")),
                            "Move up",
                            IconName::ChevronUp,
                            up,
                        )
                        .debug_selector(move || format!("queue-entry-{ix}-up"))
                        .when(up, |d| d.on_click(move_to(ix - 1))),
                    )
                    .child(
                        self.icon_button(
                            SharedString::from(format!("queue-down-{id}")),
                            "Move down",
                            IconName::ChevronDown,
                            down,
                        )
                        .debug_selector(move || format!("queue-entry-{ix}-down"))
                        .when(down, |d| d.on_click(move_to(ix + 1))),
                    )
                    .child(
                        self.button(SharedString::from(format!("queue-edit-{id}")), "Edit", live)
                            .debug_selector(move || format!("queue-entry-{ix}-edit"))
                            .when(live, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_queue_edit(&edit_entry, window, cx)
                                }))
                            }),
                    )
                    .child(
                        self.button(
                            SharedString::from(format!("queue-send-{id}")),
                            "Send now",
                            live,
                        )
                        .debug_selector(move || format!("queue-entry-{ix}-send"))
                        .when(live, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_op(Op::SendNow(send_id.clone()), cx)
                            }))
                        }),
                    )
                    .child(
                        self.icon_button(
                            SharedString::from(format!("queue-remove-{id}")),
                            "Remove from queue",
                            IconName::Close,
                            tidy,
                        )
                        .debug_selector(move || format!("queue-entry-{ix}-remove"))
                        .when(tidy, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_op(Op::Remove(remove_id.clone()), cx)
                            }))
                        }),
                    ),
            )
    }
}
