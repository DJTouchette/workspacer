//! R5.1 Toggle: on is the primary track with an `on_primary` knob at the
//! end; off is a quiet track (`border_strong`, ≤ 2:1 on the surface) with a
//! secondary-grey knob at the start, so an off toggle never reads as a
//! filled "on" pill in any theme. gpui-component's `Switch` takes the same
//! colours (`ui::configure_theme_v2`), so both agree.
use super::super::chrome::interactive_control;
use super::*;

/// A toggle's track and knob colours (0xRRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToggleColors {
    pub(crate) track: u32,
    pub(crate) knob: u32,
}

/// The colours of a toggle `on` or off. Disabled keeps the same pair; the
/// control drops to `wash.yielded` opacity instead.
pub(crate) fn toggle_colors(on: bool, enabled: bool, p: &Palette) -> ToggleColors {
    let _ = enabled;
    if on {
        ToggleColors {
            track: p.primary,
            knob: p.on_primary,
        }
    } else {
        ToggleColors {
            track: p.roles.border_strong,
            knob: p.roles.text_secondary,
        }
    }
}

/// A toggle (switch). Add `.on_click` to flip it and a `.tooltip` naming
/// the setting; the setting's row carries the label.
pub(crate) fn toggle(
    id: impl Into<gpui::ElementId>,
    on: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let sys = t.system();
    let c = toggle_colors(on, enabled, p);
    interactive_control(div().id(id.into()), *p, enabled)
        .debug_selector(move || format!("toggle-{}", if on { "on" } else { "off" }))
        .flex_shrink_0()
        .w(px(sys.toggle_width))
        .h(px(sys.toggle_height))
        .rounded(px(t.radius.full))
        .bg(rgb(c.track))
        .flex()
        .items_center()
        // The reserved 2px focus border (`interactive_control`) is the
        // knob's inset (`system().toggle_inset`).
        .when(on, |d| d.justify_end())
        .when(!enabled, |d| d.opacity(t.wash.yielded))
        .child(
            div()
                .size(px(sys.toggle_knob))
                .rounded(px(t.radius.full))
                .bg(rgb(c.knob)),
        )
}
