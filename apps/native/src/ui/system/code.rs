//! r7-pages (R7.8): the inline-code chip, from tokens. A wash of the text
//! colour (`wash.tone_rest`) over whatever it sits on, so the chip reads on
//! the canvas, a card and a decision alike, in every theme. The older
//! opaque `code_inline` fill nearly matched the dark card surface (the
//! approval title's `cargo test` read as stray spaces).
use super::super::primitives::tonal;
use super::*;

/// The inline-code chip's fill (prose, decision headlines, saved rules).
pub(crate) fn inline_code_bg(p: &Palette) -> gpui::Hsla {
    tonal(p, p.tokens.wash.tone_rest)
}
