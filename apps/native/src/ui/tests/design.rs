//! The design v2 preview: its setting, re-theming, and shared primitives.
use super::*;
use wks_native::appearance::Design;

fn theme_colors(cx: &App) -> serde_json::Value {
    serde_json::to_value(gpui_component::Theme::global(cx).colors).unwrap()
}

/// Settings → Appearance → New design (preview) re-themes everything at
/// once (gpui-component colours, terminals, the workspace palette) and
/// switching it off restores the shipped theme exactly.
#[gpui::test]
fn design_v2_switch_rethemes_and_switching_off_restores_v1(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let shipped = visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.set_appearance(Appearance::Nord, window, cx);
            this.show_screen(Screen::Settings, window, cx);
            this.settings_section = settings::SettingsSection::Appearance;
            assert_eq!(this.design(), Design::V1);
            assert_eq!(this.palette(), Appearance::Nord.palette());
        });
        theme_colors(cx)
    });
    visual.run_until_parked();
    // The Appearance card is taller than the default test window.
    visual.simulate_resize(size(px(1000.), px(1600.)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-design-v2").is_some());
    let switch = visual.debug_bounds("design-v2-switch").unwrap();
    visual.simulate_click(
        gpui::point(switch.left() + px(12.), switch.center().y),
        gpui::Modifiers::none(),
    );
    visual.run_until_parked();
    visual.update(|_, cx| {
        let p = workspace.read(cx).palette();
        assert!(workspace.read(cx).settings.design_v2);
        assert_eq!(p.tokens.design, Design::V2);
        assert_eq!(p, Appearance::Nord.palette_for(Design::V2));
        assert_eq!(
            cx.global::<terminal::TerminalPalette>().0.tokens.design,
            Design::V2
        );
        // V2 maps the widget families v1 leaves stock.
        let theme = gpui_component::Theme::global(cx);
        assert_eq!(theme.colors.list, gpui::Hsla::from(rgb(p.surface)));
        assert_eq!(theme.colors.tab_active, gpui::Hsla::from(rgb(p.selected)));
        assert_eq!(theme.colors.popover, gpui::Hsla::from(rgb(p.roles.raised)));
    });
    // Every theme still themes under v2.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            for appearance in Appearance::ALL {
                this.set_appearance(appearance, window, cx);
                let theme = gpui_component::Theme::global(cx);
                assert_eq!(theme.is_dark(), appearance.is_dark());
                assert_eq!(
                    theme.colors.list,
                    gpui::Hsla::from(rgb(appearance.palette_for(Design::V2).surface))
                );
            }
            this.set_appearance(Appearance::Nord, window, cx);
            // r2-pages: under v2 the switch lives in Settings → Labs.
            this.settings_section = settings::SettingsSection::Labs;
        })
    });
    visual.run_until_parked();
    let switch = visual.debug_bounds("design-v2-switch").unwrap();
    visual.simulate_click(
        gpui::point(switch.left() + px(12.), switch.center().y),
        gpui::Modifiers::none(),
    );
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert!(!workspace.read(cx).settings.design_v2);
        assert_eq!(workspace.read(cx).palette(), Appearance::Nord.palette());
        assert_eq!(theme_colors(cx), shipped);
    });
}

/// `--design-v2` forces the preview for a run without touching the setting;
/// switching in Settings replaces the forced choice.
#[gpui::test]
fn a_forced_design_overrides_the_setting_until_it_is_switched(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.force_design(Some(Design::V2));
            this.set_appearance(Appearance::Dark, window, cx);
            assert_eq!(this.design(), Design::V2);
            assert!(!this.settings.design_v2);
            assert_eq!(this.tokens(), Design::V2.tokens());
            this.set_design_v2(false, window, cx);
            assert_eq!(this.design(), Design::V1);
        })
    });
}

struct Gallery(wks_native::appearance::Palette);

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(primitives::gallery(&self.0))
    }
}

/// Every shared primitive renders under every theme in both designs.
#[gpui::test]
fn primitives_render_in_every_theme_and_design(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    for design in [Design::V1, Design::V2] {
        for appearance in Appearance::ALL {
            let palette = appearance.palette_for(design);
            let window = cx.add_window(|window, cx| {
                configure_theme(appearance, design, Some(window), cx);
                let view = cx.new(|_| Gallery(palette));
                Root::new(view, window, cx)
            });
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            visual.simulate_resize(size(px(1000.), px(1600.)));
            visual.run_until_parked();
            for selector in [
                "primitive-button-Primary-Small",
                "primitive-button-DangerGhost-Large",
                "primitive-page-header",
                "primitive-split",
                "primitive-overflow",
                "primitive-status-NeedsYou",
                "primitive-status-Working",
                "primitive-status-Queued",
                "primitive-avatars",
                "primitive-truncated",
                "primitive-card-head",
                "primitive-fade",
                "primitive-segmented",
                "primitive-card",
                "primitive-menu",
                "primitive-dialog",
                "primitive-empty",
            ] {
                assert!(
                    visual.debug_bounds(selector).is_some(),
                    "{design:?} {appearance:?} {selector}"
                );
            }
            // The two control heights, and status boxes of one fixed size
            // whatever the label (so titles after them line up).
            let t = palette.tokens.layout;
            for (selector, expected) in [
                ("primitive-button-Primary-Small", t.control_height_sm),
                ("primitive-button-DangerGhost-Large", t.control_height),
            ] {
                let bounds = visual.debug_bounds(selector).unwrap();
                assert_eq!(f32::from(bounds.size.height), expected, "{selector}");
            }
            let working = visual.debug_bounds("primitive-status-Working").unwrap();
            let idle = visual.debug_bounds("primitive-status-Idle").unwrap();
            assert_eq!(working.size.height, idle.size.height);
        }
    }
}
