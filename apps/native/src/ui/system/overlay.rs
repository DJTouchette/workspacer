//! R5.1 Overlay / Dialog: anything that floats over the page and takes the
//! keyboard (a dialog, the command palette, the shortcuts sheet) sits on a
//! scrim, on the Overlay elevation, centred on the *window*.
use super::*;

/// Where the panel sits on the scrim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Centred both ways: dialogs and confirmations.
    Center,
    /// Centred across, `system().overlay_top` from the top: the palette and
    /// the shortcuts sheet, which grow downward as they filter.
    Top,
}

/// The scrim layer holding `panel` (build it with
/// `primitives::dialog(&p)` or [`super::elevated`]`(Level::Overlay, &p)`).
/// Absolute and full-size: mount it as the **last child of the window's
/// root** (the Workspace render), never inside a pane, so it centres on the
/// window and dims everything. It occludes the page beneath; dismiss with
/// `.on_mouse_down_out` on the panel and with Esc.
pub(crate) fn overlay(placement: Placement, panel: impl IntoElement, p: &Palette) -> Div {
    let top = p.tokens.system().overlay_top;
    div()
        .debug_selector(|| "overlay-scrim".into())
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .map(|d| match placement {
            Placement::Center => d.justify_center(),
            Placement::Top => d.pt(px(top)),
        })
        .bg(scrim_color(p))
        .occlude()
        .child(panel)
}
