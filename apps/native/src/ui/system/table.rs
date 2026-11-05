//! r7-pages (R7.6): the one table style. History, Jobs, Machines (both of its
//! tables) and the Fleet table draw their rows through these, so every dense
//! list in the app is the same object: the card surface with a hairline
//! edge and the panel radius, a header row on `base` (12/600, secondary),
//! then body rows `pages().row` tall with a hairline above each. Group rows
//! ([`table_band`]) sit inside the same frame, never as floating headings
//! between separate tables.
//!
//! A table built as one element uses [`table`]. A table whose rows must be
//! the page scroller's own children (History, so the keyboard cursor can
//! scroll a row into view) draws the same frame in slices: wrap each item
//! in [`table_part`] with its [`TablePart`].
use super::super::primitives::{Col, page_cell, table_style};
use super::*;

/// Where a slice of a list-built table sits in its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TablePart {
    /// The only slice: the whole frame.
    Whole,
    /// The first slice (the header row): top edge and top corners.
    Head,
    /// A middle slice: the side edges only.
    Body,
    /// The last slice: bottom edge and bottom corners.
    Foot,
}

impl TablePart {
    /// The part of item `ix` in a table of `len` items.
    pub(crate) fn of(ix: usize, len: usize) -> Self {
        match (ix, len) {
            (_, 0 | 1) => Self::Whole,
            (0, _) => Self::Head,
            (ix, len) if ix + 1 == len => Self::Foot,
            _ => Self::Body,
        }
    }
}

/// The table's frame: the card surface, a hairline edge, the panel radius,
/// clipped so the header's fill follows the corners.
pub(crate) fn table(p: &Palette) -> Div {
    table_part(TablePart::Whole, p)
}

/// One slice of the frame (see the module docs). Every slice is the full
/// width with the same side edges, so stacked slices read as one table.
pub(crate) fn table_part(part: TablePart, p: &Palette) -> Div {
    let t = p.tokens;
    let r = px(t.radius.panel);
    let line = px(t.stroke.hairline);
    let top = matches!(part, TablePart::Whole | TablePart::Head);
    let bottom = matches!(part, TablePart::Whole | TablePart::Foot);
    div()
        .w_full()
        .flex()
        .flex_col()
        .bg(rgb(p.surface))
        .border_l(line)
        .border_r(line)
        .when(top, |d| d.border_t(line).rounded_t(r))
        .when(bottom, |d| d.border_b(line).rounded_b(r))
        .border_color(rgb(p.roles.card_border))
        .overflow_hidden()
}

/// The header row: 12/600 secondary labels on `base`, the same columns as
/// the rows. It is the frame's first child, so it draws no edge above.
pub(crate) fn table_head(columns: &[(&'static str, Col)], p: &Palette) -> Div {
    let s = table_style(p);
    let t = p.tokens;
    table_row(p)
        .h(px(t.layout.row_height))
        .bg(rgb(s.head_bg))
        .border_t(px(0.))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(s.head_fg))
        .children(
            columns
                .iter()
                .map(|(label, col)| page_cell(*col).child(*label)),
        )
}

/// A body row: `pages().row` tall at least, cells at the table's padding
/// and the row gap, a hairline above. Add `.id(…)` for hover or clicks
/// (hover fill: `roles.hover`; the keyboard cursor's row: `selected`).
pub(crate) fn table_row(p: &Palette) -> Div {
    let s = table_style(p);
    let t = p.tokens;
    div()
        .w_full()
        .min_h(px(t.pages().row))
        .px(px(s.cell_x))
        .flex()
        .items_center()
        .gap(px(t.space.lg))
        .border_t(px(t.stroke.hairline))
        .border_color(rgb(s.divider))
        .text_size(px(t.text.body))
        .text_color(rgb(p.text))
}

/// A group row inside the frame (History's project, a project on other
/// machines): a body row on `base` whose name is 14/600. The rows of the
/// group follow it in the same frame.
pub(crate) fn table_band(p: &Palette) -> Div {
    let t = p.tokens;
    table_row(p)
        .bg(rgb(table_style(p).head_bg))
        .gap(px(t.space.md))
        .font_weight(FontWeight(t.weight.semibold))
}

#[cfg(test)]
mod tests {
    use super::TablePart;

    #[test]
    fn slices_close_the_frame_at_both_ends() {
        assert_eq!(TablePart::of(0, 1), TablePart::Whole);
        assert_eq!(TablePart::of(0, 3), TablePart::Head);
        assert_eq!(TablePart::of(1, 3), TablePart::Body);
        assert_eq!(TablePart::of(2, 3), TablePart::Foot);
    }
}
