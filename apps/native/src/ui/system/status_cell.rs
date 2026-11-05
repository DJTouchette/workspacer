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

/// r7-status (R7.3): the verification line, for decision cards and Fleet
/// tiles: the latest test run as one StatusCell ("✕ Tests failed 4m ago ·
/// cargo test"), red when it failed. A pass claimed after the failure adds
/// "the reply says they pass" to the detail. Selector:
/// `verification-line`. Get the data from `Workspace::session_verification`
/// (or `status::verification` for a task list).
pub(crate) fn verification_line(
    v: &super::super::status::Verification,
    now: i64,
    p: &Palette,
) -> Div {
    let detail = wks_native::format::meta([
        v.command.as_str(),
        if v.contradicted {
            super::super::status::CONTRADICTED
        } else {
            ""
        },
    ]);
    StatusCell::new(v.kind())
        .label(v.headline(now))
        .detail(detail)
        .render(p)
        .debug_selector(|| "verification-line".into())
}

/// r7-qa (R7.8, one join helper): a meta line whose parts are elements (a
/// status badge, a link, plain words), joined by the same "·" that
/// `wks_native::format::meta` puts between strings, never by bare gaps. The
/// caller sets the text size and colour; the dots take the tertiary tone.
pub(crate) fn meta_row(parts: impl IntoIterator<Item = AnyElement>, p: &Palette) -> Div {
    let t = p.tokens;
    let mut row = div().flex().items_center().gap(px(t.space.sm)).min_w_0();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(p.roles.text_tertiary))
                    .child("·"),
            );
        }
        row = row.child(part);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::super::super::primitives::{MAX_STATUS_GLYPHS, STATUS_TABLE, StatusKind};

    // r6-surfaces (R6.2), r7-status (R7.3)
    #[test]
    fn the_status_table_has_one_row_per_state_and_at_most_seven_glyphs() {
        assert_eq!(STATUS_TABLE.len(), StatusKind::ALL.len());
        for (row, kind) in STATUS_TABLE.iter().zip(StatusKind::ALL) {
            assert_eq!(row.kind, kind, "the table is in StatusKind::ALL order");
            assert_eq!(kind.row().kind, kind);
            assert!(kind.has_glyph(), "{kind:?} draws its glyph everywhere");
        }
        let distinct = STATUS_TABLE
            .iter()
            .enumerate()
            .filter(|(i, r)| STATUS_TABLE[..*i].iter().all(|o| o.shape != r.shape))
            .count();
        assert!(distinct <= MAX_STATUS_GLYPHS, "{distinct} glyphs");
        // A glyph is shared only within one colour: the word tells the
        // states apart, the shape and hue never contradict.
        for (i, a) in STATUS_TABLE.iter().enumerate() {
            for b in &STATUS_TABLE[i + 1..] {
                if a.shape == b.shape {
                    assert_eq!(a.tone, b.tone, "{:?} and {:?}", a.kind, b.kind);
                }
                // Needs you and Ready for review never share a glyph.
                if matches!(a.kind, StatusKind::NeedsYou | StatusKind::Review) {
                    assert_ne!(a.shape, b.shape, "{:?} and {:?}", a.kind, b.kind);
                }
                assert_ne!(
                    a.label, b.label,
                    "{:?} and {:?} share a word",
                    a.kind, b.kind
                );
            }
        }
    }
}
