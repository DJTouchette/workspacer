use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, DefiniteLength, Div, Element, ElementId, FontStyle, FontWeight, Half,
    HighlightStyle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, Length, ListState,
    ObjectFit, ParentElement, Pixels, RenderOnce, ScrollHandle, SharedString, SharedUri,
    StatefulInteractiveElement, Styled, StyledImage as _, Window, canvas, div, img, point,
    prelude::FluentBuilder as _, px, relative, rems,
};
use markdown::mdast;
use ropey::Rope;

use crate::{
    ActiveTheme as _, Icon, IconName, StyledExt, h_flex,
    highlighter::{HighlightTheme, SyntaxHighlighter},
    scroll::{Scrollbar, ScrollbarShow},
    text::{
        CodeBlockActionsFn,
        inline::{Inline, InlineState},
    },
    tooltip::Tooltip,
    v_flex,
};

use super::{TextViewStyle, utils::list_item_prefix};

#[allow(unused)]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LinkMark {
    pub url: SharedString,
    /// Optional identifier for footnotes.
    pub identifier: Option<SharedString>,
    pub title: Option<SharedString>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TextMark {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<LinkMark>,
}

impl TextMark {
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    pub fn link(mut self, link: impl Into<LinkMark>) -> Self {
        self.link = Some(link.into());
        self
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<Span> for ElementId {
    fn from(value: Span) -> Self {
        ElementId::Name(format!("md-{}:{}", value.start, value.end).into())
    }
}

#[allow(unused)]
#[derive(Debug, Default, Clone)]
pub struct ImageNode {
    pub url: SharedUri,
    pub link: Option<LinkMark>,
    pub title: Option<SharedString>,
    pub alt: Option<SharedString>,
    pub width: Option<DefiniteLength>,
    pub height: Option<DefiniteLength>,
}

impl ImageNode {
    pub fn title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.alt.clone().unwrap_or_default())
            .to_string()
    }
}

impl PartialEq for ImageNode {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.link == other.link
            && self.title == other.title
            && self.alt == other.alt
            && self.width == other.width
            && self.height == other.height
    }
}

#[derive(Default, Clone, Debug)]
pub(crate) struct InlineNode {
    /// The text content.
    pub(crate) text: SharedString,
    pub(crate) image: Option<ImageNode>,
    /// The text styles, each tuple contains the range of the text and the style.
    pub(crate) marks: Vec<(Range<usize>, TextMark)>,

    state: Arc<Mutex<InlineState>>,
}

impl PartialEq for InlineNode {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.image == other.image && self.marks == other.marks
    }
}

impl InlineNode {
    pub(crate) fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            image: None,
            marks: vec![],
            state: Arc::new(Mutex::new(InlineState::default())),
        }
    }

    pub(crate) fn image(image: ImageNode) -> Self {
        let mut this = Self::new("");
        this.image = Some(image);
        this
    }

    pub(crate) fn marks(mut self, marks: Vec<(Range<usize>, TextMark)>) -> Self {
        self.marks = marks;
        self
    }
}

/// The paragraph element, contains multiple text nodes.
///
/// Unlike other Element, this is cloneable, because it is used in the Node AST.
/// We are keep the selection state inside this AST Nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paragraph {
    pub(super) span: Option<Span>,
    pub(super) children: Vec<InlineNode>,
    /// The link references in this paragraph, used for reference links.
    ///
    /// The key is the identifier, the value is the url.
    pub(super) link_refs: HashMap<SharedString, SharedString>,

    pub(crate) state: Arc<Mutex<InlineState>>,
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span
            && self.children == other.children
            && self.link_refs == other.link_refs
    }
}

