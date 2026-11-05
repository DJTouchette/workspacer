//! R5.1 Menu: a menu hangs from the control that opened it, never from a
//! window corner. Build the list with `primitives::menu` /
//! `menu_item` / `menu_separator`; anchor it with [`anchored_menu`].
use super::*;
use gpui::relative;

/// Which side of its trigger a menu opens on, and which edge it aligns to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAnchor {
    /// Under the trigger, left edges aligned (toolbar menus, pickers).
    BelowStart,
    /// Under the trigger, right edges aligned (row overflow, split buttons).
    BelowEnd,
    /// Above the trigger, left edges aligned (composer chips).
    AboveStart,
    /// Above the trigger, right edges aligned (the composer's send menu).
    AboveEnd,
    /// r6-surfaces (R6.1): beside the trigger on its right, top edges
    /// aligned. For a control in a card's answer row whose menu must stay
    /// inside the card without covering the row below it (Deny's "Deny
    /// with note…" beside "Always allow").
    BesideEnd,
}

/// `trigger` with `menu` (while `Some`, i.e. open) anchored to it at
/// `anchor`, `system().menu_gap` away, kept inside the window, above
/// everything else (deferred). The caller owns the open flag: toggle it
/// from the trigger's `.on_click`, close it on a choice, on Esc and with
/// `.on_mouse_down_out` on the menu.
///
/// ```ignore
/// anchored_menu(
///     overflow_trigger("job-more", &p).on_click(…),
///     open.then(|| menu(&p).child(menu_item("job-remove", "Remove…", None, true, true, &p))),
///     MenuAnchor::BelowEnd,
///     &p,
/// )
/// ```
pub(crate) fn anchored_menu(
    trigger: impl IntoElement,
    menu: Option<Div>,
    anchor: MenuAnchor,
    p: &Palette,
) -> Div {
    let gap = p.tokens.system().menu_gap;
    let (corner, place, dx, dy) = match anchor {
        MenuAnchor::BelowStart => (
            gpui::Corner::TopLeft,
            div().absolute().top(relative(1.)).left_0(),
            0.,
            gap,
        ),
        MenuAnchor::BelowEnd => (
            gpui::Corner::TopRight,
            div().absolute().top(relative(1.)).right_0(),
            0.,
            gap,
        ),
        MenuAnchor::AboveStart => (
            gpui::Corner::BottomLeft,
            div().absolute().bottom(relative(1.)).left_0(),
            0.,
            -gap,
        ),
        MenuAnchor::AboveEnd => (
            gpui::Corner::BottomRight,
            div().absolute().bottom(relative(1.)).right_0(),
            0.,
            -gap,
        ),
        MenuAnchor::BesideEnd => (
            gpui::Corner::TopLeft,
            div().absolute().top_0().left(relative(1.)),
            gap,
            0.,
        ),
    };
    div()
        .relative()
        .flex_shrink_0()
        .child(trigger)
        .when_some(menu, |d, menu| {
            d.child(
                place.child(gpui::deferred(
                    gpui::anchored()
                        .anchor(corner)
                        .offset(gpui::point(px(dx), px(dy)))
                        .snap_to_window_with_margin(px(gap))
                        .child(menu),
                )),
            )
        })
}
