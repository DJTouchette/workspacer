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
    // The wordmark sits beside the mark; the footer is one row.
    assert!(visual.debug_bounds("sidebar-wordmark").is_some());
    let footer = visual.debug_bounds("sidebar-footer").unwrap();
    assert!(f32::from(footer.size.height) <= Tokens::V2.layout.footer_height);
    for item in ["nav-jobs", "nav-hosts", "nav-scratch", "nav-settings"] {
        let bounds = visual.debug_bounds(item).unwrap();
        assert!(footer.contains(&bounds.center()), "{item} in the footer");
    }
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
    let height = |visual: &mut VisualTestContext| {
        let a = visual.debug_bounds("sidebar-session-1").unwrap();
        let b = visual.debug_bounds("sidebar-session-2").unwrap();
        f32::from(b.top() - a.top())
    };
    assert_eq!(height(&mut visual), Tokens::V2.layout.sidebar_row);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.sidebar_compact = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert_eq!(height(&mut visual), Tokens::V2.layout.sidebar_row_compact);
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
    click(&mut visual, "rail-next-needs-you");
    assert_eq!(selected(&mut commands), ["e"]);
    // The rail's first mark is the longest-waiting session.
    click(&mut visual, "sidebar-session-0");
    assert_eq!(selected(&mut commands), ["e"]);
}

#[gpui::test]
fn the_cheat_sheet_opens_and_closes(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, attention_state(), Design::V2);
    visual.update(|_, cx| workspace.update(cx, |this, cx| this.toggle_shortcuts(cx)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("shortcuts-dialog").is_some());
    click(&mut visual, "shortcuts-close");
    assert!(visual.debug_bounds("shortcuts-dialog").is_none());
}

#[gpui::test]
fn the_model_chip_opens_a_popover_not_a_page(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let mut view = attention_state();
    view.selected = Some("a".into());
    open_v2(&workspace, &mut visual, view, Design::V2);
    // The access mode is always in the capsule, even when unknown.
    assert!(visual.debug_bounds("title-access").is_some());
    click(&mut visual, "title-model");
    assert!(visual.debug_bounds("model-popover").is_some());
    assert!(visual.debug_bounds("popover-access-unknown").is_some());
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
