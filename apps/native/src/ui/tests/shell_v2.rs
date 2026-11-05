//! Design v2's shell: the attention-first sidebar, `g n`, densities, the
//! rail, the cheat sheet and the model popover.
use super::*;
use wks_native::appearance::Design;
use wks_native::tokens::Tokens;

/// a needs you, b is idle, c ended, d is working, e also needs you and
/// has waited longer than a.
fn attention_state() -> View {
    let session = |id: &str, label: &str, state: &str| Session {
        id: id.into(),
        label: label.into(),
        state: state.into(),
        provider: "claude".into(),
        ..Default::default()
    };
    let mut a = session("a", "Alpha", "input");
    a.approval = Some(serde_json::json!({"tool": "Bash"}));
    a.telemetry.last_activity_ms = Some(2_000);
    let mut e = session("e", "Echo", "input");
    e.questions = Some(serde_json::json!([]));
    e.telemetry.last_activity_ms = Some(1_000);
    View {
        connected: true,
        selected: Some("b".into()),
        sessions: Arc::new(vec![
            a,
            session("b", "Beta", "input"),
            session("c", "Charlie", "ended"),
            session("d", "Delta", "working"),
            e,
        ]),
        ..Default::default()
    }
}

fn open_v2(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    view: View,
    design: Design,
) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(design));
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

fn selected(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<String> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Select(id) => Some(id),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_sidebar_groups_sessions_by_what_needs_you(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    for group in [
        "sidebar-group-needs-you",
        "sidebar-group-working",
        "sidebar-group-idle",
        "sidebar-group-ended",
    ] {
        assert!(visual.debug_bounds(group).is_some(), "{group}");
    }
    // Needs you (longest wait first), then Working, then Idle; Ended folded.
    let order = workspace.read_with(&visual, |this, cx| {
        this.nav_sessions(cx)
            .into_iter()
            .map(|ix| this.view.sessions[ix].id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(order, ["e", "a", "d", "b"]);
    let needs = visual.debug_bounds("sidebar-group-needs-you").unwrap();
    let working = visual.debug_bounds("sidebar-group-working").unwrap();
    let idle = visual.debug_bounds("sidebar-group-idle").unwrap();
    assert!(needs.top() < working.top() && working.top() < idle.top());
    assert!(visual.debug_bounds("sidebar-next-needs-you").is_some());
    // Unfolding Ended shows its session.
    click(&mut visual, "sidebar-group-ended");
    let order = workspace.read_with(&visual, |this, cx| this.nav_sessions(cx).len());
    assert_eq!(order, 5);
    // The wordmark sits beside the mark; the footer is one row; the
    // destinations are one labelled list at the top, above the groups.
    assert!(visual.debug_bounds("sidebar-wordmark").is_some());
    let footer = visual.debug_bounds("sidebar-footer").unwrap();
    assert!(f32::from(footer.size.height) <= Tokens::V2.layout.footer_height);
    let list = visual.debug_bounds("sidebar-destinations").unwrap();
    for item in [
        "new-session-button",
        "nav-fleet",
        "nav-projects",
        "nav-jobs",
        "nav-history",
        "nav-hosts",
        "nav-scratch",
    ] {
        let bounds = visual.debug_bounds(item).unwrap();
        assert!(
            list.contains(&bounds.center()),
            "{item} in the destinations"
        );
        assert!(bounds.bottom() < needs.top(), "{item} above the groups");
    }
    // r5-shell: Settings is the footer's gear.
    let gear = visual.debug_bounds("nav-settings").unwrap();
    assert!(footer.contains(&gear.center()), "the gear in the footer");
}

#[gpui::test]
fn needs_you_is_an_inbox_with_reasons_and_an_all_clear(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    let mut view = attention_state();
    if let Some(a) = Arc::make_mut(&mut view.sessions).get_mut(0) {
        a.approval = Some(serde_json::json!({
            "toolName": "Bash", "toolInput": {"command": "cargo test"}
        }));
        a.pending_request_id = Some("req-a".into());
    }
    open_v2(&workspace, &mut visual, view, Design::V2);
    // Rows 1 (e: a question) and 2 (a: an approval) give their reason and
    // one action each; r4-calm: the amber glyph is their one treatment.
    assert!(visual.debug_bounds("sidebar-reason-1").is_some());
    assert!(visual.debug_bounds("sidebar-reason-2").is_some());
    // r3-shell: the sidebar never allows an unseen command blind; its verb
    // is Review, which opens the session at the request.
    effects(&mut commands);
    let row = visual.debug_bounds("sidebar-session-2").unwrap();
    visual.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    visual.run_until_parked();
    click(&mut visual, "sidebar-inbox-action-2");
    let sent = effects(&mut commands);
    assert!(
        !sent.iter().any(|c| matches!(
            c,
            Command::Act {
                action: Action::Approve(..),
                ..
            }
        )),
        "Review never approves"
    );
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::Select(id) if id == "a")),
        "Review opens the session"
    );
    assert!(visual.debug_bounds("sidebar-all-clear").is_none());
    // With nothing waiting, the group says so.
    let mut calm = attention_state();
    for s in Arc::make_mut(&mut calm.sessions) {
        s.approval = None;
        s.questions = None;
    }
    open_v2(&workspace, &mut visual, calm, Design::V2);
    assert!(visual.debug_bounds("sidebar-all-clear").is_some());
}

#[gpui::test]
fn modal_keys_start_off_under_v2(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    workspace.read_with(&visual, |this, _| assert!(!this.vim()));
    assert!(visual.debug_bounds("sidebar-mode").is_none());
    // `?` still opens the cheat sheet.
    visual.simulate_keystrokes("escape ?");
    visual.run_until_parked();
    assert!(visual.debug_bounds("shortcuts-dialog").is_some());
}

#[gpui::test]
fn the_palette_finds_sessions_and_actions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    effects(&mut commands);
    visual.simulate_keystrokes("ctrl-k");
    visual.run_until_parked();
    assert!(visual.debug_bounds("command-palette").is_some());
    visual.simulate_input("@delta");
    visual.run_until_parked();
    assert!(visual.debug_bounds("command-palette-scope").is_some());
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(selected(&mut commands), ["d"]);
    // (gpui's test `debug_bounds` keeps an element's last bounds after it
    // stops drawing, so closed overlays are checked by state.)
    workspace.read_with(&visual, |this, _| assert!(!this.shell.palette.open));
    // `>` scopes to actions: the Jobs page opens.
    visual.simulate_keystrokes("ctrl-k");
    visual.run_until_parked();
    visual.simulate_input(">jobs");
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Jobs));
    // Escape closes it.
    visual.simulate_keystrokes("ctrl-k");
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.shell.palette.open));
}

