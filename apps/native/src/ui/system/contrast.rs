//! R5.3 The contrast gate: every component state of `docs/design-system.md`
//! in every built-in theme. Text ≥ 4.5:1 (WCAG 1.4.3); non-text ≥ 3:1
//! (WCAG 1.4.11: glyphs, the focus ring, toggle track and knob, the Primary
//! fill). Translucent fills are composited over the surface they sit on.
//! Every failure in every theme is collected and reported at once.
//!
//! Two rules go beyond WCAG because reviewers caught them:
//! - **A light Primary fill must stand out (≥ 4.5:1 on its surfaces).**
//!   A Primary that carries a dark label is a light fill; at 3–4.5:1 off a
//!   dark surface it reads as a disabled chip (Nord's frost Allow, R4).
//! - **Off is unmistakably off.** An off toggle's track is a quiet well
//!   (≤ 2:1 on its surface), the on track a strong fill (≥ 3:1), and the
//!   knob carries the state (≥ 3:1 on both). Light's off switches were
//!   dark filled pills under a white knob (6:1), heavier than "on" (R4).
use super::super::primitives::{ButtonVariant, ControlState, CountTone, StatusKind, button_paint};
use super::*;
use wks_native::appearance::{contrast, luminance};

const TEXT: f64 = 4.5;
const NON_TEXT: f64 = 3.0;
/// A light Primary fill against its surfaces (see the module docs).
const LIGHT_FILL: f64 = 4.5;
/// The most an off toggle's track may stand off its surface.
const QUIET_TRACK: f64 = 2.0;

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

struct Gate {
    theme: Appearance,
    failures: Vec<String>,
}

impl Gate {
    fn at_least(&mut self, what: &str, fg: u32, bg: u32, min: f64) {
        let ratio = contrast(fg, bg);
        if ratio < min {
            self.failures.push(format!(
                "{:?}: {what}: {ratio:.2} < {min} ({} on {})",
                self.theme,
                hex(fg),
                hex(bg)
            ));
        }
    }

    fn at_most(&mut self, what: &str, fg: u32, bg: u32, max: f64) {
        let ratio = contrast(fg, bg);
        if ratio > max {
            self.failures.push(format!(
                "{:?}: {what}: {ratio:.2} > {max} ({} on {})",
                self.theme,
                hex(fg),
                hex(bg)
            ));
        }
    }
}

