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
//! - **Off is unmistakably off** (r6-surfaces, R6.1). An off toggle is a
//!   light, quiet track (≤ 2:1 on its surface) outlined by a hairline
//!   (≥ 3:1 on every surface) with a white knob that carries the same
//!   hairline (≥ 3:1 against it). On is a strong accent track (≥ 3:1 on
//!   every surface) under a white knob (≥ 3:1). Light's off switches were
//!   dark filled pills (R4), then a dark knob that read as on (R5).
//!
//! And three colour rules (r6-surfaces, R6.2), as perceptual distances
//! (OKLab ΔE) and hue angles, so they hold for colour-blind-ish eyes too:
//! - **Needs-you amber and fault red stay apart** in every theme.
//! - **The capacity warning hue** sits apart from both.
//! - **The scrim** dims the window 45–55% in every theme.
use super::super::primitives::{ButtonVariant, ControlState, CountTone, StatusKind, button_paint};
use super::*;
use wks_native::appearance::{contrast, luminance};

const TEXT: f64 = 4.5;
const NON_TEXT: f64 = 3.0;
/// A light Primary fill against its surfaces (see the module docs).
const LIGHT_FILL: f64 = 4.5;
/// The most an off toggle's track may stand off its surface.
const QUIET_TRACK: f64 = 2.0;
/// Amber vs red: the least OKLab distance and hue angle (degrees).
const STATUS_APART: (f64, f64) = (0.10, 30.);
/// The warning hue vs amber and vs red.
const WARNING_APART: (f64, f64) = (0.06, 15.);
/// The least a selected segment's fill stands off its track.
const SEGMENT: f64 = 1.15;
/// The scrim's band, as a share of the shadow colour.
const SCRIM: (f32, f32) = (0.45, 0.55);

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
            track: c.switch_on.map(hsla_hex).unwrap_or(p.primary),
            knob: thumb,
            edge: None,
        },
        ToggleColors {
            track: hsla_hex(c.switch),
            knob: off_thumb,
            edge: c.switch_border.map(hsla_hex),
        },
    )
}

/// OKLab (L, a, b) of an sRGB colour.
fn oklab(c: u32) -> (f64, f64, f64) {
    let lin = |v: u32| {
        let v = v as f64 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin((c >> 16) & 0xff), lin((c >> 8) & 0xff), lin(c & 0xff));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    )
}

/// The perceptual distance (OKLab ΔE) and hue angle (degrees) between two
/// colours.
fn apart(x: u32, y: u32) -> (f64, f64) {
    let (a, b) = (oklab(x), oklab(y));
    let distance = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt();
    let hue = |c: (f64, f64, f64)| c.2.atan2(c.1).to_degrees();
    let mut angle = (hue(a) - hue(b)).abs() % 360.;
    if angle > 180. {
        angle = 360. - angle;
    }
    (distance, angle)
}

/// Failures where `x` and `y` are closer than `(distance, angle)`.
fn separation(theme: Appearance, what: &str, x: u32, y: u32, min: (f64, f64)) -> Option<String> {
    let (distance, angle) = apart(x, y);
    (distance < min.0 || angle < min.1).then(|| {
        format!(
            "{theme:?}: {what}: ΔE {distance:.3} (≥ {}), hue {angle:.0}° (≥ {}) ({} vs {})",
            min.0,
            min.1,
            hex(x),
            hex(y)
        )
    })
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
    // The capacity warning hue is read as text (a meter's number).
    for (name, bg) in rows(&p) {
        g.at_least(&format!("warning (caution) on {name}"), r.caution, bg, TEXT);
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
        if on.knob != 0xffffff || off.knob != 0xffffff {
            g.failures
                .push(format!("{theme:?}: {who}: the knob is white, on and off"));
        }
        let Some(edge) = off.edge else {
            g.failures
                .push(format!("{theme:?}: {who} off: no hairline"));
            continue;
        };
        // The white knob stands off its light track by its hairline.
        g.at_least(
            &format!("{who} off: hairline on knob"),
            edge,
            off.knob,
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
                &format!("{who} off: hairline on {name}"),
                edge,
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

    // Composer (R5.4): every live state keeps its text and placeholder
    // readable on the field (a decision waiting never fades it), and the
    // focused edge reads as the focus ring.
    for state in super::ComposerState::ALL {
        if state == super::ComposerState::Disabled {
            continue;
        }
        g.at_least(
            &format!("composer {state:?}: text on field"),
            p.text,
            p.surface,
            TEXT,
        );
        g.at_least(
            &format!("composer {state:?}: placeholder on field"),
            r.text_tertiary,
            p.surface,
            TEXT,
        );
    }
    g.at_least(
        "composer Focus: edge on field",
        super::ComposerState::Focus.edge(&p),
        p.surface,
        NON_TEXT,
    );

    // SegmentedControl (r6-surfaces): the selected segment stands off its
    // track on every surface (it also carries a hairline and a shadow), and
    // its label (and count, in the label's colour) still reads.
    g.at_least("segment selected label", p.text, r.segment_selected, TEXT);

    for (name, bg) in surfaces(&p) {
        let track = composite(super::super::primitives::tonal(&p, p.tokens.wash.hover), bg);
        g.at_least(
            &format!("segment selected on its track on {name}"),
            r.segment_selected,
            track,
            SEGMENT,
        );
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

// r6-surfaces (R6.2)
#[test]
fn needs_you_fault_and_warning_stay_apart_in_every_theme() {
    let failures: Vec<String> = Appearance::ALL
        .into_iter()
        .flat_map(|theme| {
            let p = theme.palette_for(Design::V2);
            let s = p.roles.status;
            [
                separation(
                    theme,
                    "needs-you amber vs fault red",
                    s.attention,
                    s.fault,
                    STATUS_APART,
                ),
                separation(
                    theme,
                    "warning vs needs-you amber",
                    p.roles.caution,
                    s.attention,
                    WARNING_APART,
                ),
                separation(
                    theme,
                    "warning vs fault red",
                    p.roles.caution,
                    s.fault,
                    WARNING_APART,
                ),
            ]
            .into_iter()
            .flatten()
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// r6-surfaces (R6.1)
#[test]
fn the_scrim_dims_45_to_55_percent_in_every_theme() {
    for theme in Appearance::ALL {
        let a = theme.palette_for(Design::V2).roles.scrim_alpha;
        assert!((SCRIM.0..=SCRIM.1).contains(&a), "{theme:?}: scrim {a}");
    }
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
