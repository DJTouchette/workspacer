//! Question sets: picking, typing and sending answers.
use super::*;

#[gpui::test]
fn question_choices_preserve_punctuation_and_custom_answers(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| workspace.update(cx, |this, cx| {
        let mut next = state("a");
        Arc::make_mut(&mut next.sessions)[0].questions = Some(serde_json::json!([{"question":"Which?", "options":[{"label":"Yes, please"},{"label":"No"}],"multiSelect":true}]));
        this.update_view(Arc::new(next), window, cx);
        this.extras.selected_options[0].insert(0);
        assert_eq!(this.question_answers(cx), vec!["Yes, please"]);
        this.extras.selected_options[0].remove(&0);
        assert_eq!(this.question_answers(cx), vec![""]);
        this.extras.answers[0].update(cx, |input, cx| input.set_value("Another approach", window, cx));
        assert_eq!(this.question_answers(cx), vec!["Another approach"]);
    }));
}

/// Every action sent since the last call (all for session "a").
fn sent_actions(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Action> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Act { session, action } => {
                assert_eq!(session, "a");
                Some(action)
            }
            _ => None,
        })
        .collect()
}

fn click_selector(visual: &mut VisualTestContext, selector: &str) {
    let bounds = bounds_of(visual, selector);
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();
}

#[gpui::test]
fn question_picker_answers_by_click_and_keyboard_without_touching_the_draft(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    // Tall enough that every row is on screen; small windows are the
    // geometry test's job.
    visual.simulate_resize(size(px(1400.), px(1400.)));
    let selected = |visual: &mut VisualTestContext, ix: usize| {
        workspace.read_with(visual, |this, _| {
            this.extras.selected_options[ix]
                .iter()
                .copied()
                .collect::<Vec<_>>()
        })
    };
    // Busy: rows are inert.
    let mut busy = question_state();
    busy.busy = true;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(busy), window, cx))
    });
    visual.run_until_parked();
    click_selector(&mut visual, "question-0-option-1");
    assert!(
        selected(&mut visual, 0).is_empty(),
        "busy picker ignores clicks"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(question_state()), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("KEEP_DRAFT", window, cx));
        })
    });
    visual.run_until_parked();
    // Nothing answered: neither Send nor Ctrl+Enter sends anything (in
    // particular not the composer draft), and Enter in a typed answer
    // moves on to the next unanswered question.
    click_selector(&mut visual, "submit-answers");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.answers[2].update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    visual.simulate_keystrokes("enter");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            assert!(
                this.extras.answers[0]
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window),
                "Enter moves to the first unanswered question"
            )
        })
    });
    assert!(sent_actions(&mut commands).is_empty());
    // Single choice: a click chooses, another click replaces it.
    click_selector(&mut visual, "question-0-option-1");
    assert_eq!(selected(&mut visual, 0), vec![1]);
    click_selector(&mut visual, "question-0-option-0");
    assert_eq!(selected(&mut visual, 0), vec![0]);
    assert!(visual.debug_bounds("question-0-answered").is_some());
    assert!(
        sent_actions(&mut commands).is_empty(),
        "choosing never sends"
    );
    // Multiple choice from the keyboard: Space toggles the focused row,
    // digits pick within the same question, Down moves to the next row.
    visual.update(|window, cx| {
        workspace.update(cx, |this, _| window.focus(&this.extras.option_focus[1][0]))
    });
    visual.run_until_parked();
    press(&mut visual, "space");
    assert_eq!(selected(&mut visual, 1), vec![0]);
    press(&mut visual, "3");
    assert_eq!(selected(&mut visual, 1), vec![0, 2]);
    press(&mut visual, "space");
    assert_eq!(selected(&mut visual, 1), vec![2]);
    press(&mut visual, "down");
    visual.update(|window, cx| {
        workspace.update(cx, |this, _| {
            assert!(this.extras.option_focus[1][1].is_focused(window))
        })
    });
    assert_eq!(
        selected(&mut visual, 0),
        vec![0],
        "other questions keep their choice"
    );
    // Typed answer, then Ctrl+Enter inside the picker sends exactly the
    // literal labels and text once, and leaves the draft alone.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.answers[2].update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.simulate_input("Ship it, 2");
    // Offline, the keyboard is held to the same gate as the button.
    let mut offline = question_state();
    offline.connected = false;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(offline), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    assert!(
        sent_actions(&mut commands).is_empty(),
        "nothing sent while offline"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(question_state()), window, cx);
            this.extras.answers[2].update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    let expected = vec![
        "Online backfill".to_owned(),
        "rustfmt --check".to_owned(),
        "Ship it, 2".to_owned(),
    ];
    visual.simulate_keystrokes("ctrl-enter"); // before any busy/receipt frame
    let sent = sent_actions(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Action::Answers(answers)] if *answers == expected),
        "{sent:?}"
    );
    // Wire shape (answerKinds all "text"): tests/protocol.rs.
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "KEEP_DRAFT")
    });
    // Accepted: read-only (no second send) until the user edits or the
    // question set changes.
    let mut accepted = question_state();
    accepted.receipt = Some(wks_native::controller::Receipt {
        number: 1,
        session: "a".into(),
        action: Action::Answers(expected),
        error: None,
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(accepted), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(workspace.read_with(&visual, |this, _| this.extras.answers_sent));
    click_selector(&mut visual, "submit-answers");
    click_selector(&mut visual, "question-0-option-2");
    visual.simulate_keystrokes("ctrl-enter");
    assert!(
        sent_actions(&mut commands).is_empty(),
        "sent answers are not resent"
    );
    assert_eq!(selected(&mut visual, 0), vec![0]);
    click_selector(&mut visual, "edit-answers");
    assert!(!workspace.read_with(&visual, |this, _| this.extras.answers_sent));
    // Resolution, then a new set: the picker starts fresh.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.update_view(Arc::new(question_state()), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(selected(&mut visual, 0).is_empty() && selected(&mut visual, 1).is_empty());
    workspace.read_with(&visual, |this, cx| {
        assert!(!this.extras.answers_sent);
        assert!(this.extras.answers[2].read(cx).value().is_empty());
        assert_eq!(this.composer.read(cx).value().as_ref(), "KEEP_DRAFT");
    });
}

#[gpui::test]
fn old_answer_receipt_does_not_lock_a_new_question_set(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(question_state()), window, cx);
            for input in &this.extras.answers {
                input.update(cx, |input, cx| {
                    input.set_value("literal answer", window, cx)
                });
            }
            this.composer
                .update(cx, |input, cx| input.set_value("KEEP_DRAFT", window, cx));
            this.submit_answers(cx);
            this.submit_answers(cx);
        })
    });
    let sent = sent_actions(&mut commands);
    assert_eq!(sent.len(), 1, "an in-flight answer is submitted once");
    let mut next = question_state();
    Arc::make_mut(&mut next.sessions)[0]
        .questions
        .as_mut()
        .unwrap()[0]["question"] = serde_json::json!("A different request");
    next.receipt = Some(wks_native::controller::Receipt {
        number: 1,
        session: "a".into(),
        action: sent[0].clone(),
        error: None,
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(
            !this.extras.answers_sent,
            "an old receipt must not acknowledge new questions"
        );
        assert!(this.extras.answer_submission.is_none());
        assert!(
            this.extras
                .answers
                .iter()
                .all(|input| input.read(cx).value().is_empty())
        );
        assert_eq!(this.composer.read(cx).value().as_ref(), "KEEP_DRAFT");
    });
}

