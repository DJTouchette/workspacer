//! Delivery while the agent works (steer, queue, send now) and the hub's
//! message queue docked above the composer.
use super::super::queue::{Edit, Problem};
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
        uuid: None,
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
        confirmed: None,
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
            Op::Resume,
            Op::Resend {
                uncertain: "q0".into(),
                text: "maybe sent".into()
            },
            Op::Remove("q0".into()),
            Op::Clear,
        ]
    );
}

fn type_edit(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, text: &str) {
    let text = text.to_owned();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.queue.editor.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.focus(window, cx);
            })
        })
    });
    visual.run_until_parked();
}

fn edit_of(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Option<Edit> {
    workspace.read_with(visual, |this, _| this.queue_edit().cloned())
}

/// Show `view` with a receipt for `action` (numbered past any earlier one).
fn answer(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    mut view: View,
    number: u64,
    action: Action,
    error: Option<&str>,
) {
    view.receipt = Some(Receipt {
        number,
        session: view.selected.clone().unwrap(),
        action,
        error: error.map(str::to_owned),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

fn with_entry(view: View, ix: usize, text: &str) -> View {
    let mut view = view;
    Arc::make_mut(&mut view.sessions)[0].queue.entries[ix].text = text.into();
    view
}

/// An untouched edit sends nothing; a real one saves against the text it
/// started from (exactly, no trimming) and closes only once accepted.
#[gpui::test]
fn queue_edits_save_against_their_start_text_and_close_only_when_accepted(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    let base = with_entry(queued(), 0, "first\n");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(base.clone()), window, cx)
        })
    });
    visual.run_until_parked();
    click(&mut visual, "queue-entry-0-edit");
    bounds_of(&mut visual, "queue-editor");
    visual.simulate_keystrokes("enter");
    assert!(actions(&mut commands).is_empty(), "untouched: nothing sent");
    assert!(edit_of(&workspace, &visual).is_none());
    click(&mut visual, "queue-entry-0-edit");
    type_edit(&workspace, &mut visual, "first, edited\n");
    visual.simulate_keystrokes("enter");
    let sent = actions(&mut commands);
    assert!(matches!(
        &sent[..],
        [Action::Queue(Op::Update { id, text, expected: Some(start) })]
            if id == "q1" && text == "first, edited\n" && start == "first\n"
    ));
    let edit = edit_of(&workspace, &visual).expect("open until the hub answers");
    assert!(edit.pending.is_some());
    answer(&workspace, &mut visual, base, 1, sent[0].clone(), None);
    assert!(edit_of(&workspace, &visual).is_none(), "accepted: closed");
    click(&mut visual, "queue-entry-1-edit");
    visual.simulate_keystrokes("escape");
    assert!(edit_of(&workspace, &visual).is_none());
    assert!(
        actions(&mut commands).is_empty(),
        "a cancelled edit sends nothing"
    );
}

/// Another client changed the entry: saving mine is refused, my text stays
/// with theirs shown, and I choose Overwrite (against their text) or
/// Discard mine.
#[gpui::test]
fn a_conflicting_edit_keeps_mine_and_offers_overwrite_or_discard(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(queued()), window, cx)
        })
    });
    visual.run_until_parked();
    click(&mut visual, "queue-entry-0-edit");
    // Theirs lands while I edit; my edit still starts from "first".
    let theirs = with_entry(queued(), 0, "theirs");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(theirs.clone()), window, cx)
        })
    });
    type_edit(&workspace, &mut visual, "mine");
    visual.simulate_keystrokes("enter");
    let sent = actions(&mut commands);
    assert!(matches!(
        &sent[..],
        [Action::Queue(Op::Update { expected: Some(start), .. })] if start == "first"
    ));
    answer(
        &workspace,
        &mut visual,
        theirs.clone(),
        1,
        sent[0].clone(),
        Some("Queue change failed: queued message changed elsewhere"),
    );
    let edit = edit_of(&workspace, &visual).expect("kept");
    assert_eq!(edit.problem, Some(Problem::Conflict("theirs".into())));
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.queue.editor.read(cx).value().as_ref(), "mine")
    });
    bounds_of(&mut visual, "queue-edit-conflict");
    click(&mut visual, "queue-edit-overwrite");
    let sent = actions(&mut commands);
    assert!(matches!(
        &sent[..],
        [Action::Queue(Op::Update { text, expected: Some(start), .. })]
            if text == "mine" && start == "theirs"
    ));
    answer(
        &workspace,
        &mut visual,
        theirs.clone(),
        2,
        sent[0].clone(),
        None,
    );
    assert!(edit_of(&workspace, &visual).is_none());
    // Discard mine ends a conflicting edit without sending anything.
    click(&mut visual, "queue-entry-0-edit");
    type_edit(&workspace, &mut visual, "mine again");
    visual.simulate_keystrokes("enter");
    let sent = actions(&mut commands);
    answer(
        &workspace,
        &mut visual,
        with_entry(queued(), 0, "newest"),
        3,
        sent[0].clone(),
        Some("queued message changed elsewhere"),
    );
    click(&mut visual, "queue-edit-discard");
    assert!(edit_of(&workspace, &visual).is_none());
    assert!(actions(&mut commands).is_empty());
}

