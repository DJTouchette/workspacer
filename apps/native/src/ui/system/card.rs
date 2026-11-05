//! R5.1 Card / Tile: a card is as tall as its content. Tiles in a grid
//! keep their natural height (top-aligned), so a short tile is never a
//! mostly empty box with its content pinned to the bottom.
use super::super::primitives::attention_bar;
use super::*;

/// A tile: the Raised card ([`super::super::primitives::card`]) at its
/// natural height. Put it in a [`tile_grid`].
pub(crate) fn tile(p: &Palette) -> Div {
    super::super::primitives::card(p).h_auto()
}

/// r5-fleet: a tile that waits on the user (a Fleet tile with an ask, a
/// job proposal): the [`tile`] with the `border_strong` edge and the
/// attention bar on its leading edge, its content clear of the bar.
pub(crate) fn waiting_tile(p: &Palette) -> Div {
    let t = p.tokens;
    tile(p)
        .relative()
        .border_color(rgb(p.roles.border_strong))
        .pl(px(t.space.xl + t.signals().attention_bar))
        .child(attention_bar(t.radius.panel, p))
}

/// r5-fleet: the frame a tile sits in, so keyboard focus shows *outside*
/// the tile (and its attention bar): the 2px `focus` ring when `focused`,
/// a transparent one of the same width otherwise, so nothing moves.
/// Add `.on_click` and the tile.
pub(crate) fn tile_frame(
    id: impl Into<gpui::ElementId>,
    focused: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    div()
        .id(id.into())
        .w_full()
        .min_w_0()
        .cursor_pointer()
        .rounded(px(t.radius.panel + t.stroke.focus))
        .border(px(t.stroke.focus))
        .border_color(if focused {
            gpui::Hsla::from(rgb(p.roles.focus))
        } else {
            gpui::transparent_black()
        })
}

/// A grid of [`tile`]s: `columns` equal tracks at `space.lg`, rows aligned
/// to the top so each tile keeps its own height (never stretched).
pub(crate) fn tile_grid(columns: u16, p: &Palette) -> Div {
    div()
        .w_full()
        .grid()
        .grid_cols(columns.max(1))
        .gap(px(p.tokens.space.lg))
        .items_start()
}
