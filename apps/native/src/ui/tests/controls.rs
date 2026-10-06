//! Buttons, focus rings, truncation, interface size and themes.
use super::*;

struct HoverControls(Entity<Workspace>);

impl Render for HoverControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |workspace, cx| {
            div()
                .text_size(px(20.))
                .flex()
                .flex_col()
                .items_start()
                .gap_4()
                .child(
                    workspace
                        .button("regular", "Regular action", true)
                        .debug_selector(|| "regular".into())
                        .on_click(|_, _, _| {}),
                )
                .child(
                    workspace
                        .primary_button("primary", "Primary action", true)
                        .debug_selector(|| "primary".into())
                        .on_click(|_, _, _| {}),
                )
                .child(
                    workspace
                        .quiet_button("quiet", "Quiet action", IconName::Info, true)
                        .debug_selector(|| "quiet".into())
                        .on_click(|_, _, _| {}),
                )
                .child(
                    workspace
                        .icon_button("icon", "Icon action", IconName::Info, true)
                        .debug_selector(|| "icon".into())
                        .on_click(|_, _, _| {}),
                )
                .child(
                    workspace
                        .file_button(
                            "file",
                            wks_native::links::tool_file("/repo", "src/main.rs", None),
                            "src/main.rs",
                            cx,
                        )
                        .debug_selector(|| "file".into()),
                )
        })
    }
}

/// A truncating title beside a fixed badge in a row of `width`; keeps the
/// title's text layout so tests can read back what GPUI actually drew.
struct TruncateProbe {
    width: gpui::Pixels,
    layout: Option<gpui::TextLayout>,
}

impl Render for TruncateProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let title = gpui::StyledText::new("A session title far too long for its row");
        self.layout = Some(title.layout().clone());
        div()
            .w(self.width)
            .flex()
            .items_center()
            .gap_2()
            .debug_selector(|| "probe-row".into())
            .child(div().min_w_0().truncate().child(title))
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(60.))
                    .debug_selector(|| "probe-badge".into())
                    .child("Running"),
            )
    }
}

