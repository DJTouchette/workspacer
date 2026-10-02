use std::sync::Arc;

use gpui::{App, Hsla, Pixels, Rems, SharedString, StyleRefinement, Window, px, rems};

/// Application-owned link routing, invoked for clicks rather than selections.
pub type LinkClickFn = dyn Fn(&str, &mut Window, &mut App) + Send + Sync + 'static;

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
}

impl PartialEq for TextViewStyle {
    fn eq(&self, other: &Self) -> bool {
        self.paragraph_gap == other.paragraph_gap
            && self.unordered_list_marker == other.unordered_list_marker
            && self.prose == other.prose
            && self.heading_base_font_size == other.heading_base_font_size
            && self.highlight_theme == other.highlight_theme
    }
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            on_link_click: None,
            unordered_list_marker: None,
            prose: None,
            paragraph_gap: rems(1.),
            heading_base_font_size: px(14.),
            heading_font_size: None,
            highlight_theme: HighlightTheme::default_light().clone(),
            code_block: StyleRefinement::default(),
            is_dark: false,
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
