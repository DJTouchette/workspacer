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

/// r6-decisions (R6.3): a waiting tile that has the keyboard focus. One
/// accent per card: the focus ring (from [`tile_frame`]) carries it, so
/// the attention bar steps aside; the content keeps its inset, so nothing
/// moves as the focus arrives.
pub(crate) fn waiting_tile_focused(p: &Palette) -> Div {
    let t = p.tokens;
    tile(p)
        .relative()
        .border_color(rgb(p.roles.border_strong))
        .pl(px(t.space.xl + t.signals().attention_bar))
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

/// R7.6: `cards` in rows of `columns` equal columns, `space.lg` apart, each
/// card at its own height (top-aligned): a row without a decision does
/// not stretch to match one with it. Rows of flex cells rather than a CSS
/// grid, so wrapped text in a tall card lays out in place.
pub(crate) fn tile_rows(columns: usize, cards: Vec<gpui::AnyElement>, p: &Palette) -> Div {
    let columns = columns.max(1);
    let gap = px(p.tokens.space.lg);
    let mut rows = Vec::new();
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let row: Vec<gpui::AnyElement> = cards.by_ref().take(columns).collect();
        let empty = columns - row.len();
        rows.push(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(gap)
                .children(
                    row.into_iter()
                        .map(|card| div().flex_1().min_w_0().child(card)),
                )
                .children((0..empty).map(|_| div().flex_1().min_w_0())),
        );
    }
    div().w_full().flex().flex_col().gap(gap).children(rows)
}
