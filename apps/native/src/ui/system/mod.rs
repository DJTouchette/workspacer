//! r5-system (R5.1–R5.3): the design system. `docs/design-system.md` is the
//! spec; this module and [`super::primitives`] are its only implementation.
//!
//! - **Foundations** live in `wks_native::tokens` (scale, spacing, radii,
//!   elevation, motion; `Tokens::system()` for the measures added here) and
//!   `wks_native::appearance` (colour roles per theme).
//! - **Components** are free functions and small builders that take a
//!   [`Palette`] and return a `Div` / `Stateful<Div>`; callers add
//!   `.on_click(cx.listener(…))`, children and `.debug_selector(…)`.
//!   The older ones stay in `primitives` (Button, Table, Card, CountBadge,
//!   Keycap…); the ones added in R5 live here, one file each.
//! - **Enforcement**: [`gallery`] renders every component in every state
//!   (`Screen::Gallery`, `wks-native --gallery` in dev builds), and two tests
//!   keep surfaces honest: `contrast` (every component state in every theme)
//!   and `lint` (no raw colours, px literals, shadows or border colours in
//!   surface files outside this layer).
#![allow(dead_code)]

use super::*;
use gpui::AnyElement;

pub(crate) mod actions;
mod card;
mod code;
mod composer;
mod edge;
mod elevation;
pub(crate) mod gallery;
mod icon_button;
mod menu;
mod nav;
mod overlay;
mod page;
mod segmented;
mod status_cell;
mod table;
mod toggle;

#[cfg(test)]
mod contrast;
#[cfg(test)]
mod lint;

pub(crate) use actions::{composer_send_state, send_state, stop_variant};
pub(crate) use card::{tile, tile_frame, tile_grid, tile_rows, waiting_tile, waiting_tile_focused};
pub(crate) use code::inline_code_bg;
pub(crate) use composer::{
    ComposerState, access_chip, access_option, access_tone, composer_body, composer_field,
    composer_reply_line, composer_reply_link, composer_specimen, composer_text, composer_toolbar,
    composer_width, decision_keys_hint,
};
pub(crate) use edge::{EdgeTone, Side, edge, floating, framed, lifted};
pub(crate) use elevation::{Level, elevated, scrim_color};
pub(crate) use icon_button::IconButton;
pub(crate) use menu::{MenuAnchor, anchored_menu};
pub(crate) use nav::NavItem;
pub(crate) use overlay::{Placement, overlay};
pub(crate) use page::{PageWidth, page_shell, page_shell_fill};
pub(crate) use segmented::{Segment, segmented_control};
pub(crate) use status_cell::{StatusCell, verification_line};
pub(crate) use table::{TablePart, table, table_band, table_head, table_part, table_row};
// The re-exports are the system's API: some are only used by the
// migration still to come (R5.4) or by the tests.
#[allow(unused_imports)]
pub(crate) use toggle::{ToggleColors, toggle, toggle_colors};

/// `color` (with its alpha) laid over the opaque `base`, as 0xRRGGBB: what a
/// translucent wash or fill looks like on the surface it sits on.
pub(crate) fn composite(color: gpui::Hsla, base: u32) -> u32 {
    let c = gpui::Rgba::from(color);
    let ch = |shift: u32, v: f32| {
        let b = ((base >> shift) & 0xff) as f32 / 255.;
        (((v * c.a + b * (1. - c.a)) * 255.).round().clamp(0., 255.) as u32) << shift
    };
    ch(16, c.r) | ch(8, c.g) | ch(0, c.b)
}