#[gpui::test]
fn next_needs_you_walks_the_waiting_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    effects(&mut commands);
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.next_needs_you(window, cx)));
    visual.run_until_parked();
    assert_eq!(selected(&mut commands), ["e"], "longest wait first");
    // The badge in the group header goes to the same place.
    let mut view = attention_state();
    view.selected = Some("e".into());
    open_v2(&workspace, &mut visual, view, Design::V2);
    effects(&mut commands);
    click(&mut visual, "sidebar-next-needs-you");
    assert_eq!(selected(&mut commands), ["a"]);
}

#[gpui::test]
fn v1_keeps_g_n_for_scratch(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V1);
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.next_needs_you(window, cx)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Scratch));
    // And draws none of v2's shell.
    assert!(visual.debug_bounds("sidebar-group-needs-you").is_none());
    assert!(visual.debug_bounds("sidebar-wordmark").is_none());
}

#[gpui::test]
fn g_n_means_needs_you_under_v2_and_scratch_under_v1(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    // Modal keys are a choice under v2.
    workspace.update(&mut visual, |this, cx| {
        this.settings.vim_navigation_v2 = true;
        cx.notify();
    });
    visual.run_until_parked();
    effects(&mut commands);
    visual.simulate_keystrokes("g n");
    visual.run_until_parked();
    assert_eq!(selected(&mut commands), ["e"]);
    visual.simulate_keystrokes("g x");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Scratch));
    visual.simulate_keystrokes("escape ?");
    visual.run_until_parked();
    assert!(visual.debug_bounds("shortcuts-dialog").is_some());

    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V1);
    effects(&mut commands);
    visual.simulate_keystrokes("g n");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Scratch));
    assert!(selected(&mut commands).is_empty());
    visual.simulate_keystrokes("escape ?");
    visual.run_until_parked();
    assert!(visual.debug_bounds("shortcuts-dialog").is_none());
}

