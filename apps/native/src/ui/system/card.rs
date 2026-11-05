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

/// r6-decisions (R6.3): a waiting tile that has the keyboard focus.
/// r8-decisions (R8.4): focus changes only weight, never which parts show:
/// the attention bar stays, the focus ring (from [`tile_frame`]) is added
/// around it, and the content keeps its inset, so nothing moves.
pub(crate) fn waiting_tile_focused(p: &Palette) -> Div {
    waiting_tile(p)
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

/// r8-decisions (R8.5): `cards` in `columns` equal columns, `space.lg`
/// apart, flowing down each column (masonry): card `i` goes to column
/// `i % columns`, so reading order is still row by row, and a tall card
/// pushes down only its own column. No holes beside a tall card.
pub(crate) fn tile_columns(columns: usize, cards: Vec<gpui::AnyElement>, p: &Palette) -> Div {
    let columns = columns.max(1);
    let gap = px(p.tokens.space.lg);
    let mut stacks: Vec<Vec<gpui::AnyElement>> = (0..columns).map(|_| Vec::new()).collect();
    for (ix, card) in cards.into_iter().enumerate() {
        stacks[ix % columns].push(card);
    }
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(gap)
        .children(stacks.into_iter().map(|stack| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(gap)
                .children(stack)
        }))
}

/// r8-decisions (R8.5): a side sheet: a panel on the page's right edge,
/// full height, `system().overlay_width` wide, on the Overlay elevation,
/// over the page (which does not reflow). Mount it as the last child of a
/// `.relative()` page frame; give it a header and content. Close it with
/// its close button and Esc.
pub(crate) fn side_sheet(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(px(t.system().overlay_width))
        .max_w_full()
        .flex()
        .flex_col()
        .gap(px(t.space.lg))
        .p(px(t.space.xl))
        .bg(rgb(p.surface))
        .border_l(px(t.stroke.hairline))
        .border_color(rgb(p.roles.border_strong))
        .shadow(super::Level::Overlay.shadow(p))
        .occlude()
}
