//! History and session controls beyond chat: the collapsed "Thinking" block,
//! per-message Rewind files / Undo from here / Fork from here, and the
//! Session page's Context, MCP and Codex cards (compact, fork, goal, review,
//! Windows sandbox). Every control is shown only where the session's
//! provider, transport and the connection's access can carry it out.
use super::*;
use gpui::AnyElement;
use gpui_component::tooltip::Tooltip;
use wks_native::{
    codex_extras::{GoalChange, ReviewTarget},
    features::Request,
    model::Row,
    session_controls::{GoBack, Rewind, refusal, usable_uuid},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MessageActionKind {
    Rewind,
    Undo,
    Fork,
}

/// The confirmation open under one of the user's messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MessageAction {
    pub session: String,
    pub row: u64,
    pub uuid: String,
    pub kind: MessageActionKind,
    /// Rewind: the real call has been sent (after its preview).
    pub applied: bool,
    /// When the latest rewind call was asked for, to say when it is late.
    pub asked_at: std::time::Instant,
}

/// How long a rewind may go unanswered before the panel offers Retry (the
/// daemon itself gives up after 30 seconds).
pub(super) const REWIND_WAIT: std::time::Duration = std::time::Duration::from_secs(35);

/// What Review changes reviews; the text field supplies the branch, commit
/// or instructions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ReviewKind {
    #[default]
    Uncommitted,
    Branch,
    Commit,
    Custom,
}

impl ReviewKind {
    const ALL: [Self; 4] = [Self::Uncommitted, Self::Branch, Self::Commit, Self::Custom];
    fn label(self) -> &'static str {
        match self {
            Self::Uncommitted => "Uncommitted changes",
            Self::Branch => "Against a branch",
            Self::Commit => "A commit",
            Self::Custom => "Custom",
        }
    }
    fn placeholder(self) -> &'static str {
        match self {
            Self::Uncommitted => "",
            Self::Branch => "Base branch, such as main",
            Self::Commit => "Commit SHA",
            Self::Custom => "What should the review look at?",
        }
    }
    fn target(self, text: &str) -> ReviewTarget {
        match self {
            Self::Uncommitted => ReviewTarget::Uncommitted,
            Self::Branch => ReviewTarget::BaseBranch(text.to_owned()),
            Self::Commit => ReviewTarget::Commit {
                sha: text.to_owned(),
                title: None,
            },
            Self::Custom => ReviewTarget::Custom(text.to_owned()),
        }
    }
}

/// Inputs and choices the Session page's cards keep between frames.
pub(super) struct ToolsUi {
    pub message_action: Option<MessageAction>,
    /// The card id the question picker was built for (captured when the
    /// question set appeared), sent with its answers.
    pub question_card: Option<String>,
    /// The card id a "Deny with note" box was opened on.
    pub deny_card: Option<String>,
    /// An MCP link to open once its Open answer is accepted:
    /// (question signature, URL).
    pub open_after: Option<(String, String)>,
    /// The question set whose link was opened (each card opens at most once).
    pub opened: Option<String>,
    /// Wakes the rewind panel when its call is late.
    pub rewind_timer: Option<Task<()>>,
    pub compact_note: Entity<InputState>,
    pub review_kind: ReviewKind,
    pub review_text: Entity<InputState>,
    pub goal_objective: Entity<InputState>,
    pub goal_budget: Entity<InputState>,
}

impl ToolsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            message_action: None,
            question_card: None,
            deny_card: None,
            open_after: None,
            opened: None,
            rewind_timer: None,
            compact_note: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Optional: what the summary should keep")
            }),
            review_kind: ReviewKind::default(),
            review_text: cx.new(|cx| InputState::new(window, cx)),
            goal_objective: cx
                .new(|cx| InputState::new(window, cx).placeholder("What should Codex achieve?")),
            goal_budget: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Token budget (optional), such as 50k")
            }),
        }
    }
}

fn panel(p: Palette) -> Div {
    div()
        .w_full()
        .mt_1()
        .p_3()
        .rounded(px(p.control_radius))
        .bg(rgb(p.surface))
        .border_1()
        .border_color(rgb(p.border))
        .flex()
        .flex_col()
        .gap_2()
        .text_size(px(12.))
}

