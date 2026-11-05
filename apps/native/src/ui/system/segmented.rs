//! r6-decisions (R6.7) SegmentedControl: one choice of a few, always
//! visible (Grid / Table, Needs you / Active / All, the Fleet's sort). The
//! selected segment reads as selected at a glance in every theme: the
//! `roles.segment_selected` fill (r6-surfaces: white on Light's grey
//! track, the lighter selected fill in dark themes), a `border_strong`
//! hairline, the raised shadow and the primary text, where the rest are
//! secondary text on the track (`wash.hover`). A count beside a label
//! follows its segment's text.
//!
//! ```ignore
//! system::segmented_control(&p)
//!     .child(system::Segment::new("fleet-filter-needs", "Needs you").count(3).selected(true).render(&p).on_click(…))
//! ```
use super::super::primitives::{elevation, tabular, tonal};
use super::*;

/// The track the segments sit in.
pub(crate) fn segmented_control(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.xxs))
        .p(px(t.space.xxs))
        .rounded(px(t.radius.control))
        .bg(tonal(p, t.wash.hover))
}

/// One segment: its label, an optional count, and whether it is the one
/// chosen. Add `.on_click`.
pub(crate) struct Segment {
    id: SharedString,
    label: SharedString,
    count: Option<usize>,
    selected: bool,
}

impl Segment {
    pub(crate) fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: None,
            selected: false,
        }
    }

    pub(crate) fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let selected = self.selected;
        let fg = if selected {
            p.text
        } else {
            p.roles.text_secondary
        };
        let hover = p.text;
        let hover_bg = tonal(p, t.wash.hover);
        div()
            .id(gpui::ElementId::Name(self.id.clone()))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(t.space.sm))
            .h(px(t.layout.control_height_sm - 2. * t.space.xxs))
            .px(px(t.space.lg))
            .rounded(px(t.radius.control - t.space.xxs))
            .border(px(t.stroke.hairline))
            .cursor_pointer()
            .text_size(px(t.text.meta))
            .text_color(rgb(fg))
            .when(selected, |d| {
                d.bg(rgb(p.roles.segment_selected))
                    .border_color(rgb(p.roles.border_strong))
                    .font_weight(FontWeight(t.weight.semibold))
                    .shadow(elevation(p, t.elevation.raised))
            })
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .font_weight(FontWeight(t.weight.medium))
                    .hover(move |s| s.bg(hover_bg).text_color(rgb(hover)))
            })
            .child(self.label)
            .children(self.count.map(|n| {
                tabular(
                    div()
                        .text_color(rgb(if selected {
                            p.text
                        } else {
                            p.roles.text_tertiary
                        }))
                        .child(super::super::primitives::count_text(n)),
                )
            }))
    }
}
