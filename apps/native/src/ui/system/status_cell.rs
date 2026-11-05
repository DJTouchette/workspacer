//! R5.1 StatusCell: a status in a table, list or card. A fixed glyph slot
//! (`system().glyph_slot`, 16) then the word, then an optional detail.
//! **Every** state has a glyph, healthy ones included (Ready, Done), and
//! Idle is a hollow ring, so labels in a column always share one left edge.
use super::super::primitives::{StatusKind, status_mark};
use super::*;

/// A status cell. `StatusCell::new(kind).label("Accepting work")
/// .detail("3 agents").render(&p)`.
#[derive(Clone, Debug)]
pub(crate) struct StatusCell {
    kind: StatusKind,
    label: Option<SharedString>,
    detail: Option<SharedString>,
}

impl StatusCell {
    pub(crate) fn new(kind: StatusKind) -> Self {
        Self {
            kind,
            label: None,
            detail: None,
        }
    }

    /// A more specific word than the kind's ("Accepting work", "Waiting to
    /// start"); the glyph and colour stay the kind's.
    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A tertiary fact after the word ("· 2m", "exit code 101").
    pub(crate) fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Div {
        let t = p.tokens;
        let slot = t.system().glyph_slot;
        div()
            .debug_selector(move || format!("status-cell-{:?}", self.kind))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .text_size(px(t.text.meta))
            .child(
                div()
                    .size(px(slot))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(status_mark(self.kind, t.signals().glyph, p)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(self.kind.text_color(p)))
                    .child(self.label.unwrap_or_else(|| self.kind.label().into())),
            )
            .when_some(self.detail, |d, detail| {
                d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(detail),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::primitives::{STATUS_TABLE, StatusKind};

    // r6-surfaces (R6.2)
    #[test]
    fn the_status_table_has_one_row_per_state_each_with_its_own_glyph() {
        assert_eq!(STATUS_TABLE.len(), StatusKind::ALL.len());
        for (row, kind) in STATUS_TABLE.iter().zip(StatusKind::ALL) {
            assert_eq!(row.kind, kind, "the table is in StatusKind::ALL order");
            assert_eq!(kind.row().kind, kind);
        }
        for (i, a) in STATUS_TABLE.iter().enumerate() {
            for b in &STATUS_TABLE[i + 1..] {
                assert_ne!(
                    a.shape, b.shape,
                    "{:?} and {:?} share a glyph",
                    a.kind, b.kind
                );
                assert_ne!(
                    a.label, b.label,
                    "{:?} and {:?} share a word",
                    a.kind, b.kind
                );
            }
        }
    }
}
