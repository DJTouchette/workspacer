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
        "nav-projects",
        "nav-jobs",
        "nav-hosts",
        "nav-scratch",
        "nav-settings",
    ] {
        let bounds = visual.debug_bounds(item).unwrap();
        assert!(
            list.contains(&bounds.center()),
            "{item} in the destinations"
        );
        assert!(bounds.bottom() < needs.top(), "{item} above the groups");
    }
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
    // one action each.
    assert!(visual.debug_bounds("sidebar-reason-1").is_some());
    assert!(visual.debug_bounds("sidebar-reason-2").is_some());
    effects(&mut commands);
    click(&mut visual, "sidebar-inbox-action-2");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Approve(true, Some(card)) }
            if session == "a" && card == "req-a")));
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
    let height = |visual: &mut VisualTestContext, row: &'static str| {
        f32::from(visual.debug_bounds(row).unwrap().size.height) + 2. * Tokens::V2.stroke.hairline
    };
    let shell = Tokens::V2.shell();
    assert_eq!(height(&mut visual, "sidebar-session-1"), shell.row_double);
    assert_eq!(height(&mut visual, "sidebar-session-4"), shell.row_double);
    assert_eq!(height(&mut visual, "sidebar-session-6"), shell.row_single);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.sidebar_compact = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    // Waiting rows keep their reason; the rest take one compact line.
    assert_eq!(height(&mut visual, "sidebar-session-1"), shell.row_double);
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
    // r3-composer: "a" waits on a decision, which holds the composer's
    // slot; "Reply instead…" opens the composer and its model chip.
    click(&mut visual, "composer-reply");
    click(&mut visual, "composer-model");
    assert!(visual.debug_bounds("model-popover").is_some());
    assert!(visual.debug_bounds("popover-access-unknown").is_none());
    assert!(visual.debug_bounds("popover-model-custom").is_some());
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
