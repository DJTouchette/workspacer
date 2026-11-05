//! R5.1 Elevation: three levels. Flat sits in the page; Raised is a card
//! resting on it; Overlay floats above everything (menus, popovers,
//! dialogs, the palette) with a shadow, and a centred overlay also gets a
//! scrim ([`super::overlay`]).
use super::super::primitives::{elevation, wash};
use super::*;
use gpui::{BoxShadow, Hsla};

/// One of the three elevation levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    /// In the page: no shadow, no fill of its own beyond `chat`.
    Flat,
    /// A card or table on the page: `surface`, the `card_border` hairline
    /// and the `raised` shadow.
    Raised,
    /// Above everything: `roles.raised`, the `border_strong` hairline and
    /// the `modal` shadow.
    Overlay,
}

impl Level {
    pub(crate) const ALL: [Self; 3] = [Self::Flat, Self::Raised, Self::Overlay];

    pub(crate) fn fill(self, p: &Palette) -> u32 {
        match self {
            Self::Flat => p.chat,
            Self::Raised => p.surface,
            Self::Overlay => p.roles.raised,
        }
    }

    pub(crate) fn edge(self, p: &Palette) -> u32 {
        match self {
            Self::Flat => p.border,
            Self::Raised => p.roles.card_border,
            Self::Overlay => p.roles.border_strong,
        }
    }

    pub(crate) fn shadow(self, p: &Palette) -> Vec<BoxShadow> {
        let e = p.tokens.elevation;
        match self {
            Self::Flat => Vec::new(),
            Self::Raised => elevation(p, e.raised),
            Self::Overlay => elevation(p, e.modal),
        }
    }
}

/// A surface at `level`: its fill, hairline edge, shadow and the panel
/// radius. Add padding and children.
pub(crate) fn elevated(level: Level, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .rounded(px(t.radius.panel))
        .bg(rgb(level.fill(p)))
        .border(px(t.stroke.hairline))
        .border_color(rgb(level.edge(p)))
        .shadow(level.shadow(p))
}

/// The scrim behind a centred overlay: the theme's shadow colour at
/// `elevation.scrim_alpha`.
pub(crate) fn scrim_color(p: &Palette) -> Hsla {
    wash(p.shadow, p.tokens.elevation.scrim_alpha)
}
