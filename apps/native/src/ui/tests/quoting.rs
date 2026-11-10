//! Quoting the chat: selected text becomes a quote the next message carries.
use super::*;

fn show_reply(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, text: &str) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: text.into(),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

/// Drag across columns `from..to` of the reply's first line.
fn select(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, from: f32, to: f32) {
    let bounds = bounds_of(visual, "markdown-inline-live:a:0-0");
    // The test text system is monospaced: every glyph is 0.6em wide.
    let font = workspace.read_with(visual, |this, _| this.settings.text_size as f32);
    let at = |col: f32| {
        bounds.origin + gpui::point(px((col + 0.1) * font * 0.6), px(font * 1.6 / 2.))
    };
    visual.simulate_mouse_down(at(from), gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_move(
        at(to),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    visual.simulate_mouse_up(at(to), gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
}

/// Selecting text in a reply offers Quote beside it; pressing it puts the
/// passage above the draft, focuses the composer, and the card can be removed.
#[gpui::test]
fn selected_chat_text_can_be_quoted_into_the_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    show_reply(&workspace, &mut visual, "alpha beta gamma delta");
    assert!(workspace.read_with(&visual, |this, _| this.extras.quote_pick.is_none()));
    select(&workspace, &mut visual, 6., 16.);
    workspace.read_with(&visual, |this, _| {
        let pick = this.extras.quote_pick.as_ref().expect("a selection to quote");
        assert_eq!(pick.text, "beta gamma");
        assert_eq!(pick.owner, "a");
    });
    let quote = bounds_of(&mut visual, "quote-selection");
    visual.simulate_click(quote.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.quotes["a"], ["beta gamma"]);
        assert!(this.extras.quote_pick.is_none(), "the action is spent");
    });
    assert!(visual.update(|window, cx| {
        workspace
            .read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    let composer = bounds_of(&mut visual, "chat-composer");
    let card = bounds_of(&mut visual, "draft-quote-0");
    assert!(composer.contains(&card.center()), "the quote sits in the composer");
    click(&mut visual, "remove-quote-0");
    workspace.read_with(&visual, |this, _| assert!(this.extras.quotes["a"].is_empty()));
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|command| matches!(command, Command::Act { .. })),
        "quoting sends nothing"
    );
}

/// `r` quotes the selection from the keyboard; a press elsewhere withdraws
/// the offer, and the same passage is not quoted twice.
#[gpui::test]
fn the_selection_is_quoted_by_key_and_withdrawn_by_a_press_elsewhere(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show_reply(&workspace, &mut visual, "alpha beta gamma delta");
    for _ in 0..2 {
        select(&workspace, &mut visual, 6., 16.);
        visual.simulate_keystrokes("r");
        visual.run_until_parked();
    }
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.quotes["a"], ["beta gamma"]);
    });
    select(&workspace, &mut visual, 0., 5.);
    assert!(workspace.read_with(&visual, |this, _| this.extras.quote_pick.is_some()));
    let composer = bounds_of(&mut visual, "chat-composer");
    visual.simulate_click(composer.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.quote_pick.is_none());
        assert_eq!(this.extras.quotes["a"].len(), 1);
    });
}

/// The message carries its quotes ahead of the reply as blockquotes, the
/// quotes retire once the hub accepts it, and the sent message draws them
/// as quotes with only its own words left literal.
#[gpui::test]
fn a_reply_carries_its_quotes_and_shows_them_once_sent(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    show_reply(&workspace, &mut visual, "alpha beta gamma delta");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .quotes
                .insert("a".into(), vec!["beta gamma".into(), "two\nlines".into()]);
            this.composer
                .update(cx, |input, cx| input.set_value("Why these?", window, cx));
            this.composer.update(cx, |input, cx| input.focus(window, cx));
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    let sent = effects(&mut commands)
        .into_iter()
        .find_map(|command| match command {
            Command::Act {
                action: Action::Send(text),
                ..
            } => Some(text),
            _ => None,
        })
        .expect("the reply is sent");
    assert_eq!(sent, "> beta gamma\n\n> two\n> lines\n\nWhy these?");
    // Accepted: the draft and the quotes it carried are spent, and the
    // message is in the transcript.
    let mut accepted = state("a");
    accepted.receipt = Some(Receipt {
        number: 1,
        session: "a".into(),
        action: Action::Send(sent.clone()),
        error: None,
    });
    accepted.transcript.snapshot(ConversationSnapshot {
        seq: 2,
        first_seq: 1,
        items: vec![
            Item {
                kind: "assistant_text".into(),
                text: "alpha beta gamma delta".into(),
                ..Default::default()
            },
            Item {
                kind: "user_message".into(),
                text: sent,
                ..Default::default()
            },
        ],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(accepted), window, cx);
            assert!(this.extras.quotes["a"].is_empty());
            assert_eq!(this.composer.read(cx).value().as_ref(), "");
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("message-quotes").is_some());
}

/// A refused send keeps the quotes with the draft for another try.
#[gpui::test]
fn a_refused_send_keeps_its_quotes(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    show_reply(&workspace, &mut visual, "alpha beta gamma delta");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.quotes.insert("a".into(), vec!["beta".into()]);
            this.composer.update(cx, |input, cx| input.focus(window, cx));
        })
    });
    // A quote alone is a message.
    visual.simulate_keystrokes("ctrl-enter");
    let sent = effects(&mut commands)
        .into_iter()
        .find_map(|command| match command {
            Command::Act {
                action: Action::Send(text),
                ..
            } => Some(text),
            _ => None,
        })
        .expect("a quote alone is sent");
    assert_eq!(sent, "> beta\n\n");
    let mut refused = state("a");
    refused.receipt = Some(Receipt {
        number: 1,
        session: "a".into(),
        action: Action::Send(sent),
        error: Some("Disconnected; outcome unknown".into()),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(refused), window, cx);
            assert_eq!(this.extras.quotes["a"], ["beta"]);
        })
    });
}
