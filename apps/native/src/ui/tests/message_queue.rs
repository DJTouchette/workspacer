//! Delivery while the agent works (steer, queue, send now) and the hub's
//! message queue docked above the composer.
use super::*;
use wks_native::controller::PendingMessage;
use wks_native::queue::{Entry, Op, Queue, UNCERTAIN_PREVIEW_CHARS, Uncertain};

/// Session "a" mid-turn.
fn working() -> View {
    let mut next = state("a");
    Arc::make_mut(&mut next.sessions)[0].state = "responding".into();
    next
}

fn draft(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, text: &str) {
    let text = text.to_owned();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.focus(window, cx);
            })
        })
    });
    visual.run_until_parked();
}

fn actions(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Action> {
    effects(commands)
        .into_iter()
        .filter_map(|command| match command {
            Command::Act { session, action } => {
                assert_eq!(session, "a");
                Some(action)
            }
            _ => None,
        })
        .collect()
}

/// Enter steers the running turn, Tab queues the draft, Ctrl/Cmd+Enter sends
/// it now; idle, every key is an ordinary send and Tab keeps its job. With
/// Enter as a newline, Ctrl+Enter steers and Ctrl+Shift+Enter sends now.
#[gpui::test]
fn the_send_keys_steer_queue_or_interrupt_while_the_agent_works(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.enter_sends = true;
            this.update_view(Arc::new(working()), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("composer-queue").is_some());
    assert!(visual.debug_bounds("composer-send-now").is_some());
    draft(&workspace, &mut visual, "steer this");
    visual.simulate_keystrokes("enter");
    draft(&workspace, &mut visual, "after the turn");
    visual.simulate_keystrokes("tab");
    draft(&workspace, &mut visual, "look now");
    visual.simulate_keystrokes("ctrl-enter");
    let sent = actions(&mut commands);
    assert!(
        matches!(&sent[..], [
            Action::Send(steer),
            Action::Queue(Op::Add(queued)),
            Action::SendNow(now),
        ] if steer == "steer this" && queued == "after the turn" && now == "look now"),
        "{sent:?}"
    );
    // An empty composer's Tab queues nothing.
    draft(&workspace, &mut visual, "");
    visual.simulate_keystrokes("tab");
    assert!(actions(&mut commands).is_empty());
    // The buttons do the same as the keys.
    draft(&workspace, &mut visual, "by button");
    click(&mut visual, "composer-queue");
    click(&mut visual, "composer-send-now");
    assert!(matches!(
        &actions(&mut commands)[..],
        [Action::Queue(Op::Add(a)), Action::SendNow(b)] if a == "by button" && b == "by button"
    ));
    // Idle: no queue or send-now controls; Ctrl+Enter is an ordinary send
    // and Tab stays in the draft.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    // (Debug bounds outlive the elements that drew them; ask the state.)
    workspace.read_with(&visual, |this, _| assert!(!this.composer_busy()));
    draft(&workspace, &mut visual, "idle");
    visual.simulate_keystrokes("tab");
    assert!(actions(&mut commands).is_empty(), "idle Tab never queues");
    visual.simulate_keystrokes("ctrl-enter");
    assert!(matches!(&actions(&mut commands)[..], [Action::Send(_)]));
    // Enter as newline: Ctrl+Enter steers, Ctrl+Shift+Enter sends now.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.enter_sends = false;
            this.update_view(Arc::new(working()), window, cx)
        })
    });
    draft(&workspace, &mut visual, "steer");
    visual.simulate_keystrokes("ctrl-enter");
    draft(&workspace, &mut visual, "now");
    visual.simulate_keystrokes("ctrl-shift-enter");
    assert!(matches!(
        &actions(&mut commands)[..],
        [Action::Send(a), Action::SendNow(b)] if a == "steer" && b == "now"
    ));
}

/// A steered message shows as the user's message after the transcript's
/// last row, saying it was sent; only the daemon's reply makes it "waiting".
#[gpui::test]
fn steered_messages_read_as_sent_not_queued(cx: &mut TestAppContext) {
    let pending = |accepted, steered, now, queued| PendingMessage {
        number: 1,
        session: "a".into(),
        text: "steer".into(),
        steered,
        now,
        queued,
        accepted,
        before: None,
    };
    use super::super::transcript::pending_status;
    assert_eq!(
        pending_status(&pending(false, true, false, false)),
        "Sending…"
    );
    assert_eq!(
        pending_status(&pending(true, true, false, false)),
        "Sent · the agent reads it at its next step"
    );
    assert_eq!(
        pending_status(&pending(true, true, true, false)),
        "Sent now · the agent reads it right away"
    );
    assert_eq!(
        pending_status(&pending(true, true, false, true)),
        "Waiting for this turn to end"
    );
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut next = working();
    next.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "assistant_text".into(),
            text: "Working on it".into(),
            ..Default::default()
        }],
    });
    next.pending_messages
        .push(pending(true, true, false, false));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    let bubble = bounds_of(&mut visual, "pending-message");
    let last = bounds_of(&mut visual, "last-transcript-row");
    assert!(
        last.contains(&bubble.center()),
        "the pending message follows the transcript, not the dock"
    );
}

fn queued() -> View {
    let mut next = working();
    Arc::make_mut(&mut next.sessions)[0].queue = Queue {
        entries: vec![
            Entry {
                id: "q1".into(),
                text: "first".into(),
                created_at: Some(1),
            },
            Entry {
                id: "q2".into(),
                text: format!("second\n{}", "long ".repeat(60)),
                created_at: Some(2),
            },
        ],
        paused: true,
        error: Some("delivery refused".into()),
        uncertain: Some(Uncertain {
            id: "q0".into(),
            text: "maybe sent".into(),
            at: Some(0),
        }),
    };
    next
}