/// The opaque surfaces things rest on.
fn surfaces(p: &Palette) -> [(&'static str, u32); 4] {
    [
        ("chat", p.chat),
        ("surface", p.surface),
        ("base", p.base),
        ("raised", p.roles.raised),
    ]
}

/// Surfaces plus the row fills text sits on (a selected or hovered row).
fn rows(p: &Palette) -> [(&'static str, u32); 6] {
    let [a, b, c, d] = surfaces(p);
    [
        a,
        b,
        c,
        d,
        ("selected", p.selected),
        ("hover", p.roles.hover),
    ]
}

fn hsla_hex(c: gpui::Hsla) -> u32 {
    composite(gpui::Hsla { a: 1., ..c }, 0)
}

/// What gpui-component's `Switch` draws under the v2 theme mapping.
fn switch_colors(p: &Palette, dark: bool) -> (ToggleColors, ToggleColors) {
    let mut c = gpui_component::ThemeColor::default();
    super::super::configure_theme_v2(&mut c, p, dark);
    let thumb = hsla_hex(c.switch_thumb);
    let off_thumb = c.switch_thumb_off.map(hsla_hex).unwrap_or(thumb);
    (
        ToggleColors {
            track: p.primary,
            knob: thumb,
        },
        ToggleColors {
            track: hsla_hex(c.switch),
            knob: off_thumb,
        },
    )
}

fn check(theme: Appearance) -> Vec<String> {
    let p = theme.palette_for(Design::V2);
    let r = p.roles;
    let mut g = Gate {
        theme,
        failures: Vec::new(),
    };

    // Foundations: three text levels, prose and links on every surface and
    // row ("faint text on raised surfaces" included).
    for (name, bg) in rows(&p) {
        for (tier, fg) in [
            ("text", p.text),
            ("prose", p.prose),
            ("secondary text", r.text_secondary),
            ("tertiary text", r.text_tertiary),
            ("link (accent)", p.accent),
        ] {
            g.at_least(&format!("{tier} on {name}"), fg, bg, TEXT);
        }
        // StatusCell / StatusBadge words, and every glyph.
        for kind in StatusKind::ALL {
            g.at_least(
                &format!("{kind:?} word on {name}"),
                kind.text_color(&p),
                bg,
                TEXT,
            );
            g.at_least(
                &format!("{kind:?} glyph on {name}"),
                kind.color(&p),
                bg,
                NON_TEXT,
            );
        }
    }
    // The label of a pressed row still reads.
    g.at_least("text on pressed", p.text, r.pressed, TEXT);

    for (name, bg) in surfaces(&p) {
        // Button: every tier in every interactive state.
        for variant in [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Tertiary,
            ButtonVariant::Destructive,
        ] {
            for state in [
                ControlState::Rest,
                ControlState::Hover,
                ControlState::Pressed,
            ] {
                let (fill, label, _) = button_paint(variant, state, &p);
                let under = composite(fill, bg);
                g.at_least(
                    &format!("{variant:?} label ({state:?}) on {name}"),
                    label,
                    under,
                    TEXT,
                );
            }
        }
        // The Primary fill identifies the control; a light one must not
        // read as disabled.
        let dark_label = luminance(p.on_primary) < luminance(p.primary);
        g.at_least(
            &format!("Primary fill on {name}"),
            p.primary,
            bg,
            if dark_label { LIGHT_FILL } else { NON_TEXT },
        );
        // Focus ring.
        g.at_least(&format!("focus ring on {name}"), r.focus, bg, NON_TEXT);
        // CountBadge (fills are opaque or washes of text).
        for tone in [CountTone::Neutral, CountTone::NeedsYou, CountTone::Failed] {
            let (fill, label) = match tone {
                CountTone::Neutral => (
                    composite(
                        super::super::primitives::tonal(&p, p.tokens.wash.pressed),
                        bg,
                    ),
                    r.text_secondary,
                ),
                CountTone::NeedsYou => (r.status.attention, p.base),
                CountTone::Failed => (r.status.fault, p.base),
            };
            g.at_least(&format!("{tone:?} count on {name}"), label, fill, TEXT);
        }
        // Keycap hint off a fill (tertiary).
        g.at_least(&format!("keycap hint on {name}"), r.text_tertiary, bg, TEXT);
    }
    // Keycap on the Primary fill: the label colour at rest, hover, pressed.
    for (state, fill) in [
        ("rest", p.primary),
        ("hover", p.primary_hover),
        ("pressed", p.primary_pressed),
    ] {
        g.at_least(
            &format!("keycap on Primary ({state})"),
            p.on_primary,
            fill,
            TEXT,
        );
    }

    // NavItem: selected label on the selected fill, rest label on the
    // sidebar, hovered label on the hover fill.
    g.at_least("NavItem selected", p.text, p.selected, TEXT);
    g.at_least("NavItem rest", r.text_secondary, p.base, TEXT);
    g.at_least("NavItem hover", p.text, r.hover, TEXT);

    // Menu: items on the raised surface and its hover; destructive items.
    for (name, bg) in [("raised", r.raised), ("hover", r.hover)] {
        g.at_least(&format!("menu item on {name}"), p.text, bg, TEXT);
        g.at_least(&format!("destructive item on {name}"), p.error, bg, TEXT);
    }

    // Toggle: ours and the Settings switch, on and off, on every surface.
    let dark = theme.is_dark();
    let (switch_on, switch_off) = switch_colors(&p, dark);
    for (who, on, off) in [
        (
            "toggle",
            toggle_colors(true, true, &p),
            toggle_colors(false, true, &p),
        ),
        ("switch", switch_on, switch_off),
    ] {
        g.at_least(
            &format!("{who} on: knob on track"),
            on.knob,
            on.track,
            NON_TEXT,
        );
        g.at_least(
            &format!("{who} off: knob on track"),
            off.knob,
            off.track,
            NON_TEXT,
        );
        for (name, bg) in surfaces(&p) {
            g.at_least(
                &format!("{who} on: track on {name}"),
                on.track,
                bg,
                NON_TEXT,
            );
            g.at_least(
                &format!("{who} off: knob on {name}"),
                off.knob,
                bg,
                NON_TEXT,
            );
            g.at_most(
                &format!("{who} off: track on {name} (quiet)"),
                off.track,
                bg,
                QUIET_TRACK,
            );
        }
    }

    // Avatars.
    for &fill in &r.avatar {
        g.at_least("avatar monogram", p.avatar_text(), fill, TEXT);
    }
    g.failures
}

#[test]
fn every_component_state_meets_contrast_in_every_theme() {
    let failures: Vec<String> = Appearance::ALL.into_iter().flat_map(check).collect();
    assert!(
        failures.is_empty(),
        "{} contrast failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn destructive_is_never_brand_blue() {
    for theme in Appearance::ALL {
        let p = theme.palette_for(Design::V2);
        for state in ControlState::ALL {
            let (fill, label, _) = button_paint(ButtonVariant::Destructive, state, &p);
            assert_ne!(label, p.primary, "{theme:?} {state:?}");
            assert_ne!(label, p.accent, "{theme:?} {state:?}");
            assert_ne!(composite(fill, p.surface), p.primary, "{theme:?} {state:?}");
        }
    }
}
