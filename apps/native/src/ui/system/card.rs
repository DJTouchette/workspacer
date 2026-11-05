//! R5.1 Card / Tile: a card is as tall as its content. Tiles in a grid
//! keep their natural height (top-aligned), so a short tile is never a
//! mostly empty box with its content pinned to the bottom.
use super::*;

/// A tile: the Raised card ([`super::super::primitives::card`]) at its
/// natural height. Put it in a [`tile_grid`].
pub(crate) fn tile(p: &Palette) -> Div {
    super::super::primitives::card(p).h_auto()
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
