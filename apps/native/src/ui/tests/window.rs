//! The app-drawn caption: drag surfaces and the controls kept clear of it.
use super::*;

fn caption_mouse_down(
    visual: &mut VisualTestContext,
    position: gpui::Point<gpui::Pixels>,
) -> (bool, bool) {
    chrome::DRAG_HIT.set(false);
    visual.simulate_mouse_down(
        position,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    let default_prevented = visual.update(|window, _| window.default_prevented());
    (chrome::DRAG_HIT.get(), default_prevented)
}

#[gpui::test]
fn app_drawn_caption_drag_surfaces_survive_layouts_and_exclude_controls(cx: &mut TestAppContext) {
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    for (width, sidebar_width) in [
        (720., 200.),
        (720., 304.),
        (720., 520.),
        (1000., 304.),
        (1600., 520.),
    ] {
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.settings.sidebar_width = sidebar_width;
                cx.notify();
            })
        });
        visual.simulate_resize(size(px(width), px(700.)));
        for collapsed in [false, true] {
            for screen in [
                Screen::Conversation,
                Screen::Projects,
                Screen::Settings,
                Screen::Recent,
                Screen::Jobs,
                Screen::Changes,
                Screen::History,
                Screen::Session,
                Screen::Setup,
                Screen::Model,
            ] {
                // Also exercise New Session, which takes a separate render branch.
                for new_session in [false, true] {
                    visual.update(|_, cx| {
                        workspace.update(cx, |this, cx| {
                            this.sidebar_collapsed = collapsed;
                            this.screen = screen;
                            this.new_session = new_session;
                            cx.notify();
                        })
                    });
                    visual.run_until_parked();
                    let drag = visual.debug_bounds("sidebar-drag-region").unwrap();
                    assert!(
                        drag.size.width >= px(40.) && drag.size.height >= px(32.),
                        "positive, practical sidebar drag bounds: {drag:?}"
                    );
                    let grab = if collapsed {
                        drag.center()
                    } else {
                        let actions = visual.debug_bounds("new-session-button").unwrap();
                        assert!(
                            actions.left() - drag.left() >= px(40.),
                            "at least 40px remains beside the controls at minimum width"
                        );
                        for point in [
                            gpui::point(drag.left() + px(6.), drag.top() + px(6.)),
                            gpui::point(drag.left() + px(38.), drag.bottom() - px(6.)),
                        ] {
                            let (hit, prevented) = caption_mouse_down(&mut visual, point);
                            assert!(hit && !prevented, "row padding is usable chrome");
                        }
                        gpui::point(drag.left() + px(24.), drag.center().y)
                    };
                    let (hit, default_prevented) = caption_mouse_down(&mut visual, grab);
                    assert!(
                        hit,
                        "sidebar grab reachable: {width} {collapsed} {screen:?}"
                    );
                    assert!(
                        !default_prevented,
                        "Windows must be allowed to start the native move"
                    );
                    for selector in [
                        "sidebar-toggle",
                        "new-session-button",
                        "caption-minimize",
                        "caption-maximize",
                        "caption-close",
                    ] {
                        let control = visual.debug_bounds(selector).unwrap();
                        assert!(control.size.width > px(0.) && control.size.height > px(0.));
                        assert!(
                            !caption_mouse_down(&mut visual, control.center()).0,
                            "{selector} must exclude native Drag"
                        );
                    }
                    let caption = visual.debug_bounds("window-caption").unwrap();
                    assert_eq!(caption.top(), px(0.));
                    assert_eq!(caption.right(), px(width));
                    assert_eq!(
                        visual.debug_bounds("caption-close").unwrap().right(),
                        px(width)
                    );
                    if screen == Screen::Conversation && !new_session {
                        let pill = visual.debug_bounds("title-bar").unwrap();
                        assert!(pill.right() <= caption.left(), "title pill clears caption");
                        assert!(
                            !caption_mouse_down(&mut visual, pill.center()).0,
                            "the occluding pill excludes its underlying drag hitbox"
                        );
                        let header = visual.debug_bounds("chat-drag-region").unwrap();
                        let (hit, default_prevented) = caption_mouse_down(
                            &mut visual,
                            gpui::point(header.left() + px(8.), header.top() + px(8.)),
                        );
                        assert!(hit && !default_prevented);
                        assert!(
                            !caption_mouse_down(
                                &mut visual,
                                gpui::point(header.center().x, header.bottom() + px(20.))
                            )
                            .0,
                            "content below the chrome must not drag"
                        );
                    }
                }
            }
        }
    }
    // The toggle still performs its normal action with the native chrome present.
    let toggle = visual.debug_bounds("sidebar-toggle").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));
}