#[gpui::test]
fn rows_take_the_chosen_density(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    // Rows: needs-you header, e, a, working header, d, idle header, b.
    // r4-calm (R4.4), r6-surfaces (R6.5): one line by default; a second,
    // faint line only says what the agent asks (in compact too).
    let height = |visual: &mut VisualTestContext, row: &'static str| {
        f32::from(visual.debug_bounds(row).unwrap().size.height) + 2. * Tokens::V2.stroke.hairline
    };
    let shell = Tokens::V2.shell();
    assert_eq!(height(&mut visual, "sidebar-session-1"), shell.row_double);
    assert!(visual.debug_bounds("sidebar-reason-1").is_some());
    assert_eq!(height(&mut visual, "sidebar-session-6"), shell.row_single);
    assert!(visual.debug_bounds("sidebar-project-6").is_none());
    // One needs-you style: no attention frame around the waiting rows.
    assert!(visual.debug_bounds("attention-bar").is_none());
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.sidebar_compact = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    // Compact keeps what the agent asks; everything else is one line.
    assert_eq!(height(&mut visual, "sidebar-session-1"), shell.row_double);
    assert!(visual.debug_bounds("sidebar-reason-1").is_some());
    assert_eq!(
        height(&mut visual, "sidebar-session-6"),
        Tokens::V2.layout.sidebar_row_compact
    );
}

#[gpui::test]
fn the_rail_leads_with_who_needs_you(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.sidebar_collapsed = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    effects(&mut commands);
    // The rail's first tile is the longest-waiting session; only what
    // needs you or works gets a tile (e, a, d), the rest fold into "+n".
    click(&mut visual, "sidebar-session-0");
    assert_eq!(selected(&mut commands), ["e"]);
    assert!(visual.debug_bounds("sidebar-session-2").is_some());
    assert!(visual.debug_bounds("sidebar-session-3").is_none());
    assert!(visual.debug_bounds("rail-more").is_some());
}

