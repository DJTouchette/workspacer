//! R5.1 Toggle, redrawn in r6-surfaces (R6.1). On is the accent track
//! (`roles.toggle_on`: the accent, deepened until the white knob stands
//! off it) with a white knob at the end. Off is a light, quiet track (the
//! hover wash) with a hairline (`roles.toggle_edge`, ≥ 3:1 on every
//! surface) and a white knob at the start that carries the same hairline,
//! so an off toggle reads as an empty switch in every theme and never as
//! a filled "on" pill. gpui-component's `Switch` takes the same colours
//! (`ui::configure_theme_v2`), so both agree.
use super::super::chrome::interactive_control;
use super::*;

/// A toggle's colours (0xRRGGBB): its track, its knob, and the hairline
/// both carry when off (`None` when on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToggleColors {
    pub(crate) track: u32,
    pub(crate) knob: u32,
    pub(crate) edge: Option<u32>,
}

/// The colours of a toggle `on` or off. Disabled keeps the same set; the
/// control drops to `wash.yielded` opacity instead.
pub(crate) fn toggle_colors(on: bool, enabled: bool, p: &Palette) -> ToggleColors {
    let _ = enabled;
    let r = p.roles;
    if on {
        ToggleColors {
            track: r.toggle_on,
            knob: r.toggle_knob,
            edge: None,
        }
    } else {
        ToggleColors {
            track: r.hover,
            knob: r.toggle_knob,
            edge: Some(r.toggle_edge),
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
    let hairline = px(t.stroke.hairline);
    interactive_control(div().id(id.into()), *p, enabled)
        .debug_selector(move || format!("toggle-{}", if on { "on" } else { "off" }))
        .flex_shrink_0()
        .w(px(sys.toggle_width))
        .h(px(sys.toggle_height))
        .rounded(px(t.radius.full))
        .bg(rgb(c.track))
        .relative()
        .flex()
        .items_center()
        // The reserved 2px focus border (`interactive_control`) is the
        // knob's inset (`system().toggle_inset`).
        .when(on, |d| d.justify_end())
        .when(!enabled, |d| d.opacity(t.wash.yielded))
        // The off hairline traces the track's outer edge (over the
        // reserved focus border, which the track's fill already covers).
        .when_some(c.edge, |d, edge| {
            let out = px(-sys.toggle_inset);
            d.child(
                div()
                    .absolute()
                    .top(out)
                    .left(out)
                    .right(out)
                    .bottom(out)
                    .rounded(px(t.radius.full))
                    .border(hairline)
                    .border_color(rgb(edge)),
            )
        })
        .child(
            div()
                .size(px(sys.toggle_knob))
                .rounded(px(t.radius.full))
                .bg(rgb(c.knob))
                .when_some(c.edge, |d, edge| d.border(hairline).border_color(rgb(edge))),
        )
}