// Patched GPUI (vendor/gpui/WORKSPACER-PATCHES.md): truncated text is
// re-measured at its final flex width, so it gains "…" when the row is
// narrow and loses it again when the row widens. Upstream 0.2.2 kept the
// first, unconstrained measurement and only clipped.
#[gpui::test]
fn truncated_text_ellipsizes_at_its_flex_width_and_recovers(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| TruncateProbe {
        width: px(180.),
        layout: None,
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let drawn = |visual: &mut VisualTestContext| {
        window
            .update(visual, |probe, _, _| probe.layout.as_ref().unwrap().text())
            .unwrap()
    };
    let resize = |visual: &mut VisualTestContext, width: f32| {
        window
            .update(visual, |probe, _, cx| {
                probe.width = px(width);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
    };
    visual.run_until_parked();
    let narrow = drawn(&mut visual);
    assert!(
        narrow.ends_with('…'),
        "narrow title was clipped, not ellipsized: {narrow:?}"
    );
    assert!(narrow.len() < "A session title far too long for its row".len());
    let row = visual.debug_bounds("probe-row").unwrap();
    let badge = visual.debug_bounds("probe-badge").unwrap();
    assert!(badge.right() <= row.right(), "badge pushed out of its row");

    resize(&mut visual, 2000.);
    assert_eq!(
        drawn(&mut visual),
        "A session title far too long for its row"
    );

    resize(&mut visual, 150.);
    let narrower = drawn(&mut visual);
    assert!(
        narrower.ends_with('…'),
        "re-narrowed title lost its ellipsis: {narrower:?}"
    );
    assert!(narrower.len() < narrow.len(), "{narrower:?} vs {narrow:?}");
}

#[gpui::test]
fn button_hover_and_press_preserve_geometry(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (controller, _commands, _updates) = Controller::test_channels();
    let window = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(controller, true, window, cx));
        workspace.update(cx, |workspace, _| workspace.view = Arc::new(state("a")));
        let controls = cx.new(|_| HoverControls(workspace));
        Root::new(controls, window, cx)
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    for selector in ["regular", "primary", "quiet", "icon", "file"] {
        visual.simulate_mouse_move(
            gpui::point(px(800.), px(600.)),
            None,
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        let before = visual.debug_bounds(selector).unwrap();
        visual.simulate_mouse_move(before.center(), None, gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            before,
            visual.debug_bounds(selector).unwrap(),
            "{selector} hover changed geometry"
        );
        visual.simulate_mouse_down(
            before.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        assert_eq!(
            before,
            visual.debug_bounds(selector).unwrap(),
            "{selector} press changed geometry"
        );
        visual.simulate_mouse_up(
            gpui::point(px(800.), px(600.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
    }
}

#[gpui::test]
fn font_controls_preserve_drafts_and_survive_theme_changes(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            view.transcript.snapshot(ConversationSnapshot {
                seq: 50,
                first_seq: 1,
                items: (0..50)
                    .map(|i| Item {
                        kind: "user_message".into(),
                        text: format!("Message {i}"),
                        ..Default::default()
                    })
                    .collect(),
            });
            this.update_view(Arc::new(view), window, cx);
            this.follow = false;
            this.list.scroll_to(ListOffset {
                item_ix: 12,
                offset_in_item: px(7.),
            });
            this.composer.update(cx, |input, cx| {
                input.set_value("Keep this draft 🦀", window, cx)
            });
            this.fonts.interface.update(cx, |_, cx| {
                cx.emit(
                    SelectEvent::<SearchableVec<typography::FontChoice>>::Confirm(Some(
                        String::new(),
                    )),
                )
            });
            this.fonts.code.update(cx, |_, cx| {
                cx.emit(
                    SelectEvent::<SearchableVec<typography::FontChoice>>::Confirm(Some(
                        "Inter".into(),
                    )),
                )
            });
        });
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            assert!(this.settings.interface_font.is_empty());
            assert_eq!(this.settings.code_font, "Inter");
            let anchor = this.list.logical_scroll_top();
            this.settings.text_size = 19;
            this.set_appearance(Appearance::Nord, window, cx);
            let theme = gpui_component::Theme::global(cx);
            assert_eq!(theme.font_family.as_ref(), ".SystemUIFont");
            assert_eq!(theme.mono_font_family.as_ref(), "Inter");
            assert_eq!(theme.font_size, px(19.));
            assert_eq!(this.list.logical_scroll_top().item_ix, 12);
            assert_eq!(
                this.list.logical_scroll_top().offset_in_item,
                anchor.offset_in_item
            );
            assert_eq!(
                this.composer.read(cx).value().as_ref(),
                "Keep this draft 🦀"
            );
            this.follow = true;
            this.apply_typography(cx);
            assert_eq!(
                this.list.logical_scroll_top().item_ix,
                this.list.item_count()
            );
        });
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn switching_themes_preserves_session_and_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Keep my draft", window, cx));
            for appearance in Appearance::ALL {
                this.set_appearance(appearance, window, cx);
                assert_eq!(this.appearance, appearance);
                assert_eq!(this.view.selected.as_deref(), Some("a"));
                assert_eq!(this.composer.read(cx).value().as_ref(), "Keep my draft");
                let theme = gpui_component::Theme::global(cx);
                assert_eq!(theme.is_dark(), appearance.is_dark());
                assert_eq!(
                    theme.colors.background,
                    gpui::Hsla::from(rgb(appearance.palette().base))
                );
            }
        });
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn keyboard_controls_skip_disabled_actions_and_keep_their_geometry(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("Keyboard draft", window, cx)
            });
            window.focus(&this.focus);
        })
    });
    visual.run_until_parked();
    let before = visual.debug_bounds("sidebar-toggle").unwrap();
    visual.simulate_keystrokes("tab tab tab");
    visual.run_until_parked();
    assert_eq!(before, visual.debug_bounds("sidebar-toggle").unwrap());
    visual.simulate_keystrokes("enter");
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
    // New session is disabled in this demo fixture; Tab should skip it.
    visual.simulate_keystrokes("tab space");
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("space").unwrap(),
    });
    visual.run_until_parked();
    assert!(visual.update(|window, cx| {
        workspace
            .read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    workspace.read_with(&visual, |this, cx| {
        assert!(!this.sidebar_collapsed);
        assert!(!this.new_session);
        assert_eq!(this.composer.read(cx).value().as_str(), "Keyboard draft");
    });
    assert!(commands.try_recv().is_err());
}

/// A pressed control keeps focus (Enter/Space then act on it), but only
/// keyboard focus draws the accent ring: a chip or toggle turned off by
/// mouse must not keep an "on"-looking border until focus moves.
#[gpui::test]
fn pointer_focus_draws_no_ring_but_keyboard_focus_does(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_screen(Screen::Settings, window, cx);
            this.settings_section = settings::SettingsSection::Keyboard;
        })
    });
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    let selected = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.settings.enter_sends)
    };
    let before = selected(&mut visual);
    let other = visual
        .debug_bounds(if before { "send-key-0" } else { "send-key-1" })
        .unwrap();
    visual.simulate_click(other.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    assert_ne!(selected(&mut visual), before, "the click picked the option");
    let chip = visual
        .debug_bounds(if before { "send-key-0" } else { "send-key-1" })
        .unwrap();
    let pressed = visual.update(|window, cx| window.focused(cx));
    assert!(pressed.is_some(), "a pressed control takes focus");
    assert!(
        !accent_border(&workspace, &mut visual, chip),
        "a mouse press leaves no focus ring"
    );

    // Away and back by keyboard: the same control now shows its ring.
    visual.simulate_keystrokes("tab shift-tab");
    visual.run_until_parked();
    assert_eq!(visual.update(|window, cx| window.focused(cx)), pressed);
    assert!(
        accent_border(&workspace, &mut visual, chip),
        "keyboard focus is visible"
    );

    // Pressing it again (turning it back off) hides the ring at once.
    let back = visual
        .debug_bounds(if before { "send-key-1" } else { "send-key-0" })
        .unwrap();
    visual.simulate_click(back.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    assert_eq!(selected(&mut visual), before);
    assert!(!accent_border(&workspace, &mut visual, back));
    assert!(!accent_border(&workspace, &mut visual, chip));
}

#[gpui::test]
fn interface_size_zooms_layout_and_widgets_together(cx: &mut TestAppContext) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_zoom(1.);
        }
    }
    let _reset = Reset;
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    let before = visual.debug_bounds("session-sidebar").unwrap().size.width;
    // Ctrl + steps to the next offered size.
    visual.simulate_keystrokes("ctrl-=");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.settings.interface_scale, 110);
        // gpui-component sizes widgets from the theme font size (rem).
        assert_eq!(
            gpui_component::Theme::global(cx).font_size,
            gpui::px(15. * 1.1)
        );
    });
    let after = visual.debug_bounds("session-sidebar").unwrap().size.width;
    assert!((f32::from(after) - f32::from(before) * 1.1).abs() < 1.);
    // The stored width stays in unzoomed units.
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.settings.sidebar_width, 304.)
    });
    visual.simulate_keystrokes("ctrl-0");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.settings.interface_scale, 100)
    });
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        before
    );
}

