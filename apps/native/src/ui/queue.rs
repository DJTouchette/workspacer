//! The selected session's message queue, docked above the composer: what the
//! hub will send when the turn ends, with edit, reorder, remove and send now,
//! the paused state and its reason, and a send that may have gone out.
//!
//! The hub owns the queue (see `wks_native::queue`); every control here is
//! one `sessions.queue*` call, and the panel redraws from the next snapshot
//! row. Nothing is ever resent on the user's behalf: a may-have-been-sent
//! message offers Send again (or, when the hub's preview may be cut, copies
//! it to the composer) and Dismiss.
use super::*;
use wks_native::queue::{Entry, Op, Queue};

/// Characters of an entry shown before it needs expanding.
const PREVIEW_CHARS: usize = 140;

pub(super) struct QueueUi {
    /// The inline editor for the entry being edited.
    pub(super) editor: Entity<InputState>,
    /// `(session, entry id)` being edited.
    pub(super) editing: Option<(String, String)>,
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
            editing: None,
            expanded: Default::default(),
        }
    }
}

/// Long or multi-line text folds behind Show all.
fn long(text: &str) -> bool {
    text.lines().nth(1).is_some() || text.chars().count() > PREVIEW_CHARS
}

impl Workspace {
    /// An edit whose entry the hub already sent (or someone removed) ends,
    /// keeping the typed text in the composer rather than losing it.
    pub(super) fn sync_queue(&mut self, next: &View, window: &mut Window, cx: &mut Context<Self>) {
        let Some((session, id)) = self.queue.editing.clone() else {
            return;
        };
        let alive = next
            .sessions
            .iter()
            .find(|s| s.id == session)
            .is_some_and(|s| s.queue.entries.iter().any(|e| e.id == id));
        if alive && next.selected.as_ref() == Some(&session) {
            return;
        }
        self.queue.editing = None;
        if !alive {
            let typed = self.queue.editor.read(cx).value().to_string();
            if !typed.trim().is_empty() && next.selected.as_ref() == Some(&session) {
                self.prefill(&typed, window, cx);
            }
            self.local_notice = "That queued message was sent before the edit was saved; your text is in the composer.".into();
        }
    }

    pub(super) fn queue_op(&mut self, op: Op, cx: &mut Context<Self>) {
        self.act(Action::Queue(op), cx);
    }

    fn start_queue_edit(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        self.queue.editing = Some((session, entry.id.clone()));
        let text = entry.text.clone();
        self.queue.editor.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Save the inline edit: an unchanged or empty text saves nothing.
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
        let Some((session, id)) = self.queue.editing.clone() else {
            return;
        };
        let text = self.queue.editor.read(cx).value().trim_end().to_owned();
        if text.trim().is_empty() {
            self.queue
                .editor
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let unchanged = self
            .selected_session()
            .filter(|s| s.id == session)
            .and_then(|s| s.queue.entries.iter().find(|e| e.id == id))
            .is_some_and(|e| e.text == text);
        self.queue.editing = None;
        if !unchanged {
            self.queue_op(Op::Update { id, text }, cx);
        }
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn cancel_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue.editing = None;
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// The queue panel, when there is anything to show.
    pub(super) fn render_queue_panel(
        &self,
        session: &Session,
        enabled: bool,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let queue = &session.queue;
        if !queue.visible() || self.view.child.is_some() {
            return None;
        }
        let p = self.appearance.palette();
        let live = enabled && !session.stopped();
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
                        (0, _) => "Queue".to_owned(),
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
                    self.button("queue-clear", "Clear all", enabled)
                        .debug_selector(|| "queue-clear".into())
                        .when(enabled, |d| {
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
            card = card.child(self.render_queue_paused(queue, live, cx));
        }
        if let Some(uncertain) = &queue.uncertain {
            card = card.child(self.render_queue_uncertain(uncertain, enabled, live, cx));
        }
        for (ix, entry) in queue.entries.iter().enumerate() {
            card = card.child(self.render_queue_entry(ix, count, entry, enabled, live, window, cx));
        }
        Some(card)
    }

    fn render_queue_paused(&self, queue: &Queue, live: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let reason = queue
            .error
            .clone()
            .unwrap_or_else(|| "Sending is paused.".into());
        // Any change resumes the hub's queue; moving the head to where it
        // already is changes nothing else.
        let head = queue.entries.first().map(|e| e.id.clone());
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
            .when_some(head, |d, head| {
                d.child(
                    self.button("queue-resume", "Resume", live)
                        .debug_selector(|| "queue-resume".into())
                        .when(live, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_op(
                                    Op::Move {
                                        id: head.clone(),
                                        index: 0,
                                    },
                                    cx,
                                )
                            }))
                        }),
                )
            })
    }

    fn render_queue_uncertain(
        &self,
        uncertain: &wks_native::queue::Uncertain,
        enabled: bool,
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
                                self.button("queue-uncertain-copy", "Copy to composer", enabled)
                                    .debug_selector(|| "queue-uncertain-copy".into())
                                    .when(enabled, |d| {
                                        d.on_click(cx.listener(move |this, _, window, cx| {
                                            this.prefill(&text, window, cx)
                                        }))
                                    }),
                            )
                        }
                    })
                    .child(
                        self.button("queue-uncertain-dismiss", "Dismiss", enabled)
                            .debug_selector(|| "queue-uncertain-dismiss".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.queue_op(Op::Remove(id.clone()), cx)
                                }))
                            }),
                    ),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_queue_entry(
        &self,
        ix: usize,
        count: usize,
        entry: &Entry,
        enabled: bool,
        live: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let id = entry.id.clone();
        let editing = self
            .queue
            .editing
            .as_ref()
            .is_some_and(|(session, editing)| {
                Some(session) == self.view.selected.as_ref() && *editing == id
            });
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
        if editing {
            return row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| "queue-editor".into())
                            .key_context("QueueEditor")
                            .on_action(cx.listener(|this, _: &QueueEditSave, window, cx| {
                                this.save_queue_edit(window, cx)
                            }))
                            .on_action(cx.listener(|this, _: &QueueEditCancel, window, cx| {
                                this.cancel_queue_edit(window, cx)
                            }))
                            .child(Input::new(&self.queue.editor)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(
                                        "Enter saves · Shift Enter for a new line · Esc cancels",
                                    ),
                            )
                            .child(self.button("queue-edit-cancel", "Cancel", true).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.cancel_queue_edit(window, cx)
                                }),
                            ))
                            .child(
                                self.primary_button("queue-edit-save", "Save", live)
                                    .debug_selector(|| "queue-edit-save".into())
                                    .when(live, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.save_queue_edit(window, cx)
                                        }))
                                    }),
                            ),
                    ),
            );
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
        let (up, down) = (ix > 0 && enabled, ix + 1 < count && enabled);
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
                            enabled,
                        )
                        .debug_selector(move || format!("queue-entry-{ix}-remove"))
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_op(Op::Remove(remove_id.clone()), cx)
                            }))
                        }),
                    ),
            )
    }
}
