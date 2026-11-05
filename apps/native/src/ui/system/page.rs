//! R5.1 PageShell: one template for every page. The page scroller
//! (`Workspace::page_view`) gives every page the same top offset and side
//! gutter; [`page_shell`] stacks the shared header over the body and caps
//! *both* at the page's width rule, so a narrow page's header actions sit
//! on its content edge (never 470px away at the window edge).
use super::*;

/// The width rule of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageWidth {
    /// Tables, grids and lists: `pages().wide` (1240).
    Wide,
    /// Forms (New session, Agent setup, Session details, Model, Handoff,
    /// Pull request): `pages().form` (760), left-aligned.
    Form,
}

impl PageWidth {
    pub(crate) fn max(self, p: &Palette) -> f32 {
        match self {
            Self::Wide => p.tokens.pages().wide,
            Self::Form => p.tokens.pages().form,
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
    div()
        .debug_selector(|| "page-shell".into())
        .w_full()
        .max_w(px(width.max(p)))
        .flex()
        .flex_col()
        .gap(px(t.space.xxl))
        .child(header)
        .children(notice)
        .child(body)
}