#[gpui::test]
fn secondary_pages_keep_actions_clear_of_the_caption_and_drag_from_the_top(
    cx: &mut TestAppContext,
) {
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    for width in [720., 1200.] {
        visual.simulate_resize(size(px(width), px(700.)));
        for (screen, new_session) in [
            (Screen::Projects, false),
            (Screen::Settings, false),
            (Screen::Recent, false),
            (Screen::Jobs, false),
            (Screen::Changes, false),
            (Screen::History, false),
            (Screen::Session, false),
            (Screen::Setup, false),
            (Screen::Model, false),
            (Screen::Conversation, true),
        ] {
            visual.update(|_, cx| {
                workspace.update(cx, |this, cx| {
                    this.screen = screen;
                    this.new_session = new_session;
                    cx.notify();
                })
            });
            visual.run_until_parked();
            let caption = visual.debug_bounds("window-caption").unwrap();
            let title = visual.debug_bounds("page-title").unwrap();
            assert!(
                title.top() >= caption.bottom(),
                "{screen:?} title starts below the caption strip at {width}"
            );
            for selector in ["page-actions", "feature-back"] {
                if let Some(control) = visual.debug_bounds(selector) {
                    assert!(
                        !control.intersects(&caption),
                        "{selector} on {screen:?} sits under the caption at {width}"
                    );
                }
            }
            let strip = visual.debug_bounds("page-drag-region").unwrap();
            assert!(
                strip.right() <= caption.left(),
                "strip stops at the caption"
            );
            let (hit, prevented) = caption_mouse_down(
                &mut visual,
                gpui::point(strip.center().x, strip.top() + px(8.)),
            );
            assert!(hit && !prevented, "{screen:?} top strip drags the window");
            assert!(
                !caption_mouse_down(&mut visual, title.center()).0,
                "page content below the strip never drags"
            );
        }
    }
}

#[gpui::test]
fn original_non_occluding_drag_is_cancelled_by_shell_focus(cx: &mut TestAppContext) {
    // Reproduce cd7a5028's event path without requiring a Windows window.
    // Native WM_NCLBUTTONDOWN checks this exact DispatchEventResult before
    // delegating HTCAPTION to DefWindowProc.
    struct DragFixture {
        focus: FocusHandle,
        occlude: bool,
    }
    impl Render for DragFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().track_focus(&self.focus).child(
                div()
                    .w(px(200.))
                    .h(px(52.))
                    .window_control_area(gpui::WindowControlArea::Drag)
                    .when(self.occlude, |d| d.occlude()),
            )
        }
    }
    let mut view = None;
    let window = cx.add_window(|_, cx| {
        view = Some(cx.entity());
        DragFixture {
            focus: cx.focus_handle(),
            occlude: false,
        }
    });
    let view = view.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let point = gpui::point(px(24.), px(24.));
    let (_, before_prevented) = caption_mouse_down(&mut visual, point);
    assert!(before_prevented, "original shell focus cancels native move");
    visual.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.occlude = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    let (_, default_prevented) = caption_mouse_down(&mut visual, point);
    assert!(
        !default_prevented,
        "occluding drag hitbox excludes the focusable shell"
    );
}