#[gpui::test]
fn single_question_needs_an_explicit_send_and_keeps_typed_numbers_literal(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut next = state("a");
    Arc::make_mut(&mut next.sessions)[0].questions = Some(serde_json::json!([
        {"question":"Ship now?","options":[{"label":"Yes, please"},{"label":"No"}]}
    ]));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    click_selector(&mut visual, "question-0-option-0");
    assert!(
        sent_actions(&mut commands).is_empty(),
        "a choice is not a send"
    );
    // Typing replaces the choice; the typed number goes out as text.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.answers[0].update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.simulate_input("2");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.question_answers(cx),
            vec!["2"],
            "typing replaces the choice"
        )
    });
    visual.simulate_keystrokes("enter");
    let sent = sent_actions(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Action::Answers(answers)] if answers == &["2".to_owned()]),
        "{sent:?}"
    );
}

#[gpui::test]
fn question_picker_keeps_send_reachable_in_every_theme_and_window(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(question_state()), window, cx)
        })
    });
    for (width, height) in [(720., 480.), (760., 520.), (1000., 700.), (1400., 900.)] {
        visual.simulate_resize(size(px(width), px(height)));
        for appearance in Appearance::ALL {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| this.set_appearance(appearance, window, cx))
            });
            visual.run_until_parked();
            let label = format!("{} {width}x{height}", appearance.label());
            let card = bounds_of(&mut visual, "question-card");
            let list = bounds_of(&mut visual, "question-list");
            let submit = bounds_of(&mut visual, "submit-answers");
            let composer = bounds_of(&mut visual, "chat-composer");
            let first = bounds_of(&mut visual, "question-0-text");
            assert!(
                card.top() >= px(0.) && card.bottom() <= composer.top(),
                "{label}: {card:?} over {composer:?}"
            );
            for (name, inner) in [("list", list), ("send", submit)] {
                assert!(
                    inner.left() >= card.left()
                        && inner.right() <= card.right()
                        && inner.top() >= card.top()
                        && inner.bottom() <= card.bottom(),
                    "{label}: {name} {inner:?} outside {card:?}"
                );
            }
            assert!(
                !submit.intersects(&list),
                "{label}: Send never scrolls with the list"
            );
            assert!(
                list.size.height >= px(56.),
                "{label}: list too short {list:?}"
            );
            assert!(
                first.top() >= list.top() && first.top() < list.bottom(),
                "{label}: first question hidden"
            );
            if height >= 620. {
                assert!(
                    submit.top() >= list.bottom(),
                    "{label}: Send sits below the list"
                );
            }
        }
    }
    // Moving focus to a row outside the list scrolls it into view.
    visual.simulate_resize(size(px(760.), px(520.)));
    visual.run_until_parked();
    let list = bounds_of(&mut visual, "question-list");
    assert!(
        bounds_of(&mut visual, "question-2-answer").top() >= list.bottom(),
        "starts out of view"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.answers[2].update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.run_until_parked();
    visual.run_until_parked();
    let answer = bounds_of(&mut visual, "question-2-answer");
    assert!(
        answer.top() >= list.top() && answer.bottom() <= list.bottom() + px(1.),
        "{answer:?} not scrolled into {list:?}"
    );
}