impl Paragraph {
    pub(crate) fn new(text: String) -> Self {
        Self {
            span: None,
            children: vec![InlineNode::new(&text)],
            link_refs: HashMap::new(),
            state: Arc::new(Mutex::new(InlineState::default())),
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();

        for c in self.children.iter() {
            let state = c.state.lock().unwrap();
            if let Some(selection) = &state.selection {
                let part_text = state.text.clone();
                text.push_str(&part_text[selection.start..selection.end]);
            }
        }

        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let all_text = state.text.clone();
            text.push_str(&all_text[selection.start..selection.end]);
        }

        text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Table {
    pub children: Vec<TableRow>,
    pub column_aligns: Vec<ColumnumnAlign>,
}

impl Table {
    pub(crate) fn column_align(&self, index: usize) -> ColumnumnAlign {
        self.column_aligns.get(index).copied().unwrap_or_default()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub(crate) enum ColumnumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl From<mdast::AlignKind> for ColumnumnAlign {
    fn from(value: mdast::AlignKind) -> Self {
        match value {
            mdast::AlignKind::None => ColumnumnAlign::Left,
            mdast::AlignKind::Left => ColumnumnAlign::Left,
            mdast::AlignKind::Center => ColumnumnAlign::Center,
            mdast::AlignKind::Right => ColumnumnAlign::Right,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableRow {
    pub children: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableCell {
    pub children: Paragraph,
    pub width: Option<DefiniteLength>,
}

impl Paragraph {
    pub(crate) fn take(&mut self) -> Paragraph {
        std::mem::replace(
            self,
            Paragraph {
                span: None,
                children: vec![],
                link_refs: Default::default(),
                state: Arc::new(Mutex::new(InlineState::default())),
            },
        )
    }

    pub(crate) fn is_image(&self) -> bool {
        false
    }

    pub(crate) fn set_span(&mut self, span: Span) {
        self.span = Some(span);
    }

    pub(crate) fn push_str(&mut self, text: &str) {
        self.children.push(
            InlineNode::new(text.to_string()).marks(vec![(0..text.len(), TextMark::default())]),
        );
    }

    pub(crate) fn push(&mut self, text: InlineNode) {
        self.children.push(text);
    }

    pub(crate) fn push_image(&mut self, image: ImageNode) {
        self.children.push(InlineNode::image(image));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.children.is_empty()
            || self
                .children
                .iter()
                .all(|node| node.text.is_empty() && node.image.is_none())
    }

    /// Return length of children text.
    pub(crate) fn text_len(&self) -> usize {
        self.children
            .iter()
            .map(|node| node.text.len())
            .sum::<usize>()
    }

    /// Up to `count` of the longest whitespace-separated words, with the
    /// marks that set their font (for table column minimums).
    pub(crate) fn long_words(&self, header: bool, count: usize) -> Vec<StyledWord> {
        let mut words: Vec<StyledWord> = self
            .children
            .iter()
            .flat_map(|node| {
                let text = node.text.as_ref();
                text.split_whitespace().map(move |word| {
                    let start = word.as_ptr() as usize - text.as_ptr() as usize;
                    let mark = |f: fn(&TextMark) -> bool| {
                        node.marks
                            .iter()
                            .any(|(range, mark)| range.contains(&start) && f(mark))
                    };
                    StyledWord {
                        text: word.to_owned(),
                        header,
                        bold: mark(|m| m.bold),
                        italic: mark(|m| m.italic),
                        code: mark(|m| m.code),
                    }
                })
            })
            .collect();
        words.sort_by_key(|word| std::cmp::Reverse(word.text.chars().count()));
        words.truncate(count);
        words
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.children.extend(other.children);
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    lang: Option<SharedString>,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    state: Arc<Mutex<InlineState>>,
}

impl PartialEq for CodeBlock {
    fn eq(&self, other: &Self) -> bool {
        self.lang == other.lang && self.styles == other.styles
    }
}

impl CodeBlock {
    /// Get the language of the code block.
    pub fn lang(&self) -> Option<SharedString> {
        self.lang.clone()
    }

    /// Get the code content of the code block.
    pub fn code(&self) -> SharedString {
        self.state.lock().unwrap().text.clone()
    }

    pub(crate) fn new(
        code: SharedString,
        lang: Option<SharedString>,
        _: &TextViewStyle,
        highlight_theme: &HighlightTheme,
    ) -> Self {
        let mut styles = vec![];
        if let Some(lang) = &lang {
            let mut highlighter = SyntaxHighlighter::new(&lang);
            highlighter.update(None, &Rope::from_str(code.as_str()));
            styles = highlighter.styles(&(0..code.len()), highlight_theme);
        };

        let state = Arc::new(Mutex::new(InlineState::default()));
        state.lock().unwrap().set_text(code);

        Self {
            lang,
            styles,
            state,
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let part_text = state.text.clone();
            text.push_str(&part_text[selection.start..selection.end]);
        }
        text
    }

    fn render(
        &self,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let style = &node_cx.style;

        if let Some(prose) = style.prose {
            return div()
                .when(!options.is_last, |this| this.pb(style.paragraph_gap))
                .child(
                    div()
                        .id("codeblock")
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(prose.border)
                        .overflow_hidden()
                        .bg(cx.theme().muted)
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(cx.theme().mono_font_size)
                        .relative()
                        .refine_style(&style.code_block)
                        .when_some(self.lang.clone().filter(|l| !l.is_empty()), |this, lang| {
                            this.child(
                                div()
                                    .id("codeblock-language")
                                    .px_3()
                                    .py(px(3.))
                                    .bg(prose.code_header)
                                    .border_b_1()
                                    .border_color(prose.border)
                                    .text_size(cx.theme().mono_font_size * 0.85)
                                    .text_color(prose.muted)
                                    .child(lang),
                            )
                        })
                        .child(div().p_3().child(Inline::new(
                            "code",
                            self.state.clone(),
                            vec![],
                            self.styles.clone(),
                            style.on_link_click.clone(),
                        )))
                        .when_some(node_cx.code_block_actions.clone(), |this, actions| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_1()
                                    .right_2()
                                    .child(actions(&self, window, cx)),
                            )
                        }),
                )
                .into_any_element();
        }

        div()
            .when(!options.is_last, |this| this.pb(style.paragraph_gap))
            .child(
                div()
                    .id("codeblock")
                    .p_3()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .relative()
                    .refine_style(&style.code_block)
                    .child(Inline::new(
                        "code",
                        self.state.clone(),
                        vec![],
                        self.styles.clone(),
                        style.on_link_click.clone(),
                    ))
                    .when_some(node_cx.code_block_actions.clone(), |this, actions| {
                        this.child(
                            div()
                                .absolute()
                                .top_2()
                                .right_2()
                                .bg(cx.theme().muted)
                                .rounded(cx.theme().radius)
                                .child(actions(&self, window, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// A context for rendering nodes, contains link references.
#[derive(Default, Clone)]
pub(crate) struct NodeContext {
    pub(crate) link_refs: HashMap<SharedString, LinkMark>,
    pub(crate) style: TextViewStyle,
    pub(crate) code_block_actions: Option<Arc<CodeBlockActionsFn>>,
}

impl NodeContext {
    pub(super) fn add_ref(&mut self, identifier: SharedString, link: LinkMark) {
        self.link_refs.insert(identifier, link);
    }
}

impl PartialEq for NodeContext {
    fn eq(&self, other: &Self) -> bool {
        self.link_refs == other.link_refs && self.style == other.style
        // Note: code_block_buttons is intentionally not compared (closures can't be compared)
    }
}

/// The AST Node of the rich text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Root {
        children: Vec<Node>,
    },
    Paragraph(Paragraph),
    Heading {
        level: u8,
        children: Paragraph,
    },
    Blockquote {
        children: Vec<Node>,
    },
    List {
        /// Only contains ListItem, others will be ignored
        children: Vec<Node>,
        ordered: bool,
    },
    ListItem {
        children: Vec<Node>,
        spread: bool,
        /// Whether the list item is checked, if None, it's not a checkbox
        checked: Option<bool>,
    },
    CodeBlock(CodeBlock),
    Table(Table),
    Break {
        html: bool,
    },
    Divider,
    /// Use for to_markdown get raw definition
    Definition {
        identifier: SharedString,
        url: SharedString,
        title: Option<SharedString>,
    },
    Unknown,
}

impl Node {
    pub(super) fn is_list_item(&self) -> bool {
        matches!(self, Self::ListItem { .. })
    }

    pub(super) fn is_break(&self) -> bool {
        matches!(self, Self::Break { .. })
    }

    /// Combine all children, omitting the empt parent nodes.
    pub(super) fn compact(self) -> Node {
        match self {
            Self::Root { mut children } if children.len() == 1 => children.remove(0).compact(),
            _ => self,
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        match self {
            Node::Root { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Paragraph(paragraph) => {
                let mut block_text = String::new();
                block_text.push_str(&paragraph.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Heading { children, .. } => {
                let mut block_text = String::new();
                block_text.push_str(&children.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::List { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::ListItem { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::Blockquote { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Table(table) => {
                let mut block_text = String::new();
                for row in table.children.iter() {
                    let mut row_texts = vec![];
                    for cell in row.children.iter() {
                        row_texts.push(cell.children.selected_text());
                    }
                    if !row_texts.is_empty() {
                        block_text.push_str(&row_texts.join(" "));
                        block_text.push('\n');
                    }
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::CodeBlock(code_block) => {
                let block_text = code_block.selected_text();
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Definition { .. } | Node::Break { .. } | Node::Divider | Node::Unknown => {}
        }

        text
    }
}

impl Paragraph {
    fn render(
        &self,
        node_cx: &NodeContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let span = self.span;
        let children = &self.children;

        let mut child_nodes: Vec<AnyElement> = vec![];

        let mut text = String::new();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = vec![];
        let mut links: Vec<(Range<usize>, LinkMark)> = vec![];
        let mut code_ranges: Vec<Range<usize>> = vec![];
        let mono = node_cx.style.prose.map(|_| {
            node_cx
                .style
                .inline_code_family
                .clone()
                .unwrap_or_else(|| cx.theme().mono_font_family.clone())
        });
        let code_background = node_cx.style.prose.map(|prose| prose.code_background);
        let mut offset = 0;

        let mut ix = 0;
        for inline_node in children {
            let text_len = inline_node.text.len();
            text.push_str(&inline_node.text);

            if let Some(image) = &inline_node.image {
                if text.len() > 0 {
                    inline_node
                        .state
                        .lock()
                        .unwrap()
                        .set_text(text.clone().into());
                    child_nodes.push(
                        Inline::new(
                            ix,
                            inline_node.state.clone(),
                            links.clone(),
                            highlights.clone(),
                            node_cx.style.on_link_click.clone(),
                        )
                        .mono(mono.clone(), code_background, code_ranges.clone())
                        .into_any_element(),
                    );
                }
                // An app-routed view never loads image sources itself: a path
                // names a file on the agent's machine, not this one, and a URL
                // would be fetched without asking. It shows a label and lets the
                // application decide what a click opens (workspacer patch).
                if let Some(on_link_click) = node_cx.style.on_link_click.clone() {
                    let url = image
                        .link
                        .as_ref()
                        .map(|link| link.url.clone())
                        .unwrap_or_else(|| image.url.to_string().into());
                    let label: SharedString = match image.alt.as_ref().filter(|a| !a.is_empty()) {
                        Some(alt) => alt.clone(),
                        None => image
                            .url
                            .rsplit(['/', '\\'])
                            .find(|s| !s.is_empty())
                            .unwrap_or("image")
                            .to_string()
                            .into(),
                    };
                    let tooltip = match image.title() {
                        title if title.is_empty() => image.url.to_string(),
                        title => title,
                    };
                    child_nodes.push(
                        h_flex()
                            .id(ix)
                            .gap_1()
                            .cursor_pointer()
                            .text_color(cx.theme().link)
                            .hover(|this| this.underline())
                            .child(Icon::new(IconName::GalleryVerticalEnd).size_3())
                            .child(label)
                            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                on_link_click(&url, window, cx);
                            })
                            .into_any_element(),
                    );
                    text.clear();
                    links.clear();
                    highlights.clear();
                    code_ranges.clear();
                    offset = 0;
                    ix += 1;
                    continue;
                }
                child_nodes.push(
                    img(image.url.clone())
                        .id(ix)
                        .object_fit(ObjectFit::Contain)
                        .max_w(relative(1.))
                        .when_some(image.width, |this, width| this.w(width))
                        .when_some(image.link.clone(), |this, link| {
                            let title = image.title();
                            this.cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(title.clone()).build(window, cx)
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    cx.open_url(&link.url);
                                })
                        })
                        .into_any_element(),
                );

                text.clear();
                links.clear();
                highlights.clear();
                code_ranges.clear();
                offset = 0;
            } else {
                let mut node_highlights = vec![];
                for (range, style) in &inline_node.marks {
                    let inner_range = (offset + range.start)..(offset + range.end);

                    let prose = node_cx.style.prose;
                    let mut highlight = HighlightStyle::default();
                    if style.bold {
                        highlight.font_weight = Some(FontWeight::BOLD);
                    }
                    if style.italic {
                        highlight.font_style = Some(FontStyle::Italic);
                    }
                    if let Some(prose) = prose.filter(|_| style.bold || style.italic) {
                        highlight.color = Some(prose.strong);
                    }
                    if style.strikethrough {
                        highlight.strikethrough = Some(gpui::StrikethroughStyle {
                            thickness: gpui::px(1.),
                            ..Default::default()
                        });
                    }
                    if style.code {
                        code_ranges.push(inner_range.clone());
                        if let Some(prose) = prose {
                            // Inline paints a rounded, padded fill instead.
                            highlight.color = Some(prose.code);
                        } else {
                            highlight.background_color = Some(cx.theme().accent);
                        }
                    }

                    if let Some(mut link_mark) = style.link.clone() {
                        highlight.color = Some(cx.theme().link);
                        highlight.underline = Some(gpui::UnderlineStyle {
                            thickness: gpui::px(1.),
                            ..Default::default()
                        });

                        // convert link references, replace link
                        if let Some(identifier) = link_mark.identifier.as_ref() {
                            if let Some(mark) = node_cx.link_refs.get(identifier) {
                                link_mark = mark.clone();
                            }
                        }

                        links.push((inner_range.clone(), link_mark));
                    }

                    node_highlights.push((inner_range, highlight));
                }

                highlights = gpui::combine_highlights(highlights, node_highlights).collect();
                offset += text_len;
            }
            ix += 1;
        }

        // Add the last text node
        if text.len() > 0 {
            self.state.lock().unwrap().set_text(text.into());
            child_nodes.push(
                Inline::new(
                    ix,
                    self.state.clone(),
                    links,
                    highlights,
                    node_cx.style.on_link_click.clone(),
                )
                .mono(mono, code_background, code_ranges)
                .into_any_element(),
            );
        }

        div().id(span.unwrap_or_default()).children(child_nodes)
    }
}

#[derive(Default, Clone, Copy)]
struct NodeRenderOptions {
    in_list: bool,
    todo: bool,
    ordered: bool,
    depth: usize,
    is_last: bool,
}

impl NodeRenderOptions {
    fn is_last(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

impl Paragraph {
    fn to_markdown(&self) -> String {
        let mut text = self
            .children
            .iter()
            .map(|text_node| {
                let mut text = text_node.text.to_string();
                for (range, style) in &text_node.marks {
                    if style.bold {
                        text = format!("**{}**", &text_node.text[range.clone()]);
                    }
                    if style.italic {
                        text = format!("*{}*", &text_node.text[range.clone()]);
                    }
                    if style.strikethrough {
                        text = format!("~~{}~~", &text_node.text[range.clone()]);
                    }
                    if style.code {
                        text = format!("`{}`", &text_node.text[range.clone()]);
                    }
                    if let Some(link) = &style.link {
                        text = format!("[{}]({})", &text_node.text[range.clone()], link.url);
                    }
                }

                if let Some(image) = &text_node.image {
                    let alt = image.alt.clone().unwrap_or_default();
                    let title = image
                        .title
                        .clone()
                        .map_or(String::new(), |t| format!(" \"{}\"", t));
                    text.push_str(&format!("![{}]({}{})", alt, image.url, title))
                }

                text
            })
            .collect::<Vec<_>>()
            .join("");

        text.push_str("\n\n");
        text
    }
}

impl Node {
    /// Converts the node to markdown format.
    ///
    /// This is used to generate markdown for test.
    #[allow(dead_code)]
    pub(crate) fn to_markdown(&self) -> String {
        match self {
            Node::Root { children } => children
                .iter()
                .map(|child| child.to_markdown())
                .collect::<Vec<_>>()
                .join("\n\n"),
            Node::Paragraph(paragraph) => paragraph.to_markdown(),
            Node::Heading { level, children } => {
                let hashes = "#".repeat(*level as usize);
                format!("{} {}", hashes, children.to_markdown())
            }
            Node::Blockquote { children } => {
                let content = children
                    .iter()
                    .map(|child| child.to_markdown())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                content
                    .lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Node::List { children, ordered } => children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let prefix = if *ordered {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".to_string()
                    };
                    format!("{}{}", prefix, child.to_markdown())
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Node::ListItem {
                children, checked, ..
            } => {
                let checkbox = if let Some(checked) = checked {
                    if *checked { "[x] " } else { "[ ] " }
                } else {
                    ""
                };
                format!(
                    "{}{}",
                    checkbox,
                    children
                        .iter()
                        .map(|child| child.to_markdown())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            Node::CodeBlock(code_block) => {
                format!(
                    "```{}\n{}\n```",
                    code_block.lang.clone().unwrap_or_default(),
                    code_block.code()
                )
            }
            Node::Table(table) => {
                let header = table
                    .children
                    .first()
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                let alignments = table
                    .column_aligns
                    .iter()
                    .map(|align| {
                        match align {
                            ColumnumnAlign::Left => ":--",
                            ColumnumnAlign::Center => ":-:",
                            ColumnumnAlign::Right => "--:",
                        }
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                let rows = table
                    .children
                    .iter()
                    .skip(1)
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}\n{}", header, alignments, rows)
            }
            Node::Break { html } => {
                if *html {
                    "<br>".to_string()
                } else {
                    "\n".to_string()
                }
            }
            Node::Divider => "---".to_string(),
            Node::Definition {
                identifier,
                url,
                title,
            } => {
                if let Some(title) = title {
                    format!("[{}]: {} \"{}\"", identifier, url, title)
                } else {
                    format!("[{}]: {}", identifier, url)
                }
            }
            Node::Unknown => "".to_string(),
        }
        .trim()
        .to_string()
    }
}

impl Node {
    fn render_list_item(
        item: &Node,
        ix: usize,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match item {
            Node::ListItem {
                children,
                spread,
                checked,
            } => v_flex()
                .id("li")
                .when(*spread, |this| this.child(div()))
                .children({
                    let mut items: Vec<Div> = Vec::with_capacity(children.len());

                    for (child_ix, child) in children.iter().enumerate() {
                        match child {
                            Node::Paragraph(_) => {
                                let last_not_list = child_ix > 0
                                    && !matches!(children[child_ix - 1], Node::List { .. });

                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );

                                // merge content into last item.
                                if last_not_list {
                                    if let Some(item_item) = items.last_mut() {
                                        item_item.extend(vec![
                                            div().overflow_hidden().child(text).into_any_element(),
                                        ]);
                                        continue;
                                    }
                                }

                                items.push(
                                    h_flex()
                                        .flex_1()
                                        .relative()
                                        .items_start()
                                        .content_start()
                                        .when(!options.todo && checked.is_none(), |this| {
                                            let marker: SharedString = if !options.ordered {
                                                node_cx
                                                    .style
                                                    .unordered_list_marker
                                                    .clone()
                                                    .unwrap_or_else(|| {
                                                        list_item_prefix(ix, false, options.depth)
                                                            .into()
                                                    })
                                            } else {
                                                list_item_prefix(ix, true, options.depth).into()
                                            };
                                            match node_cx.style.prose {
                                                Some(prose) => this.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .text_color(if options.ordered {
                                                            prose.muted
                                                        } else {
                                                            prose.marker
                                                        })
                                                        .child(marker),
                                                ),
                                                None => this.child(marker),
                                            }
                                        })
                                        .when_some(*checked, |this, checked| {
                                            // Todo list checkbox
                                            this.child(
                                                div()
                                                    .flex()
                                                    .mt(rems(0.4))
                                                    .mr_1p5()
                                                    .size(rems(0.875))
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(cx.theme().radius.half())
                                                    .border_1()
                                                    .border_color(cx.theme().primary)
                                                    .text_color(cx.theme().primary_foreground)
                                                    .when(checked, |this| {
                                                        this.bg(cx.theme().primary).child(
                                                            Icon::new(IconName::Check)
                                                                .size_2()
                                                                .text_xs(),
                                                        )
                                                    }),
                                            )
                                        })
                                        .child(div().overflow_hidden().child(text)),
                                );
                            }
                            Node::List { .. } => {
                                items.push(div().ml(rems(1.)).child(child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )));
                            }
                            _ => {}
                        }
                    }
                    items
                })
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_table(
        item: &Node,
        mb: gpui::Rems,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        const DEFAULT_LENGTH: usize = 5;
        const MAX_LENGTH: usize = 150;
        let col_lens = match item {
            Node::Table(table) => {
                let mut col_lens = vec![];
                for row in table.children.iter() {
                    for (ix, cell) in row.children.iter().enumerate() {
                        if col_lens.len() <= ix {
                            col_lens.push(DEFAULT_LENGTH);
                        }

                        let len = cell.children.text_len();
                        if len > col_lens[ix] {
                            col_lens[ix] = len;
                        }
                    }
                }
                col_lens
            }
            _ => vec![],
        };

        if let (Some(prose), Node::Table(table)) = (node_cx.style.prose, item) {
            return Self::render_prose_table(table, &col_lens, prose, mb, node_cx, window, cx)
                .into_any_element();
        }

        match item {
            Node::Table(table) => div()
                .pb(rems(1.))
                .w_full()
                .child(
                    div()
                        .id("table")
                        .w_full()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .children({
                            let mut rows = Vec::with_capacity(table.children.len());
                            for (row_ix, row) in table.children.iter().enumerate() {
                                rows.push(
                                    div()
                                        .id("row")
                                        .w_full()
                                        .when(row_ix < table.children.len() - 1, |this| {
                                            this.border_b_1()
                                        })
                                        .border_color(cx.theme().border)
                                        .flex()
                                        .flex_row()
                                        .children({
                                            let mut cells = Vec::with_capacity(row.children.len());
                                            for (ix, cell) in row.children.iter().enumerate() {
                                                let align = table.column_align(ix);
                                                let is_last_col = ix == row.children.len() - 1;
                                                let len = col_lens
                                                    .get(ix)
                                                    .copied()
                                                    .unwrap_or(MAX_LENGTH)
                                                    .min(MAX_LENGTH);

                                                cells.push(
                                                    div()
                                                        .id("cell")
                                                        .flex()
                                                        .when(
                                                            align == ColumnumnAlign::Center,
                                                            |this| this.justify_center(),
                                                        )
                                                        .when(
                                                            align == ColumnumnAlign::Right,
                                                            |this| this.justify_end(),
                                                        )
                                                        .w(Length::Definite(relative(len as f32)))
                                                        .px_2()
                                                        .py_1()
                                                        .when(!is_last_col, |this| {
                                                            this.border_r_1()
                                                                .border_color(cx.theme().border)
                                                        })
                                                        .truncate()
                                                        .child(
                                                            cell.children
                                                                .render(node_cx, window, cx),
                                                        ),
                                                )
                                            }
                                            cells
                                        }),
                                )
                            }
                            rows
                        }),
                )
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// Desktop chat tables (components/markdown.tsx `renderTable`): a quiet
    /// rounded frame, a bright bold header on the code-header tint, hairline
    /// row rules, faint zebra striping, smaller text, and wrapping cells
    /// instead of truncation so nothing is hidden.
    fn render_prose_table(
        table: &Table,
        col_lens: &[usize],
        prose: super::ProseColors,
        mb: gpui::Rems,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        const MAX_LENGTH: usize = 60;
        let row_count = table.children.len();
        let cols = col_lens.len().max(1);
        let len = |ix: usize| col_lens.get(ix).copied().unwrap_or(1).clamp(3, MAX_LENGTH);
        let total: usize = (0..cols).map(len).sum();
        // A column never narrows below its longest word, measured in the
        // cell's own font (bold header, monospace code) and capped so a long
        // token such as a URL still wraps inside its column: flex
        // shrinks wider columns instead of breaking a short header like
        // "Status" mid-word. When the minimums together exceed the width,
        // the table scrolls sideways (`ProseTableFrame`) rather than
        // clipping its right-hand columns or breaking words.
        let font_size = rems(0.875).to_pixels(window.rem_size());
        let mono = node_cx
            .style
            .inline_code_family
            .clone()
            .unwrap_or_else(|| cx.theme().mono_font_family.clone());
        let base = window.text_style().font();
        let min_widths: Vec<Pixels> = (0..cols)
            .map(|ix| {
                let longest = table
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(row_ix, row)| Some((row_ix == 0, row.children.get(ix)?)))
                    .flat_map(|(header, cell)| cell.children.long_words(header, 3))
                    .map(|word| {
                        let mut font = base.clone();
                        if word.code {
                            font.family = mono.clone();
                        }
                        if word.bold {
                            font.weight = FontWeight::BOLD;
                        } else if word.header {
                            font.weight = FontWeight::SEMIBOLD;
                        }
                        if word.italic {
                            font.style = FontStyle::Italic;
                        }
                        // Code spans carry their thin-space margins
                        // (`INLINE_CODE_MARGIN`) on the same line.
                        let text = if word.code {
                            format!("\u{2009}{}\u{2009}", word.text)
                        } else {
                            word.text
                        };
                        let run = gpui::TextRun {
                            len: text.len(),
                            font,
                            color: Hsla::default(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        window
                            .text_system()
                            .shape_line(text.into(), font_size, &[run], None)
                            .width
                    })
                    .fold(px(0.), Pixels::max);
                // The cap (about twenty characters of code) and the 10px
                // side padding, plus a pixel against rounding.
                longest.min(font_size * 12.) + px(21.)
            })
            .collect();
        // Stable while a message streams in: the source offset of the first
        // cell's text, so each table keeps its own scroll position.
        let key = table
            .children
            .iter()
            .flat_map(|row| row.children.iter())
            .find_map(|cell| cell.children.span)
            .map_or(0, |span| span.start);
        let rows = table.children.iter().enumerate().map(|(row_ix, row)| {
            let header = row_ix == 0;
            let striped = !header && row_ix % 2 == 0;
            h_flex()
                .id(("row", row_ix))
                .w_full()
                .items_start()
                .when(header, |this| {
                    this.bg(prose.code_header)
                        .text_color(prose.strong)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(striped, |this| this.bg(prose.code_header.opacity(0.45)))
                .when(row_ix + 1 < row_count, |this| {
                    this.border_b_1().border_color(if header {
                        prose.border
                    } else {
                        prose.border.opacity(0.6)
                    })
                })
                .children((0..cols).map(|ix| {
                    let align = table.column_align(ix);
                    // A block, not a flex row: the paragraph then takes the
                    // cell's width and wraps inside it instead of sizing to its
                    // unwrapped text and spilling into the next column. Text
                    // alignment replaces flex justification; clipping catches
                    // anything still too wide (an image, a giant token).
                    div()
                        .id(("cell", ix))
                        .debug_selector(move || format!("prose-table-cell-{row_ix}-{ix}"))
                        .min_w(min_widths[ix])
                        .flex_shrink()
                        .overflow_hidden()
                        .w(relative(len(ix) as f32 / total as f32))
                        .when(align == ColumnumnAlign::Center, |this| this.text_center())
                        .when(align == ColumnumnAlign::Right, |this| this.text_right())
                        .px(px(10.))
                        .py(px(if header { 6. } else { 5. }))
                        .whitespace_normal()
                        .children(
                            row.children
                                .get(ix)
                                .map(|cell| cell.children.render(node_cx, window, cx)),
                        )
                }))
        });
        div()
            .debug_selector(|| "prose-table".into())
            .pb(mb)
            .w_full()
            .child(ProseTableFrame {
                key,
                min_width: min_widths.iter().copied().fold(px(0.), |a, b| a + b),
                border: prose.border,
                focus_border: prose.marker,
                rows: rows.map(IntoElement::into_any_element).collect(),
            })
    }

    /// A short name for a block's kind; task lists are `task-list` and
    /// ordered lists `ordered-list` (workspacer patch, for app tests).
    pub(crate) fn kind_name(&self) -> &'static str {
        match self {
            Node::Root { .. } => "root",
            Node::Paragraph(_) => "paragraph",
            Node::Heading { .. } => "heading",
            Node::Blockquote { .. } => "blockquote",
            Node::List { children, ordered } => {
                if children
                    .iter()
                    .any(|item| matches!(item, Node::ListItem { checked: Some(_), .. }))
                {
                    "task-list"
                } else if *ordered {
                    "ordered-list"
                } else {
                    "list"
                }
            }
            Node::ListItem { .. } => "list-item",
            Node::CodeBlock(_) => "code",
            Node::Table(_) => "table",
            Node::Break { .. } => "break",
            Node::Divider => "divider",
            Node::Definition { .. } => "definition",
            Node::Unknown => "unknown",
        }
    }

    /// Level and plain text of a heading block, for application heading
    /// navigation (workspacer patch). Inline-code thin spaces are dropped.
    pub(crate) fn heading(&self) -> Option<(u8, String)> {
        match self {
            Node::Heading { level, children } => Some((
                *level,
                children
                    .children
                    .iter()
                    .map(|node| node.text.as_ref())
                    .collect::<String>()
                    .replace('\u{2009}', ""),
            )),
            _ => None,
        }
    }

    pub(super) fn render_root(
        &self,
        list_state: Option<ListState>,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let options = NodeRenderOptions {
            is_last: true,
            ..Default::default()
        };

        let Some(list_state) = list_state else {
            return self
                .render_block(options, node_cx, window, cx)
                .into_any_element();
        };

        let children = match self {
            Node::Root { children } => children,
            _ => return div().into_any_element(),
        };

        let children = children.clone();
        let node_cx = node_cx.clone();

        if list_state.item_count() != children.len() {
            list_state.reset(children.len());
        }

        gpui::list(list_state, move |ix, window, cx| {
            let is_last = ix + 1 == children.len();
            children[ix]
                .render_block(options.is_last(is_last), &node_cx, window, cx)
                .into_any_element()
        })
        .size_full()
        .into_any()
    }

    fn render_block(
        &self,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let mb = if options.in_list || options.is_last {
            rems(0.)
        } else {
            node_cx.style.paragraph_gap
        };

        match self {
            Node::Root { children } => {
                // Only the final block is last; passing the root's flag to every
                // child removed all paragraph gaps outside the scrolling list.
                let count = children.len();
                div()
                    .id("div")
                    .children(children.iter().enumerate().map(move |(ix, node)| {
                        node.render_block(
                            options.is_last(options.is_last && ix + 1 == count),
                            node_cx,
                            window,
                            cx,
                        )
                    }))
                    .into_any_element()
            }
            Node::Paragraph(paragraph) => div()
                .id("p")
                .pb(mb)
                .child(paragraph.render(node_cx, window, cx))
                .into_any_element(),
            Node::Heading { level, children } => {
                let (text_size, font_weight) = match level {
                    1 => (rems(2.), FontWeight::BOLD),
                    2 => (rems(1.5), FontWeight::SEMIBOLD),
                    3 => (rems(1.25), FontWeight::SEMIBOLD),
                    4 => (rems(1.125), FontWeight::SEMIBOLD),
                    5 => (rems(1.), FontWeight::SEMIBOLD),
                    6 => (rems(1.), FontWeight::MEDIUM),
                    _ => (rems(1.), FontWeight::NORMAL),
                };

                let mut text_size = text_size.to_pixels(node_cx.style.heading_base_font_size);
                if let Some(f) = node_cx.style.heading_font_size.as_ref() {
                    text_size = (f)(*level, node_cx.style.heading_base_font_size);
                }

                h_flex()
                    .id(("h", *level as usize))
                    .pb(rems(0.3))
                    .whitespace_normal()
                    .text_size(text_size)
                    .font_weight(font_weight)
                    .when_some(node_cx.style.prose, |this, prose| {
                        this.text_color(prose.strong)
                            .when(*level <= 2, |this| {
                                this.mb(rems(0.4)).border_b_1().border_color(prose.rule)
                            })
                    })
                    .child(children.render(node_cx, window, cx))
                    .into_any_element()
            }
            Node::Blockquote { children } => div()
                .w_full()
                .pb(mb)
                .child(
                    div()
                        .id("blockquote")
                        .w_full()
                        .map(|this| match node_cx.style.prose {
                            // Desktop parity: a slim marker bar and muted italic copy.
                            Some(prose) => this
                                .text_color(prose.muted)
                                .italic()
                                .border_l_2()
                                .border_color(prose.marker.opacity(0.55))
                                .pl(px(12.))
                                .py(px(2.)),
                            None => this
                                .text_color(cx.theme().muted_foreground)
                                .border_l_3()
                                .border_color(cx.theme().secondary_active)
                                .px_4(),
                        })
                        .children({
                            let children_len = children.len();
                            children.into_iter().enumerate().map(move |(index, c)| {
                                let is_last = index == children_len - 1;
                                c.render_block(options.is_last(is_last), node_cx, window, cx)
                            })
                        }),
                )
                .into_any_element(),
            Node::List { children, ordered } => v_flex()
                .id(if *ordered { "ol" } else { "ul" })
                .pb(mb)
                .children({
                    let mut items = Vec::with_capacity(children.len());
                    let mut ix = 0;
                    for item in children.into_iter() {
                        let is_item = item.is_list_item();

                        items.push(Self::render_list_item(
                            item,
                            ix,
                            NodeRenderOptions {
                                ordered: *ordered,
                                ..options
                            },
                            node_cx,
                            window,
                            cx,
                        ));

                        if is_item {
                            ix += 1;
                        }
                    }
                    items
                })
                .into_any_element(),
            Node::CodeBlock(code_block) => code_block.render(&options, node_cx, window, cx),
            Node::Table { .. } => {
                Self::render_table(self, mb, node_cx, window, cx).into_any_element()
            }
            Node::Divider => div()
                .pb(mb)
                .child(match node_cx.style.prose {
                    Some(prose) => div().id("divider").bg(prose.rule).h(px(1.)),
                    None => div().id("divider").bg(cx.theme().border).h(px(2.)),
                })
                .into_any_element(),
            Node::Break { .. } => div().id("break").into_any_element(),
            Node::Unknown | Node::Definition { .. } => div().into_any_element(),
            _ => {
                if cfg!(debug_assertions) {
                    tracing::warn!("unknown implementation: {:?}", self);
                }

                div().into_any_element()
            }
        }
    }
}

/// A word of a table cell and the marks that choose its font.
pub(crate) struct StyledWord {
    text: String,
    header: bool,
    bold: bool,
    italic: bool,
    code: bool,
}

/// The rounded frame of a prose table and its sideways scrolling
/// (workspacer patch). A table whose whole-word column minimums fit fills
/// the width with no scroll affordance. One that cannot fit keeps those
/// minimums and scrolls horizontally: trackpad or Shift+wheel over it, a
/// draggable scrollbar strip under its rows, and Left/Right once focused
/// (click it, or Tab to it). A plain vertical wheel still scrolls the page.
#[derive(IntoElement)]
struct ProseTableFrame {
    key: usize,
    min_width: Pixels,
    border: Hsla,
    focus_border: Hsla,
    rows: Vec<AnyElement>,
}

impl RenderOnce for ProseTableFrame {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.key;
        let handle = window
            .use_keyed_state(("prose-table-scroll", key), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        // From the last layout; a mismatch with this frame's layout is
        // corrected below by an immediate second draw.
        let overflowing = handle.max_offset().width > px(0.5);
        let view = window.current_view();
        let mut scroll = div()
            .id("table-scroll")
            .w_full()
            .overflow_x_scroll()
            .track_scroll(&handle)
            .child(
                div()
                    .id("table")
                    .debug_selector(move || format!("prose-table-content-{key}"))
                    .w_full()
                    .min_w(self.min_width)
                    .children(self.rows),
            );
        // A vertical wheel over a wide table scrolls the page, not the table.
        scroll.style().restrict_scroll_to_axis = Some(true);
        div()
            .id(("prose-table", key))
            .debug_selector(move || format!("prose-table-frame-{key}"))
            .w_full()
            .text_size(rems(0.875))
            .line_height(relative(1.5))
            .border_1()
            .border_color(self.border)
            .rounded(px(8.))
            .overflow_hidden()
            .when(overflowing, |this| {
                this.focusable()
                    .tab_stop(true)
                    .focus(move |s| s.border_color(self.focus_border))
            })
            .on_key_down({
                let handle = handle.clone();
                move |event: &KeyDownEvent, window, cx| {
                    let m = &event.keystroke.modifiers;
                    if m.control || m.alt || m.platform || m.function || m.shift {
                        return;
                    }
                    // Keys pass through a table that (no longer) scrolls.
                    if handle.max_offset().width <= px(0.5) {
                        return;
                    }
                    let step = (handle.bounds().size.width * 0.25).max(window.rem_size() * 2.5);
                    let delta = match event.keystroke.key.as_str() {
                        "left" => -step,
                        "right" => step,
                        _ => return,
                    };
                    if scroll_table_by(&handle, delta) {
                        cx.notify(view);
                    }
                    cx.stop_propagation();
                }
            })
            .child(scroll)
            .when(overflowing, |this| {
                this.child(
                    div()
                        .debug_selector(|| "prose-table-scrollbar".into())
                        .relative()
                        .w_full()
                        .h(px(14.))
                        .border_t_1()
                        .border_color(self.border.opacity(0.6))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .child(
                                    Scrollbar::horizontal(&handle)
                                        .id("prose-table-scrollbar")
                                        .scrollbar_show(ScrollbarShow::Always),
                                ),
                        ),
                )
            })
            .child(
                canvas(
                    move |_, _, cx| {
                        // This layout disagrees with the one the scrollbar and
                        // focusability were chosen from: redraw once it ends.
                        if (handle.max_offset().width > px(0.5)) != overflowing {
                            cx.defer(move |cx| cx.notify(view));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            )
    }
}

/// Scroll a prose table sideways by `delta`, within its content; whether
/// the offset changed.
fn scroll_table_by(handle: &ScrollHandle, delta: Pixels) -> bool {
    let max = handle.max_offset().width;
    let offset = handle.offset();
    let x = (-offset.x + delta).clamp(px(0.), max);
    if x == -offset.x {
        return false;
    }
    handle.set_offset(point(-x, offset.y));
    true
}