/// The entry left the queue mid-edit (or the save was refused): the text
/// stays, with Queue as new and Copy to composer; nothing is lost.
#[gpui::test]
fn a_refused_or_vanished_edit_keeps_the_text_to_requeue_or_copy(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(queued()), window, cx)
        })
    });
    visual.run_until_parked();
    // Refused for another reason, entry unchanged: the reason, text kept.
    click(&mut visual, "queue-entry-0-edit");
    type_edit(&workspace, &mut visual, "too long");
    visual.simulate_keystrokes("enter");
    let sent = actions(&mut commands);
    answer(
        &workspace,
        &mut visual,
        queued(),
        1,
        sent[0].clone(),
        Some("queued messages for this session would exceed 262144 bytes"),
    );
    assert!(matches!(
        edit_of(&workspace, &visual).unwrap().problem,
        Some(Problem::Refused(_))
    ));
    bounds_of(&mut visual, "queue-edit-refused");
    click(&mut visual, "queue-edit-discard");
    // The hub sends the entry while it is being edited.
    click(&mut visual, "queue-entry-1-edit");
    type_edit(&workspace, &mut visual, "my rewrite");
    let mut gone = queued();
    Arc::make_mut(&mut gone.sessions)[0].queue.entries.pop();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(gone.clone()), window, cx)
        })
    });
    visual.run_until_parked();
    assert_eq!(
        edit_of(&workspace, &visual).unwrap().problem,
        Some(Problem::Gone)
    );
    bounds_of(&mut visual, "queue-edit-gone");
    bounds_of(&mut visual, "queue-editor");
    // Survives later updates (the old notice did not).
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(gone.clone()), window, cx)
        })
    });
    assert!(edit_of(&workspace, &visual).is_some());
    click(&mut visual, "queue-edit-requeue");
    let sent = actions(&mut commands);
    assert!(matches!(&sent[..], [Action::Queue(Op::Add(text))] if text == "my rewrite"));
    answer(
        &workspace,
        &mut visual,
        gone.clone(),
        2,
        sent[0].clone(),
        Some("Could not queue the message: message queue is full (50 entries)"),
    );
    let edit = edit_of(&workspace, &visual).expect("a refused re-queue keeps it");
    assert_eq!(edit.problem, Some(Problem::Gone));
    click(&mut visual, "queue-edit-copy");
    assert!(edit_of(&workspace, &visual).is_none());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "my rewrite")
    });
}

/// An edit belongs to its session: switching away and back keeps it, and an
/// entry that leaves the queue meanwhile is noticed there.
#[gpui::test]
fn an_edit_survives_switching_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(queued()), window, cx)
        })
    });
    visual.run_until_parked();
    click(&mut visual, "queue-entry-0-edit");
    type_edit(&workspace, &mut visual, "half written");
    let mut away = queued();
    away.selected = Some("b".into());
    Arc::make_mut(&mut away.sessions)[0].queue.entries.remove(0);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(away), window, cx);
            assert!(this.queue_edit().is_none(), "b has no edit");
            assert_eq!(this.queue.edits["a"].problem, Some(Problem::Gone));
        })
    });
    let mut back = queued();
    Arc::make_mut(&mut back.sessions)[0].queue.entries.remove(0);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(back), window, cx);
            assert_eq!(this.queue.editor.read(cx).value().as_ref(), "half written");
            assert!(this.queue_edit().is_some());
        })
    });
}

/// Tab during an IME composition belongs to the input method.
#[gpui::test]
fn tab_never_queues_mid_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(working()), window, cx)
        })
    });
    draft(&workspace, &mut visual, "draft ");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "に", None, window, cx)
            })
        })
    });
    visual.simulate_keystrokes("tab");
    assert!(actions(&mut commands).is_empty());
}

/// A stopped session's queue can still be tidied (remove, clear, dismiss,
/// resume); adding, editing and sending stay off.
#[gpui::test]
fn a_stopped_sessions_queue_can_still_be_tidied(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    let mut stopped = queued();
    Arc::make_mut(&mut stopped.sessions)[0].state = "stopped".into();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(stopped), window, cx)
        })
    });
    visual.run_until_parked();
    click(&mut visual, "queue-entry-0-send");
    click(&mut visual, "queue-entry-0-edit");
    click(&mut visual, "queue-uncertain-resend");
    assert!(
        actions(&mut commands).is_empty(),
        "nothing new on a stopped session"
    );
    click(&mut visual, "queue-entry-0-remove");
    click(&mut visual, "queue-entry-1-up");
    click(&mut visual, "queue-uncertain-dismiss");
    click(&mut visual, "queue-resume");
    click(&mut visual, "queue-clear");
    assert_eq!(actions(&mut commands).len(), 5);
}

/// Send again says it queued again, and a timed-out one says its outcome
/// is unknown instead of claiming a failure.
#[gpui::test]
fn resend_wording_is_consistent(cx: &mut TestAppContext) {
    let resend = Op::Resend {
        uncertain: "q0".into(),
        text: "maybe sent".into(),
    };
    assert_eq!(resend.success(), "Queued again");
    let unknown =
        resend.failure("Request timed out; outcome unknown. Check the session before retrying");
    assert!(unknown.contains("outcome is unknown"), "{unknown}");
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut next = queued();
    next.notice = unknown.clone();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.title_notice(), unknown);
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