#[gpui::test]
fn the_cheat_sheet_opens_and_closes(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    visual.update(|_, cx| workspace.update(cx, |this, cx| this.toggle_shortcuts(cx)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("shortcuts-dialog").is_some());
    // The close button sits inside the dialog (a long description must not
    // push it out of the panel).
    let dialog = visual.debug_bounds("shortcuts-dialog").unwrap();
    let close = visual.debug_bounds("shortcuts-close").unwrap();
    assert!(
        dialog.contains(&close.center()),
        "{close:?} inside {dialog:?}"
    );
    click(&mut visual, "shortcuts-close");
    workspace.read_with(&visual, |this, _| assert!(!this.shell.shortcuts));
}

#[gpui::test]
fn the_model_chip_opens_a_popover_not_a_page(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let mut view = attention_state();
    view.selected = Some("a".into());
    open_v2(&workspace, &mut visual, view, Design::V2);
    // An unknown access mode is not shown, nor offered.
    assert!(visual.debug_bounds("title-access").is_none());
    // R6.3: "a" waits on a decision; the composer is one line under it
    // until it has focus, then its model chip is there.
    assert!(visual.debug_bounds("composer-model").is_none());
    click(&mut visual, "chat-composer");
    click(&mut visual, "composer-model");
    assert!(visual.debug_bounds("model-popover").is_some());
    assert!(visual.debug_bounds("popover-access-unknown").is_none());
    assert!(visual.debug_bounds("popover-model-custom").is_some());
    // r3-shell: the known models are listed though the agent reported
    // none, "Custom id…" last; the access shows its default, Ask first.
    assert!(visual.debug_bounds("popover-model-2").is_some());
    let last_known = visual.debug_bounds("popover-model-2").unwrap();
    let custom = visual.debug_bounds("popover-model-custom").unwrap();
    assert!(custom.top() > last_known.top());
    assert!(
        visual
            .debug_bounds("popover-access-Ask to approve")
            .is_some()
    );
    // r5-shell (R5.4): the popover hangs from the chip that opened it
    // (above it, left edges aligned), decision or not.
    let card = visual.debug_bounds("model-popover").unwrap();
    let chip = visual.debug_bounds("composer-model").unwrap();
    assert!(card.bottom() <= chip.top(), "{card:?} above {chip:?}");
    assert!(
        (f32::from(card.left()) - f32::from(chip.left())).abs() <= 2.,
        "{card:?} aligned with {chip:?}"
    );
    // Each access option says what it allows.
    assert!(visual.debug_bounds("popover-access").is_some());
    // The keys drive it: Down walks the list, Escape closes it.
    visual.simulate_keystrokes("down");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.shell.model_cursor <= 4));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.shell.model_popover));
    // R6.3: closed, the popover may hand the focus back elsewhere; the
    // one-line composer opens again on a click.
    if visual.debug_bounds("composer-model").is_none() {
        click(&mut visual, "chat-composer");
    }
    click(&mut visual, "composer-model");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation);
        assert!(this.shell.model_popover);
    });
    // The full page is one step away.
    click(&mut visual, "model-popover-page");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Model);
        assert!(!this.shell.model_popover);
    });
}

/// r6-integration (R6.4): the popover has no Apply button. A model picked
/// there is sent at once, as an access choice is; the session's own model
/// sends nothing.
#[gpui::test]
fn a_popover_pick_applies_at_once(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let mut view = attention_state();
    view.selected = Some("a".into());
    open_v2(&workspace, &mut visual, view, Design::V2);
    click(&mut visual, "chat-composer");
    click(&mut visual, "composer-model");
    assert!(visual.debug_bounds("model-popover").is_some());
    assert!(visual.debug_bounds("popover-apply-model").is_none());
    assert!(visual.debug_bounds("popover-applies-note").is_some());
    let _ = effects(&mut commands);
    // Pick a listed model that is not the session's own.
    let current = workspace.read_with(&visual, |this, _| {
        this.extras.model_base.clone().unwrap_or_default().0
    });
    let pick = workspace.read_with(&visual, |this, _| {
        this.popover_models()
            .iter()
            .position(|m| m.id != current)
            .unwrap()
    });
    click(
        &mut visual,
        &*Box::leak(format!("popover-model-{pick}").into_boxed_str()),
    );
    let sent: Vec<_> = effects(&mut commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Act { action, .. } => Some(action),
            _ => None,
        })
        .collect();
    assert!(
        matches!(sent.as_slice(), [Action::SetModel { model, .. }] if *model != current),
        "{sent:?}"
    );
}

