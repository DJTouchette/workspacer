//! R5.4 Edges: hairline dividers and rules, in one place. A surface never
//! picks a border colour itself: it asks for a side and a [`EdgeTone`].
//!
//! ```ignore
//! edge(div(), Side::Bottom, EdgeTone::Divider, &p)  // the header strip's rule
//! edge(div(), Side::Left, EdgeTone::Quote, &p)      // a quoted reply's rule
//! ```
use super::*;

/// Which sides carry the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    All,
    Top,
    Bottom,
    Left,
    Right,
}

/// What the edge is for (its colour and weight).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeTone {
    /// A hairline divider between regions or rows (`border`).
    Divider,
    /// A Raised card's hairline (`card_border`, [`super::Level::Raised`]).
    Card,
    /// An input's or Overlay's hairline (`border_strong`).
    Strong,
    /// The rule beside quoted text or a guide (`border`, `stroke.focus`).
    Quote,
}

impl EdgeTone {
    pub(crate) fn color(self, p: &Palette) -> u32 {
        match self {
            EdgeTone::Divider | EdgeTone::Quote => p.border,
            EdgeTone::Card => p.roles.card_border,
            EdgeTone::Strong => p.roles.border_strong,
        }
    }

    fn width(self, p: &Palette) -> f32 {
        match self {
            EdgeTone::Quote => p.tokens.stroke.focus,
            _ => p.tokens.stroke.hairline,
        }
    }
}

/// `el` with an edge of `tone` on `side`.
pub(crate) fn edge<E: Styled>(el: E, side: Side, tone: EdgeTone, p: &Palette) -> E {
    let w = px(tone.width(p));
    let el = match side {
        Side::All => el.border(w),
        Side::Top => el.border_t(w),
        Side::Bottom => el.border_b(w),
        Side::Left => el.border_l(w),
        Side::Right => el.border_r(w),
    };
    el.border_color(rgb(tone.color(p)))
}

/// A Raised shadow-less frame for `el`: [`EdgeTone::Card`] on every side.
/// Use for a recessed or bordered block (code, a file preview) that is not
/// a card of its own.
pub(crate) fn framed<E: Styled>(el: E, p: &Palette) -> E {
    edge(el, Side::All, EdgeTone::Card, p)
}

/// `el` with the floating shadow (the composer, Jump to latest): above the
/// transcript, below an Overlay.
pub(crate) fn floating<E: Styled>(el: E, p: &Palette) -> E {
    el.shadow(super::super::primitives::elevation(
        p,
        p.tokens.elevation.floating,
    ))
}

/// `el` with `level`'s shadow alone ([`super::Level::shadow`]), for a
/// surface that draws its own fill and edge.
pub(crate) fn lifted<E: Styled>(el: E, level: super::Level, p: &Palette) -> E {
    el.shadow(level.shadow(p))
}
