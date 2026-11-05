//! R5.1 PageShell. r7-pages (R7.6): one page template.
//!
//! Every top-level page (Projects, History, Jobs, Machines, Scratch,
//! Settings, Agent setup) and every drill-in page draws the same header in
//! the same place: the title and its one-line purpose at the page's left
//! edge, the actions in one slot at the page's right edge, at the same top
//! offset and gutter (`Workspace::page_view`). Only the *body* has a width
//! rule, and there are two:
//!
//! - **Form** ([`PageWidth::Form`]): `pages().form` (880), left-aligned under
//!   the title. Settings, Agent setup, Session details, Model, Handoff,
//!   Pull request.
//! - **Full width** ([`PageWidth::Wide`]): tables and lists fill the page
//!   (`pages().wide`). Projects, Jobs, History, Machines, Scratch, Changes.
//!   Fleet's tile grid is the same template at the grid's cap
//!   ([`PageWidth::Grid`]).
//!
//! New session is not a page of this kind: it is the welcome's centred
//! composer ([`PageWidth::Composer`]), at the one composer width.
use super::*;

/// The width rule of a page's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageWidth {
    /// Tables, grids and lists: the full page, `pages().wide` (1240).
    Wide,
    /// A form: `pages().form` (880), left-aligned under the full-width
    /// header, so the title sits where every other page's title sits.
    Form,
    /// The table template at Fleet's tile-grid cap, `fleet().grid_max`
    /// (1440), so its header sits on the grid's edge (r5-shell).
    Grid,
    /// New session: the welcome's centred column at the one composer width
    /// ([`super::composer_width`]), header and body alike.
    Composer,
}

impl PageWidth {
    /// The body's cap.
    pub(crate) fn max(self, p: &Palette) -> f32 {
        match self {
            Self::Wide => p.tokens.pages().wide,
            Self::Form => p.tokens.pages().form,
            Self::Grid => p.tokens.fleet().grid_max,
            Self::Composer => super::composer_width(p),
        }
    }

    /// The header's cap: the page's frame. Form bodies sit under a header
    /// as wide as a table page's, so titles and actions never move between
    /// pages.
    pub(crate) fn frame(self, p: &Palette) -> f32 {
        match self {
            Self::Wide | Self::Form => p.tokens.pages().wide,
            Self::Grid | Self::Composer => self.max(p),
        }
    }
}

/// The page column: `header` (a `primitives::page_header`), an optional
/// notice, then `body`, `space.xxl` apart. The header and notice take the
/// frame's width ([`PageWidth::frame`]); the body takes its own cap
/// ([`PageWidth::max`]), left-aligned. Pass the result to
/// `page_view(id, pages().wide, false, …)`. No page adds its own top
/// offset: every page header starts at the same y.
pub(crate) fn page_shell(
    header: impl IntoElement,
    notice: Option<AnyElement>,
    body: impl IntoElement,
    width: PageWidth,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    // The column has a definite width (the frame), capped by its parent
    // when the parent's is definite: inside a column sized by its content
    // a bare `w_full` would collapse, and with it the header's own
    // `w_full` (title left, actions right). Every page gets this, not
    // only the ones that patch it at the call site.
    let column = div()
        .debug_selector(|| "page-shell".into())
        .w(px(width.frame(p)))
        .max_w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(t.space.xxl))
        .child(div().w_full().flex().flex_col().child(header))
        .children(notice)
        .child(
            div()
                .debug_selector(|| "page-body".into())
                .w_full()
                .max_w(px(width.max(p)))
                .flex()
                .flex_col()
                .child(body),
        );
    // The composer column is centred in the page, like the welcome.
    if width == PageWidth::Composer {
        div().w_full().flex().justify_center().child(column)
    } else {
        column
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn every_page_header_shares_one_frame() {
        let p = Appearance::ALL[0].palette_for(Design::V2);
        // r7-pages (R7.6): forms and tables put their titles and actions in
        // the same place; only the body's width differs.
        assert_eq!(PageWidth::Form.frame(&p), PageWidth::Wide.frame(&p));
        assert_eq!(PageWidth::Form.max(&p), 880.);
        assert_eq!(PageWidth::Composer.max(&p), PageWidth::Form.max(&p));
    }
}