/// r3-shell: the title is a header strip with the transcript beneath it.
/// r4-calm (R4.4): the session's tools are icon buttons on its right, and
/// History, Details and Model share one "…" menu.
#[gpui::test]
fn the_header_is_a_strip_with_icon_tools_and_one_overflow(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    let mut view = attention_state();
    view.selected = Some("d".into());
    open_v2(&workspace, &mut visual, view, Design::V2);
    let strip = visual.debug_bounds("chat-header").unwrap();
    let height = f32::from(strip.size.height);
    assert!((36. ..=40.).contains(&height), "{height}");
    let bar = visual.debug_bounds("title-bar").unwrap();
    assert!(strip.contains(&bar.center()));
    for tool in [
        "open-changes",
        "open-editor",
        "open-terminal",
        "title-tasks",
        "session-more",
    ] {
        let bounds = visual
            .debug_bounds(tool)
            .unwrap_or_else(|| panic!("{tool}"));
        assert!(strip.contains(&bounds.center()), "{tool} on the strip");
        assert!(
            f32::from(bounds.size.height) >= 28. - 0.5,
            "{tool} hit area"
        );
        assert!(bounds.left() > bar.left(), "{tool} right of the title");
    }
    // r6-surfaces (R6.5): labelled at this width ("Files"), so wider than
    // it is tall.
    let files = visual.debug_bounds("open-editor").unwrap();
    assert!(
        f32::from(files.size.width) > f32::from(files.size.height),
        "{files:?}"
    );
    // Below `shell().toolbar_labels_at` the tools are icon-only squares.
    visual.simulate_resize(size(px(1000.), px(900.)));
    visual.run_until_parked();
    visual.simulate_resize(size(px(1000.), px(900.)));
    visual.run_until_parked();
    let files = visual.debug_bounds("open-editor").unwrap();
    assert!(
        f32::from(files.size.width) <= f32::from(files.size.height) + 0.5,
        "{files:?}"
    );
    visual.simulate_resize(size(px(1600.), px(900.)));
    visual.run_until_parked();
    // History, Details and Model wait in the menu.
    workspace.read_with(&visual, |this, _| assert!(!this.shell.session_menu));
    click(&mut visual, "session-more");
    let menu = visual.debug_bounds("session-more-menu").unwrap();
    for item in ["open-history", "open-session", "open-model"] {
        let bounds = visual
            .debug_bounds(item)
            .unwrap_or_else(|| panic!("{item}"));
        assert!(menu.contains(&bounds.center()), "{item} in the menu");
    }
    click(&mut visual, "open-model");
    workspace.read_with(&visual, |this, _| {
        assert!(this.shell.model_popover);
        assert!(!this.shell.session_menu);
    });
    // No rail; the transcript starts below the header.
    assert!(visual.debug_bounds("tool-rail").is_none());
    workspace.read_with(&visual, |this, _| {
        assert!(this.header_bounds.size.height >= strip.size.height);
        assert!(this.title_crumb_fits());
    });
    // Narrow: the crumb steps aside so the title keeps its room.
    visual.simulate_resize(size(px(900.), px(700.)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.title_crumb_fits()));
    assert!(visual.debug_bounds("session-more").is_some());
}

/// r3-shell: the destinations are a short block, and the footer carries no
/// modal-key label. r5-shell (R5.4): seven labelled 32px NavItem rows in a
/// tight list (New session, Fleet, Projects, Jobs, History, Machines,
/// Scratch), every one with its own glyph.
#[gpui::test]
fn the_destinations_are_a_short_labelled_block(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    let block = visual.debug_bounds("sidebar-destinations").unwrap();
    let rows = Tokens::V2.system().nav_row * 7.;
    assert!(
        f32::from(block.size.height) <= rows + Tokens::V2.space.sm,
        "{block:?}"
    );
    let row = visual.debug_bounds("nav-history").unwrap();
    assert_eq!(f32::from(row.size.height), Tokens::V2.system().nav_row);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.vim_navigation_v2 = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.vim()));
    assert!(visual.debug_bounds("sidebar-mode").is_none());
}

/// r3-shell: "Jump to latest" sits the token gap above the decision card
/// (not the dock's own padding plus the gap), over its scrim.
#[gpui::test]
fn jump_to_latest_anchors_just_above_the_decision(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    let mut view = attention_state();
    view.selected = Some("a".into());
    view.transcript.snapshot(ConversationSnapshot {
        seq: 60,
        first_seq: 1,
        items: (0..60)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nSome content."),
                ..Default::default()
            })
            .collect(),
    });
    open_v2(&workspace, &mut visual, view, Design::V2);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.follow = false;
            this.list.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            cx.notify();
        })
    });
    visual.run_until_parked();
    let jump = visual.debug_bounds("jump-latest").unwrap();
    let card = visual.debug_bounds("approval-card").unwrap();
    assert!(visual.debug_bounds("jump-latest-scrim").is_some());
    let gap = f32::from(card.top() - jump.bottom());
    let want = Tokens::V2.shell().jump_gap;
    assert!((gap - want).abs() <= 1., "gap {gap}, want {want}");
}
