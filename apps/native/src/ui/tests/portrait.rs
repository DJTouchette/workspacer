//! Tall windows: the rail, its flyout, Ctrl+K, and panels docked below.
use super::*;
use crate::ui::portrait::Peek;
use wks_native::navigation::WindowLayout;

/// A portrait monitor's maximized window (1080×1920 less the taskbar).
const TALL: (f32, f32) = (1080., 1880.);

fn tall(visual: &mut VisualTestContext) {
    visual.simulate_resize(size(px(TALL.0), px(TALL.1)));
    visual.run_until_parked();
}

#[gpui::test]
fn a_tall_window_starts_on_the_rail_and_a_wide_one_gets_its_sidebar_back(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));

    tall(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(56.)
    );
    // Its composer, on the conversation's full width, is not the narrow one.
    let composer = visual.debug_bounds("chat-composer").unwrap();
    assert!(composer.size.width > px(700.), "{composer:?}");

    // Expanding it in the tall window is allowed and kept while tall...
    click(&mut visual, "sidebar-toggle");
    workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));
    // ...and the wide layout gets back what it had.
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));

    // Wide forced in a tall window: no rail, no tall layout.
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.window_layout = WindowLayout::Landscape;
            cx.notify();
        })
    });
    tall(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));
    // Tall forced in a wide window.
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.window_layout = WindowLayout::Portrait;
            cx.notify();
        })
    });
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
}

#[gpui::test]
fn a_tall_window_docks_the_viewer_below_the_conversation(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    tall(&mut visual);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/guide.md"),
        1,
        false,
        None,
        serde_json::json!({"contents": "# Guide\n\nHello.", "size": 16}),
    );
    settle(&mut visual);
    assert!(
        visual.debug_bounds("file-viewer-backdrop").is_none(),
        "docked, not a sheet"
    );
    visual.update(|window, cx| assert!(!workspace.read(cx).viewer_modal(window)));
    let panel = visual.debug_bounds("file-viewer-panel").unwrap();
    let composer = visual.debug_bounds("chat-composer").unwrap();
    assert!(
        composer.bottom() <= panel.top() + px(1.),
        "the conversation sits above the viewer: {composer:?} {panel:?}"
    );
    assert!(composer.size.width > px(700.), "and keeps its width");
    // Most of the window's width, about 45% of its height, at the bottom.
    assert!(panel.size.width > px(TALL.0 - 100.), "{panel:?}");
    let share = f32::from(panel.size.height) / TALL.1;
    assert!((0.4..0.5).contains(&share), "{share}");
    assert!(panel.bottom() >= px(TALL.1 - 1.));

    // Its top edge drags to resize, and the share is kept.
    let edge = visual.debug_bounds("dock-below-resize").unwrap().center();
    visual.simulate_mouse_down(edge, gpui::MouseButton::Left, gpui::Modifiers::default());
    let up = gpui::point(edge.x, edge.y - px(188.));
    visual.simulate_mouse_move(
        up,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    visual.simulate_mouse_up(up, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    let grown = workspace.read_with(&visual, |this, _| this.settings.portrait_dock_share);
    assert!((grown - 0.55).abs() < 0.01, "{grown}");
    assert!(
        visual
            .debug_bounds("file-viewer-panel")
            .unwrap()
            .size
            .height
            > panel.size.height
    );
}

#[gpui::test]
fn ctrl_k_switches_sessions_from_the_rail_flyout(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    tall(&mut visual);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            window.focus(&this.focus);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-flyout").is_none());

    visual.simulate_keystrokes("ctrl-k");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.layout.peek, Some(Peek::Pinned));
        assert!(this.sidebar_collapsed, "a flyout, not the docked sidebar");
    });
    let flyout = visual.debug_bounds("sidebar-flyout").unwrap();
    assert!(flyout.left() >= px(56.), "beside the rail: {flyout:?}");
    assert!(visual.debug_bounds("sidebar-flyout-backdrop").is_some());

    visual.simulate_input("bet");
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(matches!(
        next_effect(&mut commands),
        Some(Command::Select(id)) if id == "b"
    ));
    visual.update(|window, cx| {
        let this = workspace.read(cx);
        assert_eq!(this.layout.peek, None);
        assert_eq!(
            this.search.read(cx).value().as_str(),
            "",
            "the query left with it"
        );
        assert!(this.composer.read(cx).focus_handle(cx).is_focused(window));
    });

    // Esc closes it without choosing anything.
    visual.simulate_keystrokes("ctrl-k");
    visual.simulate_input("alp");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.layout.peek, None);
        assert_eq!(this.search.read(cx).value().as_str(), "");
    });
    assert!(next_effect(&mut commands).is_none());
}

#[gpui::test]
fn resting_on_the_rail_opens_its_flyout_and_leaving_closes_it(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    tall(&mut visual);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    visual.run_until_parked();
    let row = visual.debug_bounds("sidebar-session-1").unwrap();
    visual.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    visual.run_until_parked();
    // Passing over (or clicking) does not open it; resting does.
    workspace.read_with(&visual, |this, _| assert_eq!(this.layout.peek, None));
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(500));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.layout.peek, Some(Peek::Hover))
    });
    assert!(visual.debug_bounds("sidebar-flyout").is_some());
    assert!(
        visual.debug_bounds("sidebar-flyout-backdrop").is_none(),
        "a hover flyout leaves the conversation undimmed"
    );

    let away = gpui::point(px(TALL.0 - 40.), px(TALL.1 / 2.));
    visual.simulate_mouse_move(away, None, gpui::Modifiers::default());
    visual.run_until_parked();
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(300));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert_eq!(this.layout.peek, None));
}