#[gpui::test]
fn queue_panel_edits_reorders_removes_and_sends_through_the_hub(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("queue-panel").is_none(),
        "no queue, no panel"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(queued()), window, cx)
        })
    });
    visual.run_until_parked();
    for selector in [
        "queue-panel",
        "queue-paused",
        "queue-resume",
        "queue-uncertain",
        "queue-uncertain-resend",
        "queue-entry-0",
        "queue-entry-1",
        "queue-entry-1-expand",
    ] {
        bounds_of(&mut visual, selector);
    }
    assert!(visual.debug_bounds("queue-uncertain-copy").is_none());
    assert!(
        visual.debug_bounds("queue-entry-0-expand").is_none(),
        "a short entry has nothing to expand"
    );
    let panel = bounds_of(&mut visual, "queue-panel");
    let composer = bounds_of(&mut visual, "chat-composer");
    assert!(
        panel.bottom() <= composer.top(),
        "docked above the composer"
    );
    click(&mut visual, "queue-entry-1-up");
    click(&mut visual, "queue-entry-0-send");
    click(&mut visual, "queue-entry-0-remove");
    click(&mut visual, "queue-resume");
    click(&mut visual, "queue-uncertain-resend");
    click(&mut visual, "queue-uncertain-dismiss");
    click(&mut visual, "queue-clear");
    assert_eq!(
        actions(&mut commands)
            .into_iter()
            .map(|a| match a {
                Action::Queue(op) => op,
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        vec![
            Op::Move {
                id: "q2".into(),
                index: 0
            },
            Op::SendNow("q1".into()),
            Op::Remove("q1".into()),
            Op::Move {
                id: "q1".into(),
                index: 0
            },
            Op::Resend {
                uncertain: "q0".into(),
                text: "maybe sent".into()
            },
            Op::Remove("q0".into()),
            Op::Clear,
        ]
    );
    // Inline edit: Enter saves the changed text, Escape cancels.
    click(&mut visual, "queue-entry-0-edit");
    bounds_of(&mut visual, "queue-editor");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.queue.editor.update(cx, |input, cx| {
                input.set_value("first, edited", window, cx);
                input.focus(window, cx);
            })
        })
    });
    visual.simulate_keystrokes("enter");
    assert!(matches!(
        &actions(&mut commands)[..],
        [Action::Queue(Op::Update { id, text })] if id == "q1" && text == "first, edited"
    ));
    workspace.read_with(&visual, |this, _| assert!(this.queue.editing.is_none()));
    click(&mut visual, "queue-entry-1-edit");
    visual.simulate_keystrokes("escape");
    workspace.read_with(&visual, |this, _| assert!(this.queue.editing.is_none()));
    assert!(
        actions(&mut commands).is_empty(),
        "a cancelled edit sends nothing"
    );
    // The hub sends the entry mid-edit: the typed text lands in the composer.
    click(&mut visual, "queue-entry-1-edit");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.queue
                .editor
                .update(cx, |input, cx| input.set_value("my rewrite", window, cx));
            let mut next = queued();
            Arc::make_mut(&mut next.sessions)[0].queue.entries.pop();
            this.update_view(Arc::new(next), window, cx);
            assert!(this.queue.editing.is_none());
            assert_eq!(this.composer.read(cx).value().as_ref(), "my rewrite");
        })
    });
}

/// A preview the hub may have cut is never resent: it goes to the composer
/// for the user to complete, and Dismiss is separate.
#[gpui::test]
fn a_cut_uncertain_preview_is_copied_never_resent(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut next = queued();
    let queue = &mut Arc::make_mut(&mut next.sessions)[0].queue;
    queue.entries.clear();
    queue.uncertain.as_mut().unwrap().text = "x".repeat(UNCERTAIN_PREVIEW_CHARS);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("queue-uncertain-resend").is_none());
    assert!(
        visual.debug_bounds("queue-resume").is_none(),
        "nothing queued to resume"
    );
    click(&mut visual, "queue-uncertain-copy");
    assert!(actions(&mut commands).is_empty());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.composer.read(cx).value().as_ref(),
            "x".repeat(UNCERTAIN_PREVIEW_CHARS)
        )
    });
}

/// A refused live switch is a visible notice naming what was refused; the
/// title chip keeps the hub's recorded effort.
#[gpui::test]
fn a_refused_effort_switch_is_a_notice_and_the_chip_keeps_its_value(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut next = state("a");
    Arc::make_mut(&mut next.sessions)[0].effort = "medium".into();
    next.notice = "Effort change refused: the CLI rejected the effort switch".into();
    next.receipt = Some(Receipt {
        number: 1,
        session: "a".into(),
        action: Action::SetEffort("high".into()),
        error: Some(next.notice.clone()),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.model_base = Some(("opus".into(), None, "medium".into()));
            this.update_view(Arc::new(next), window, cx);
            assert!(this.title_notice().starts_with("Effort change refused"));
            assert_eq!(
                chrome::notice_tone(&this.title_notice()),
                chrome::Tone::Error
            );
            assert_eq!(this.selected_session().unwrap().effort, "medium");
            assert_eq!(
                this.extras.model_base.as_ref().unwrap().2,
                "medium",
                "the refused effort is not the new baseline"
            );
        })
    });
}
