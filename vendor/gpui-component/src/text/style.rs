use std::sync::Arc;

use gpui::{App, Hsla, Pixels, Point, Rems, SharedString, StyleRefinement, Window, px, rems};

/// Application-owned link routing, invoked for clicks rather than selections.
pub type LinkClickFn = dyn Fn(&str, &mut Window, &mut App) + Send + Sync + 'static;

/// Application hover reporting (Workspacer patch): `Some(url)` when the
/// pointer enters a link, `None` when it leaves. The first argument identifies
/// the reporting text run, so a late `None` from the run the pointer left
/// cannot clear a link another run just reported.
pub type LinkHoverFn = dyn Fn(usize, Option<&str>, &mut Window, &mut App) + Send + Sync + 'static;

/// Application context menu for a right-clicked link (Workspacer patch), at
/// the window position of the click. Without it right-clicks fall through.
pub type LinkMenuFn = dyn Fn(&str, Point<Pixels>, &mut Window, &mut App) + Send + Sync + 'static;

/// Where text handed to [`LinkifyFn`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkifySource {
    /// A run of paragraph, heading, list or table text outside a link.
    Text,
    /// One whole inline code span.
    InlineCode,
    /// The full text of a fenced or indented code block.
    CodeBlock,
}

/// Application-owned link detection for Markdown text outside links: byte
/// ranges of `text` that become links, each with its URL (often the covered
/// text itself). Runs during the (background) parse.
pub type LinkifyFn =
    dyn Fn(&str, LinkifySource) -> Vec<(std::ops::Range<usize>, String)> + Send + Sync + 'static;

use crate::highlighter::HighlightTheme;

/// Application chat colors. When set, emphasis, inline code, list markers,
/// headings, rules and fenced-code headers use these instead of the upstream
/// look; `None` preserves upstream rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProseColors {
    /// Bold, italic and heading text.
    pub strong: Hsla,
    pub code: Hsla,
    pub code_background: Hsla,
    /// Unordered bullets.
    pub marker: Hsla,
    /// Ordered numbers and fenced-code language labels.
    pub muted: Hsla,
    /// Heading underlines and horizontal rules.
    pub rule: Hsla,
    /// Fenced-code border and header separator.
    pub border: Hsla,
    pub code_header: Hsla,
}

/// TextViewStyle used to customize the style for [`TextView`].
#[derive(Clone)]
pub struct TextViewStyle {
    pub on_link_click: Option<Arc<LinkClickFn>>,
    /// Turns detected spans of plain text and inline code into links.
    pub linkify: Option<Arc<LinkifyFn>>,
    pub on_link_hover: Option<Arc<LinkHoverFn>>,
    pub on_link_menu: Option<Arc<LinkMenuFn>>,
    pub unordered_list_marker: Option<SharedString>,
    pub prose: Option<ProseColors>,
    /// Gap of each paragraphs, default is 1 rem.
    pub paragraph_gap: Rems,
    /// Base font size for headings, default is 14px.
    pub heading_base_font_size: Pixels,
    /// Function to calculate heading font size based on heading level (1-6).
    ///
    /// The first parameter is the heading level (1-6), the second parameter is the base font size.
    /// The second parameter is the base font size.
    pub heading_font_size: Option<Arc<dyn Fn(u8, Pixels) -> Pixels + Send + Sync + 'static>>,
    /// Highlight theme for code blocks. Default: [`HighlightTheme::default_light()`]
    pub highlight_theme: Arc<HighlightTheme>,
    /// The style refinement for code blocks.
    pub code_block: StyleRefinement,
    pub is_dark: bool,
    /// Prose mode only: the font family for inline code spans and their
    /// table-width measurement. `None` uses the theme mono font. GPUI shapes a
    /// line at one size, so a smaller-looking inline code font is a family
    /// whose glyphs are drawn smaller (Workspacer patch).
    pub inline_code_family: Option<SharedString>,
}

impl PartialEq for TextViewStyle {
    fn eq(&self, other: &Self) -> bool {
        self.paragraph_gap == other.paragraph_gap
            && self.unordered_list_marker == other.unordered_list_marker
            && self.prose == other.prose
            && self.heading_base_font_size == other.heading_base_font_size
            && self.highlight_theme == other.highlight_theme
            && self.inline_code_family == other.inline_code_family
            // Identity is excluded like `on_link_click`; turning it on or off reparses.
            && self.linkify.is_some() == other.linkify.is_some()
    }
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            on_link_click: None,
            linkify: None,
            on_link_hover: None,
            on_link_menu: None,
            unordered_list_marker: None,
            prose: None,
            paragraph_gap: rems(1.),
            heading_base_font_size: px(14.),
            heading_font_size: None,
            highlight_theme: HighlightTheme::default_light().clone(),
            code_block: StyleRefinement::default(),
            is_dark: false,
            inline_code_family: None,
        }
    }
}

impl TextViewStyle {
    /// Set paragraph gap, default is 1 rem.
    pub fn paragraph_gap(mut self, gap: Rems) -> Self {
        self.paragraph_gap = gap;
        self
    }

    pub fn heading_font_size<F>(mut self, f: F) -> Self
    where
        F: Fn(u8, Pixels) -> Pixels + Send + Sync + 'static,
    {
        self.heading_font_size = Some(Arc::new(f));
        self
    }

    /// Set style for code blocks.
    pub fn code_block(mut self, style: StyleRefinement) -> Self {
        self.code_block = style;
        self
    }
}