impl Workspace {
    /// The model's reasoning: a quiet, collapsed line that opens into its
    /// text. While it streams the line pulses beside the spinner and follows
    /// the newest line of thought; once something follows it, it reads
    /// "Thought for 12s" with the block's opening line as its title.
    pub(super) fn render_thinking(
        &self,
        row: &Row,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let open_key = format!("{key}-thinking");
        let open = self.chat.open.get(&open_key).copied().unwrap_or(false);
        let rows = &self.view.transcript.rows;
        let at = rows
            .binary_search_by_key(&row.key, |r| r.key)
            .ok()
            .filter(|ix| rows[*ix].key == row.key);
        let next = at.and_then(|ix| rows.get(ix + 1));
        let working = self.view.connected && self.selected_session().is_some_and(|s| s.working());
        let live = working && next.is_none();
        let label = if live {
            "Thinking".to_owned()
        } else {
            next.and_then(|n| Some(n.timestamp_ms? - row.timestamp_ms?))
                .filter(|ms| (1_000..3_600_000).contains(ms))
                .map(|ms| format!("Thought for {}", timing::duration_label(ms)))
                .unwrap_or_else(|| "Thought".into())
        };
        let mut lines = row
            .text
            .lines()
            .map(thinking_line)
            .filter(|l| !l.is_empty());
        let preview = if live {
            lines.next_back()
        } else {
            lines.next()
        }
        .unwrap_or_default();
        let toggle_key = open_key.clone();
        let title = div().flex_shrink_0().child(label);
        div()
            .debug_selector(|| "thinking-row".into())
            .w_full()
            .px_3()
            .py_1()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(rgb(p.muted))
            .child(
                div()
                    .id(SharedString::from(format!("{open_key}-toggle")))
                    .debug_selector(|| "thinking-toggle".into())
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(rgb(p.text)))
                    .child(
                        // Wide enough for the spinner, so the label never
                        // moves when the block finishes.
                        div()
                            .w(px(22.))
                            .flex()
                            .justify_center()
                            .flex_shrink_0()
                            .child(if live && !open {
                                brand_spinner(10., p)
                            } else {
                                div().child(
                                    Icon::new(if open {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(11.)),
                                )
                            }),
                    )
                    .child(if live {
                        // A slow breath, so the line reads as alive without
                        // pulling the eye from the reply.
                        let t = gpui::bounce(gpui::ease_in_out)(motion::looping(
                            std::time::Duration::from_millis(1800),
                            0.5,
                        ));
                        title
                            .font_weight(FontWeight::MEDIUM)
                            .opacity(0.55 + 0.45 * t)
                            .into_any_element()
                    } else {
                        title.into_any_element()
                    })
                    .when(!open && !preview.is_empty(), |d| {
                        d.child(
                            div()
                                .debug_selector(|| "thinking-preview".into())
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(p.disabled))
                                .child(preview),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle_key.clone(), false, cx)
                    })),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .debug_selector(|| "thinking-body".into())
                        .mt_1()
                        // Under the label, not the chevron: the rule hangs
                        // from the icon's centre.
                        .ml(px(10.5))
                        .pl(px(16.5))
                        .py_0p5()
                        .border_l_1()
                        .border_color(rgb(p.border))
                        // Reasoning is Markdown (bold step titles, lists,
                        // code); render it as the reply would, but muted.
                        .child(
                            self.render_markdown(
                                &format!("{key}-thinking-text"),
                                &row.text,
                                window,
                                cx,
                            )
                            .text_color(rgb(p.muted))
                            .line_height(gpui::relative(1.55)),
                        ),
                )
            })
    }

    /// Hover actions on one of the user's live messages. Only a message the
    /// provider can name (its `uuid`) offers them; steered messages never do.
    pub(super) fn message_actions(&self, row: &Row, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(session) = self.selected_session() else {
            return Vec::new();
        };
        if !usable_uuid(&row.uuid) || !self.view.connected {
            return Vec::new();
        }
        let mut actions = Vec::new();
        let mut push = |kind: MessageActionKind,
                        label: &'static str,
                        icon: IconName,
                        selector: &'static str| {
            let uuid = row.uuid.clone();
            let key = row.key;
            actions.push(
                self.icon_button(
                    SharedString::from(format!("{selector}-{}", row.key)),
                    label,
                    icon,
                    true,
                )
                .debug_selector(move || selector.into())
                .size(px(24.))
                .opacity(0.)
                .group_hover("message", |style| style.opacity(1.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_message_action(key, uuid.clone(), kind, cx)
                }))
                .into_any_element(),
            );
        };
        if !session.stopped()
            && let Some(go) = GoBack::of(session.provider_id(), &session.transport)
            && self.view.may_call(go.method())
        {
            match go {
                GoBack::RewindFiles => push(
                    MessageActionKind::Rewind,
                    go.label(),
                    IconName::Undo,
                    "rewind-files",
                ),
                GoBack::Revert => push(
                    MessageActionKind::Undo,
                    go.label(),
                    IconName::Undo,
                    "undo-from-here",
                ),
            }
        }
        // Codex finds a message's fork point through its live driver, so a
        // stopped Codex session forks only as a whole (Session details).
        if matches!(session.provider_id(), "claude" | "codex")
            && !(session.provider_id() == "codex" && session.stopped())
            && self.view.may_call("agents.fork")
        {
            push(
                MessageActionKind::Fork,
                "Fork from here",
                IconName::Plus,
                "fork-from-here",
            );
        }
        actions
    }

    fn open_message_action(
        &mut self,
        row: u64,
        uuid: String,
        kind: MessageActionKind,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        self.tools.message_action = Some(MessageAction {
            session,
            row,
            uuid,
            kind,
            applied: false,
            asked_at: std::time::Instant::now(),
        });
        if kind == MessageActionKind::Rewind {
            // The preview first: nothing changes until the user has seen what
            // would be restored. It supersedes any other preview in flight.
            self.ask_rewind(false, cx);
        }
        self.remeasure_rows(cx);
    }

    /// Send the open panel's rewind call (the preview, or the real one) and
    /// wake the panel if it goes unanswered.
    fn ask_rewind(&mut self, apply: bool, cx: &mut Context<Self>) {
        let Some(action) = self.tools.message_action.as_mut() else {
            return;
        };
        action.applied = apply;
        action.asked_at = std::time::Instant::now();
        let request = Request::RewindFiles {
            session: action.session.clone(),
            message: action.uuid.clone(),
            preview: !apply,
        };
        self.request(request, cx);
        self.tools.rewind_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(REWIND_WAIT + std::time::Duration::from_millis(100))
                .await;
            let _ = this.update(cx, |this, cx| this.remeasure_rows(cx));
        }));
        self.remeasure_rows(cx);
    }

    fn close_message_action(&mut self, cx: &mut Context<Self>) {
        self.tools.message_action = None;
        self.tools.rewind_timer = None;
        self.remeasure_rows(cx);
    }

    /// The transcript caches row heights; a panel that opens, closes or
    /// changes inside a row must have them measured again.
    fn remeasure_rows(&mut self, cx: &mut Context<Self>) {
        let anchor = self.scroll_anchor();
        let count = self.view.transcript.rows.len();
        self.list.splice(0..count, count);
        self.list.scroll_to(anchor);
        cx.notify();
    }

    /// Before a new view lands: a panel for another session, or for a message
    /// that has gone, closes; a rewind answer re-measures its row.
    pub(super) fn sync_message_action(&mut self, next: &View, cx: &mut Context<Self>) {
        let Some(action) = &self.tools.message_action else {
            return;
        };
        let gone = next.selected.as_ref() != Some(&action.session)
            || next.child.is_some()
            || !next
                .transcript
                .rows
                .iter()
                .any(|r| r.key == action.row && r.uuid == action.uuid);
        if gone {
            self.tools.message_action = None;
            return;
        }
        let state = |view: &View| {
            view.requests
                .get("rewind-files")
                .map(|s| (s.number, s.loading))
        };
        if state(&self.view) != state(next) {
            let count = next
                .transcript
                .rows
                .len()
                .max(self.view.transcript.rows.len());
            let anchor = self.scroll_anchor();
            self.list.splice(0..self.view.transcript.rows.len(), count);
            self.list.scroll_to(anchor);
            cx.notify();
        }
    }

    /// The confirmation under a message, while one is open for it.
    pub(super) fn render_message_action(&self, row: &Row, cx: &mut Context<Self>) -> Option<Div> {
        let action = self.tools.message_action.as_ref()?;
        if action.row != row.key || self.view.selected.as_ref() != Some(&action.session) {
            return None;
        }
        let p = self.appearance.palette();
        let enabled = self.view.connected && !self.view.busy;
        let cancel = |label: &'static str| {
            self.button("message-action-cancel", label, true)
                .debug_selector(|| "message-action-cancel".into())
                .on_click(cx.listener(|this, _, _, cx| this.close_message_action(cx)))
        };
        let card = panel(p).debug_selector(|| "message-action-panel".into());
        Some(match action.kind {
            MessageActionKind::Rewind => {
                let state = self.view.requests.get("rewind-files").filter(|s| {
                    matches!(&s.request, Request::RewindFiles { session, message, preview }
                        if *session == action.session && *message == action.uuid
                            && *preview != action.applied)
                });
                let line = |text: String, tone: chrome::Tone| {
                    chrome::notice_line(text, tone, p)
                        .debug_selector(|| "message-action-status".into())
                };
                // Retry asks for a fresh preview: it changes nothing, so it is
                // always safe, even after a real rewind's outcome went unknown.
                let retry = || {
                    self.button("rewind-retry", "Retry", enabled)
                        .debug_selector(|| "rewind-retry".into())
                        .when(enabled, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.ask_rewind(false, cx)))
                        })
                };
                let late = action.asked_at.elapsed() >= REWIND_WAIT;
                let other_real = self.view.requests.get("rewind-files").is_some_and(|s| {
                    s.loading && matches!(&s.request, Request::RewindFiles { preview: false, .. })
                });
                let buttons = |retry_too: bool, close: &'static str| {
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(cancel(close))
                        .when(retry_too, |d| d.child(retry()))
                };
                match state {
                    None if other_real => card
                        .child(line(
                            "Another rewind is still finishing; try again when it has.".into(),
                            chrome::Tone::Warning,
                        ))
                        .child(buttons(true, "Cancel")),
                    None | Some(_)
                        if late && state.is_none_or(|s| s.loading) && !action.applied =>
                    {
                        card.child(line(
                            "No answer from Claude yet. It may still be busy; try again.".into(),
                            chrome::Tone::Error,
                        ))
                        .child(buttons(true, "Cancel"))
                    }
                    None => card
                        .child(line(
                            "Checking what would change…".into(),
                            chrome::Tone::Loading,
                        ))
                        .child(buttons(false, "Cancel")),
                    Some(s) if s.loading => card
                        .child(line(
                            if action.applied {
                                "Rewinding files…".into()
                            } else {
                                "Checking what would change…".into()
                            },
                            chrome::Tone::Loading,
                        ))
                        .child(buttons(
                            false,
                            if action.applied { "Close" } else { "Cancel" },
                        )),
                    Some(s) if s.error.is_some() => card
                        .child(line(
                            refusal(s.error.as_deref().unwrap_or_default()),
                            chrome::Tone::Error,
                        ))
                        .child(buttons(true, "Close")),
                    Some(s) if action.applied => {
                        let result = Rewind::parse(&s.value);
                        let (text, tone) = match result.error {
                            Some(error) => (error, chrome::Tone::Error),
                            None => (
                                "Files restored to how they were at this message.".to_owned(),
                                chrome::Tone::Success,
                            ),
                        };
                        card.child(line(text, tone))
                            .child(div().flex().justify_end().child(cancel("Close")))
                    }
                    Some(s) => {
                        let preview = Rewind::parse(&s.value);
                        let can = preview.can_rewind && preview.error.is_none();
                        let more = preview.files.len().saturating_sub(8);
                        card.child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(p.text))
                                .child("Rewind files to this message?"),
                        )
                        .child(line(
                            preview.summary(),
                            if can {
                                chrome::Tone::Info
                            } else {
                                chrome::Tone::Error
                            },
                        ))
                        .children(preview.files.iter().take(8).map(|file| {
                            div()
                                .pl(px(19.))
                                .truncate()
                                .font_family(mono_font())
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(file.clone())
                        }))
                        .when(more > 0, |d| {
                            d.child(
                                div()
                                    .pl(px(19.))
                                    .text_color(rgb(p.muted))
                                    .child(format!("and {more} more")),
                            )
                        })
                        .when(can, |d| {
                            d.child(
                                div().text_color(rgb(p.muted)).child(
                                    "Only files are restored; the conversation stays as it is.",
                                ),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(cancel("Cancel"))
                                .when(can, |d| {
                                    d.child(
                                        self.danger_button(
                                            "rewind-files-confirm",
                                            "Rewind files",
                                            enabled,
                                        )
                                        .debug_selector(|| "rewind-files-confirm".into())
                                        .when(
                                            enabled,
                                            |d| {
                                                d.on_click(cx.listener(|this, _, _, cx| {
                                                    this.confirm_rewind(cx)
                                                }))
                                            },
                                        ),
                                    )
                                }),
                        )
                    }
                }
            }
            MessageActionKind::Undo => {
                let uuid = action.uuid.clone();
                card.child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(p.text))
                        .child("Undo from this message?"),
                )
                .child(div().text_color(rgb(p.muted)).child(
                    "Codex drops this message and every turn after it; the conversation goes back to just before it. Files the agent changed are not restored.",
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(cancel("Cancel"))
                        .child(
                            self.danger_button("undo-confirm", "Undo from here", enabled)
                                .debug_selector(|| "undo-confirm".into())
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.act(Action::Revert(uuid.clone()), cx);
                                        this.close_message_action(cx);
                                    }))
                                }),
                        ),
                )
            }
            MessageActionKind::Fork => {
                let (session, uuid) = (action.session.clone(), action.uuid.clone());
                let forking = self.view.requests.get("fork").is_some_and(|s| s.loading);
                card.child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(p.text))
                        .child("Fork from this message?"),
                )
                .child(div().text_color(rgb(p.muted)).child(
                    "A new session starts with the conversation before this message. This one keeps running unchanged.",
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(cancel("Cancel"))
                        .child(
                            self.primary_button("fork-confirm", "Fork", enabled && !forking)
                                .debug_selector(|| "fork-confirm".into())
                                .when(enabled && !forking, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.request(
                                            Request::Fork {
                                                session: session.clone(),
                                                at: Some(uuid.clone()),
                                                label: None,
                                            },
                                            cx,
                                        );
                                        this.close_message_action(cx);
                                    }))
                                }),
                        ),
                )
            }
        })
    }

    fn confirm_rewind(&mut self, cx: &mut Context<Self>) {
        self.ask_rewind(true, cx);
    }

    /// Session page: compaction and forking for a managed session.
    pub(super) fn render_context_card(&self, s: &Session, cx: &mut Context<Self>) -> Option<Div> {
        let p = self.appearance.palette();
        let managed = matches!(s.provider_id(), "claude" | "codex");
        let compact = managed && self.view.may_call("claude.compact");
        let fork = managed && self.view.may_call("agents.fork");
        if !compact && !fork {
            return None;
        }
        let live = self.view.connected && !s.stopped();
        let can_compact = live && !self.view.busy && !s.compacting;
        let forking = self.view.requests.get("fork").filter(|state| {
            matches!(&state.request, Request::Fork { session, at: None, .. } if *session == s.id)
        });
        let history = match (s.compaction_count, s.last_compact_at) {
            (0, _) => None,
            (n, at) => Some(format!(
                "Compacted {n} time{}{}",
                if n == 1 { "" } else { "s" },
                at.map(|at| format!(
                    " · last {}",
                    timing::timestamp_label(
                        Some(at),
                        timing::now_ms(),
                        self.settings.twelve_hour_clock
                    )
                ))
                .unwrap_or_default()
            )),
        };
        Some(
            chrome::card(p)
                .debug_selector(|| "context-card".into())
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(projects::section_label("Context and history", p))
                .when(compact, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.tools.compact_note)),
                            )
                            .child(
                                self.button("compact-context", "Compact context", can_compact)
                                    .debug_selector(|| "compact-context".into())
                                    .flex_shrink_0()
                                    .when(can_compact, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.compact_context(window, cx)
                                        }))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(if s.compacting {
                                "Compacting the conversation now…".to_owned()
                            } else {
                                history.clone().unwrap_or_else(|| {
                                    "Summarises the conversation so far to free context. The agent keeps working from the summary.".into()
                                })
                            }),
                    )
                })
                .when(fork, |d| {
                    let id = s.id.clone();
                    let busy = forking.is_some_and(|f| f.loading);
                    let enabled = self.view.connected && !busy;
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                self.button("fork-session", "Fork session", enabled)
                                    .debug_selector(|| "fork-session".into())
                                    .when(enabled, |d| {
                                        d.on_click(cx.listener(move |this, _, _, cx| {
                                            this.request(
                                                Request::Fork {
                                                    session: id.clone(),
                                                    at: None,
                                                    label: None,
                                                },
                                                cx,
                                            )
                                        }))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(match forking {
                                        Some(f) if f.loading => "Forking…".to_owned(),
                                        Some(f) if f.error.is_some() => {
                                            refusal(f.error.as_deref().unwrap_or_default())
                                        }
                                        _ => "A new session with this conversation; this one keeps running.".to_owned(),
                                    }),
                            ),
                    )
                }),
        )
    }

    pub(super) fn compact_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let note = self.tools.compact_note.read(cx).value().trim().to_owned();
        self.tools
            .compact_note
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.act(Action::Compact(note), cx);
    }

    /// Session page: MCP servers as the provider reports them, with
    /// Reconnect for a failed one.
    pub(super) fn render_mcp_card(&self, s: &Session, cx: &mut Context<Self>) -> Option<Div> {
        let servers = s.status.mcp_servers.as_ref()?;
        let p = self.appearance.palette();
        let live = self.view.connected && !s.stopped() && !self.view.busy;
        let reconnect = live && self.view.may_call("claude.mcpReconnect");
        let refresh = live && self.view.may_call("sessions.mcpStatus");
        Some(
            chrome::card(p)
                .debug_selector(|| "mcp-card".into())
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(projects::section_label("MCP servers", p)),
                        )
                        .child(
                            self.button("mcp-refresh", "Refresh", refresh)
                                .debug_selector(|| "mcp-refresh".into())
                                .when(refresh, |d| {
                                    d.on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.act(Action::McpStatus, cx)
                                        }),
                                    )
                                }),
                        ),
                )
                .when(s.provider_id() == "codex" && servers.iter().any(|m| m.failed()), |d| {
                    d.child(
                        div()
                            .debug_selector(|| "mcp-codex-reload".into())
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child("Codex reconnects by reloading every MCP server of this session, not only the one you choose."),
                    )
                })
                .when(servers.is_empty(), |d| {
                    d.child(
                        div()
                            .text_color(rgb(p.muted))
                            .text_size(px(12.))
                            .child("This session has no MCP servers."),
                    )
                })
                .children(servers.iter().enumerate().map(|(ix, server)| {
                    let color = match server.status.as_str() {
                        "connected" => p.success,
                        "failed" => p.error,
                        "needs-auth" | "pending" => p.warning,
                        _ => p.muted,
                    };
                    let name = server.name.clone();
                    div()
                        .debug_selector(move || format!("mcp-server-{ix}"))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .child(status_dot(color))
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_weight(FontWeight::MEDIUM)
                                .child(server.name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(p.muted))
                                .child(match &server.error {
                                    Some(error) => format!("{} · {error}", server.status_label()),
                                    None => server.status_label().to_owned(),
                                }),
                        )
                        .when(server.failed(), |d| {
                            d.child(
                                self.button(
                                    SharedString::from(format!("mcp-reconnect-{ix}")),
                                    "Reconnect",
                                    reconnect,
                                )
                                .debug_selector(move || format!("mcp-reconnect-{ix}"))
                                .when(reconnect, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.act(Action::McpReconnect(name.clone()), cx)
                                    }))
                                }),
                            )
                        })
                }))
                .when(s.provider_id() == "claude" && !self.demo, |d| {
                    d.child(self.render_external_mcp(cx))
                }),
        )
    }

    /// Session page, Codex: the thread's goal, a review of changes and the
    /// Windows sandbox.
    pub(super) fn render_codex_card(&self, s: &Session, cx: &mut Context<Self>) -> Option<Div> {
        if s.provider_id() != "codex" {
            return None;
        }
        let p = self.appearance.palette();
        let live = self.view.connected && !s.stopped() && !self.view.busy;
        let goals = self.view.may_call("agents.setGoal");
        let review = self.view.may_call("agents.review");
        let sandbox = s
            .windows_sandbox
            .as_ref()
            .filter(|_| self.view.may_call("agents.windowsSandboxSetup"));
        if !goals && !review && sandbox.is_none() {
            return None;
        }
        let card = chrome::card(p)
            .debug_selector(|| "codex-card".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        let card = card.when(goals, |d| {
            let goal = s.goal.clone();
            let mut section = div()
                .debug_selector(|| "goal-section".into())
                .flex()
                .flex_col()
                .gap_2()
                .child(projects::section_label("Goal", p));
            if let Some(goal) = &goal {
                let status = goal.status.clone();
                let usage = goal.usage();
                section = section
                    .child(
                        div()
                            .debug_selector(|| "goal-current".into())
                            .text_size(px(13.))
                            .text_color(rgb(p.text))
                            .child(goal.objective.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(if usage.is_empty() {
                                goal.status_label()
                            } else {
                                format!("{} · {usage}", goal.status_label())
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child({
                                let (label, next) = if status == "paused" {
                                    ("Resume goal", "active")
                                } else {
                                    ("Pause goal", "paused")
                                };
                                self.button("goal-pause", label, live)
                                    .debug_selector(|| "goal-pause".into())
                                    .when(live, |d| {
                                        d.on_click(cx.listener(move |this, _, _, cx| {
                                            this.act(
                                                Action::Goal(GoalChange::Status(next.into())),
                                                cx,
                                            )
                                        }))
                                    })
                            })
                            .when(status != "complete", |d| {
                                d.child(
                                    self.button("goal-complete", "Mark complete", live)
                                        .debug_selector(|| "goal-complete".into())
                                        .when(live, |d| {
                                            d.on_click(cx.listener(|this, _, _, cx| {
                                                this.act(
                                                    Action::Goal(GoalChange::Status(
                                                        "complete".into(),
                                                    )),
                                                    cx,
                                                )
                                            }))
                                        }),
                                )
                            })
                            .child(
                                self.danger_button("goal-clear", "Clear goal", live)
                                    .debug_selector(|| "goal-clear".into())
                                    .when(live, |d| {
                                        d.on_click(cx.listener(|this, _, _, cx| {
                                            this.act(Action::Goal(GoalChange::Clear), cx)
                                        }))
                                    }),
                            ),
                    );
            }
            section = section.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .child(Input::new(&self.tools.goal_objective)),
                    )
                    .child(div().w(px(150.)).child(Input::new(&self.tools.goal_budget)))
                    .child(
                        self.primary_button(
                            "goal-save",
                            if goal.is_some() {
                                "Update goal"
                            } else {
                                "Set goal"
                            },
                            live,
                        )
                        .debug_selector(|| "goal-save".into())
                        .when(live, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| this.save_goal(window, cx)),
                            )
                        }),
                    ),
            );
            d.child(section)
        });
        let card = card.when(review, |d| {
            let kind = self.tools.review_kind;
            d.child(
                div()
                    .debug_selector(|| "review-section".into())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(projects::section_label("Review changes", p))
                    .child(div().flex().flex_wrap().gap_2().children(
                        ReviewKind::ALL.into_iter().enumerate().map(|(ix, choice)| {
                            let active = choice == kind;
                            self.button(
                                SharedString::from(format!("review-kind-{ix}")),
                                choice.label(),
                                true,
                            )
                            .debug_selector(move || format!("review-kind-{ix}"))
                            .when(active, |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.tools.review_kind = choice;
                                let placeholder = choice.placeholder();
                                this.tools.review_text.update(cx, |input, cx| {
                                    input.set_placeholder(placeholder, window, cx)
                                });
                                cx.notify();
                            }))
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(kind != ReviewKind::Uncommitted, |d| {
                                d.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.tools.review_text)),
                                )
                            })
                            .when(kind == ReviewKind::Uncommitted, |d| {
                                d.child(
                                    div()
                                        .flex_1()
                                        .text_size(px(chrome::scale::CAPTION))
                                        .text_color(rgb(p.muted))
                                        .child("Codex reviews the working tree's uncommitted changes in this conversation."),
                                )
                            })
                            .child(
                                self.primary_button("review-start", "Start review", live)
                                    .debug_selector(|| "review-start".into())
                                    .when(live, |d| {
                                        d.on_click(
                                            cx.listener(|this, _, _, cx| this.start_review(cx)),
                                        )
                                    }),
                            ),
                    ),
            )
        });
        let card =
            card.when_some(sandbox, |d, sandbox| {
                let setting_up = sandbox.setting_up();
                let enabled = live && !sandbox.ready && !setting_up;
                d.child(
                    div()
                        .debug_selector(|| "sandbox-section".into())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(projects::section_label("Windows sandbox", p))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(chrome::notice_line(
                                    sandbox.notice().unwrap_or_else(|| {
                                        "Codex's Windows sandbox is ready.".into()
                                    }),
                                    if sandbox.ready {
                                        chrome::Tone::Success
                                    } else if setting_up {
                                        chrome::Tone::Loading
                                    } else {
                                        chrome::Tone::Warning
                                    },
                                    p,
                                )))
                                .when(!sandbox.ready, |d| {
                                    d.child(
                                        self.button("sandbox-setup", "Set up sandbox", enabled)
                                            .debug_selector(|| "sandbox-setup".into())
                                            .when(enabled, |d| {
                                                d.on_click(cx.listener(|this, _, _, cx| {
                                                    this.act(Action::SandboxSetup, cx)
                                                }))
                                            }),
                                    )
                                }),
                        ),
                )
            });
        Some(card)
    }

    fn save_goal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let objective = self.tools.goal_objective.read(cx).value().trim().to_owned();
        let budget = self.tools.goal_budget.read(cx).value().to_string();
        if objective.is_empty() {
            self.extras.notice = "Describe the goal first.".into();
            cx.notify();
            return;
        }
        let token_budget = match wks_native::codex_extras::parse_budget(&budget) {
            Ok(budget) => budget,
            Err(why) => {
                self.extras.notice = why.into();
                cx.notify();
                return;
            }
        };
        for input in [&self.tools.goal_objective, &self.tools.goal_budget] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.act(
            Action::Goal(GoalChange::Set {
                objective,
                token_budget,
            }),
            cx,
        );
    }

    fn start_review(&mut self, cx: &mut Context<Self>) {
        let text = self.tools.review_text.read(cx).value().to_string();
        let target = self.tools.review_kind.target(&text);
        if let Err(why) = target.wire() {
            self.extras.notice = why.into();
            cx.notify();
            return;
        }
        self.act(Action::Review(target), cx);
    }

    /// Small status chips for the title capsule, each shown only when the
    /// session reports its fact: compacting, Codex plan mode, the goal, a
    /// running hook, failed MCP servers and fast mode.
    pub(super) fn status_chips(
        &self,
        session: &Session,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = self.appearance.palette();
        let chip = |id: &'static str, text: String, color: u32| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_shrink_0()
                .max_w(px(180.))
                .truncate()
                .px_2()
                .py(px(3.))
                .rounded_full()
                .bg(rgb(p.selected))
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(color))
                .child(text)
        };
        let mut chips = Vec::new();
        if session.compacting {
            chips.push(
                chip("title-compacting", "Compacting…".into(), p.accent)
                    .tooltip(|window, cx| {
                        Tooltip::new("Summarising the conversation to free context")
                            .build(window, cx)
                    })
                    .into_any_element(),
            );
        }
        if narrow {
            return chips;
        }
        if session.in_plan_mode() {
            chips.push(
                chip("title-plan", "Plan".into(), p.accent)
                    .tooltip(|window, cx| {
                        Tooltip::new("Codex plans before it acts in its next turns")
                            .build(window, cx)
                    })
                    .into_any_element(),
            );
        }
        if let Some(goal) = &session.goal {
            let tip = SharedString::from(format!("Goal: {}", goal.objective));
            chips.push(
                chip(
                    "title-goal",
                    format!("Goal · {}", goal.status_label()),
                    p.muted,
                )
                .cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(
                    cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Session, window, cx)
                    }),
                )
                .into_any_element(),
            );
        }
        // Running hooks stay out of the title: most finish in a moment, so
        // a chip for each one only flickered the bar.
        if let Some(link) = &session.pull_request {
            let unresolved = link.unresolved.filter(|n| *n > 0);
            let tip = SharedString::from(format!(
                "{} {}{}: {}",
                link.provider_label(),
                link.short(),
                if link.title.is_empty() {
                    String::new()
                } else {
                    format!(" {}", link.title)
                },
                match unresolved {
                    Some(n) => format!("{n} unresolved comment{}", if n == 1 { "" } else { "s" }),
                    None => link.state_label().to_lowercase(),
                }
            ));
            let text = match unresolved {
                Some(n) => format!("{} · {n}", link.short()),
                None => format!("{} · {}", link.short(), link.state_label()),
            };
            chips.push(
                chip("title-pr", text, super::pull_request::state_color(link, p))
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::PullRequest, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        let failed = session.status.failed_servers();
        if failed > 0 {
            chips.push(
                chip("title-mcp", format!("MCP · {failed} failed"), p.error)
                    .cursor_pointer()
                    .tooltip(|window, cx| {
                        Tooltip::new("Open Session details to reconnect").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Session, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        if let Some(label) = session.status.fast_label() {
            let on = session.status.fast_mode.as_deref() != Some("off");
            let reason = session.status.fast_mode_disabled_reason.clone();
            let toggle = self.view.connected
                && !session.stopped()
                && !self.view.busy
                && self.view.may_call("claude.setFastMode")
                && (on || reason.is_none());
            let tip = SharedString::from(match &reason {
                Some(reason) => format!("Fast mode unavailable: {reason}"),
                None if on => "Fast mode is on. Click to turn it off".into(),
                None => "Fast mode is off. Click to turn it on".into(),
            });
            chips.push(
                chip(
                    "title-fast",
                    label.into(),
                    if on { p.accent } else { p.muted },
                )
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .when(toggle, |d| {
                    d.cursor_pointer().on_click(
                        cx.listener(move |this, _, _, cx| this.act(Action::SetFastMode(!on), cx)),
                    )
                })
                .into_any_element(),
            );
        }
        chips
    }
}

/// One line of reasoning as a one-line preview: summaries often head each
/// step with a bold or `#` title, which reads as noise when shown inline.
fn thinking_line(line: &str) -> String {
    let line = line.trim().trim_start_matches('#').trim();
    let line = line
        .strip_prefix("**")
        .and_then(|l| l.strip_suffix("**"))
        .unwrap_or(line);
    line.trim().to_owned()
}

#[cfg(test)]
mod thinking_tests {
    use super::thinking_line;

    #[test]
    fn previews_drop_heading_marks() {
        assert_eq!(
            thinking_line("  **Planning the fix**  "),
            "Planning the fix"
        );
        assert_eq!(thinking_line("## Reading the lexer"), "Reading the lexer");
        assert_eq!(
            thinking_line("Look at **this** first"),
            "Look at **this** first"
        );
    }
}
