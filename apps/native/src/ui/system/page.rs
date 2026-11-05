//! R5.1 PageShell. r6-surfaces (R6.5): exactly two page templates.
//!
//! - **Centred form** ([`PageWidth::Form`]): Settings, Agent setup, New
//!   session and every drill-in form (Session details, Model, Handoff,
//!   Pull request), one `pages().form` column centred in the page.
//! - **Full-width table** ([`PageWidth::Wide`]): Projects, Jobs, History,
//!   Machines, Scratch and Changes, at `pages().wide`. Fleet's tile grid is
//!   the same template with the grid's wider cap ([`PageWidth::Grid`]).
//!
//! The page scroller (`Workspace::page_view`) gives every page the same
//! top offset and side gutter; [`page_shell`] stacks the shared header over
//! the body and caps *both* at the template's width, so header actions sit
//! on the content edge.
use super::*;

/// The width rule of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageWidth {
    /// Tables, grids and lists: `pages().wide` (1240).
    Wide,
    /// The centred form: Settings, New session, Agent setup, Session
    /// details, Model, Handoff, Pull request. `pages().form` (880), centred.
    Form,
    /// The table template at Fleet's tile-grid cap, `fleet().grid_max`
    /// (1440), so its header sits on the grid's edge (r5-shell).
    Grid,
}

impl PageWidth {
    pub(crate) fn max(self, p: &Palette) -> f32 {
        match self {
            Self::Wide => p.tokens.pages().wide,
            Self::Form => p.tokens.pages().form,
            Self::Grid => p.tokens.fleet().grid_max,
        }
    }
}

/// The page column: `header` (a `primitives::page_header`), an optional
/// notice, then `body`, `space.xxl` apart, all within `width`. Pass the
/// result to `page_view(id, pages().wide, false, …)`. No page adds its own
/// top offset: every page header starts at the same y.
pub(crate) fn page_shell(
    header: impl IntoElement,
    notice: Option<AnyElement>,
    body: impl IntoElement,
    width: PageWidth,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    let column = div()
        .debug_selector(|| "page-shell".into())
        .w_full()
        .max_w(px(width.max(p)))
        .flex()
        .flex_col()
        .gap(px(t.space.xxl))
        .child(header)
        .children(notice)
        .child(body);
    // The form template is centred in the page; tables fill it.
    if width == PageWidth::Form {
        div().w_full().flex().justify_center().child(column)
    } else {
        column
    }
}