#[gpui::test]
fn theme_picker_offers_all_eight_palettes_within_a_narrow_window(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(600.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_screen(Screen::Settings, window, cx);
        })
    });
    visual.run_until_parked();
    let picker = visual.debug_bounds("theme-picker").unwrap();
    let window_width = visual.update(|window, _| window.viewport_size().width);
    let selectors = [
        "theme-Dark",
        "theme-Light",
        "theme-Nord",
        "theme-Tokyo Night",
        "theme-Catppuccin Mocha",
        "theme-Gruvbox",
        "theme-Everforest",
        "theme-Catppuccin Latte",
    ];
    for (appearance, selector) in Appearance::ALL.into_iter().zip(selectors) {
        assert_eq!(selector, format!("theme-{}", appearance.label()));
        let tile = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{appearance:?} tile missing"));
        assert!(tile.right() <= window_width, "{appearance:?} tile clipped");
        assert!(
            picker.contains(&tile.center()),
            "{appearance:?} outside the picker"
        );
    }
    // 't' in Normal mode cycles through every palette and back.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            for appearance in Appearance::ALL.iter().cycle().skip(1).take(8) {
                let index = Appearance::ALL
                    .iter()
                    .position(|a| *a == this.appearance)
                    .unwrap();
                this.set_appearance(
                    Appearance::ALL[(index + 1) % Appearance::ALL.len()],
                    window,
                    cx,
                );
                assert_eq!(this.appearance, *appearance);
            }
        })
    });
}
