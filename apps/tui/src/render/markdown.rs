//! Markdown → styled ratatui lines, for transcript message bodies.
//!
//! The TUI's own renderer: pulldown-cmark drives a small block/inline state
//! machine that emits themed [`Line`]s — headings, bold/italic, inline code,
//! bullet/numbered lists, blockquotes, fenced code blocks (monochrome, with a
//! language tag on the top border), and underlined link labels. All colors are
//! theme roles (see `theme.rs`); no literal colors, no syntax highlighting.
//!
//! Soft breaks are kept as real line breaks (chat text is line-oriented — a
//! user's `\n` must not silently join into one paragraph).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrap::{display_width, wrap_spans};
use crate::theme::Theme;

/// Render markdown `text` into wrapped, styled lines at most `width` columns
/// wide. Returns no trailing blank line; the caller owns inter-turn spacing.
pub fn markdown_lines(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut r = Renderer {
        t: theme,
        width: width.max(10),
        out: Vec::new(),
        inline: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        link: 0,
        heading: None,
        quote: 0,
        lists: Vec::new(),
        marker_pending: false,
        code_lang: None,
        code_buf: String::new(),
    };
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(text, opts) {
        r.event(event);
    }
    r.flush_inline();
    // The block machinery separates blocks with blank lines; trim the tail.
    while r.out.last().is_some_and(|l| l.width() == 0) {
        r.out.pop();
    }
    r.out
}

struct Renderer<'a> {
    t: &'a Theme,
    width: usize,
    out: Vec<Line<'static>>,
    /// Styled spans of the block currently being accumulated.
    inline: Vec<Span<'static>>,
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
    heading: Option<HeadingLevel>,
    /// Blockquote nesting depth (each level draws a `▎` gutter).
    quote: usize,
    /// Active lists, innermost last: `None` = bullet, `Some(n)` = next number.
    lists: Vec<Option<u64>>,
    /// True until the current list item's first line takes its marker.
    marker_pending: bool,
    /// `Some(lang)` while inside a fenced/indented code block.
    code_lang: Option<String>,
    code_buf: String,
}

impl Renderer<'_> {
    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(s) => {
                if self.code_lang.is_some() {
                    self.code_buf.push_str(&s);
                } else {
                    let style = self.inline_style();
                    self.inline.push(Span::styled(s.into_string(), style));
                }
            }
            Event::Code(s) => {
                // Inline code: distinct fg over the subtle selection background.
                let style = Style::default().fg(self.t.warn).bg(self.t.selection_bg);
                self.inline.push(Span::styled(s.into_string(), style));
            }
            // Chat text is line-oriented: soft breaks stay real line breaks.
            Event::SoftBreak | Event::HardBreak => self.break_line(),
            Event::Rule => {
                self.flush_inline();
                let w = self.width.min(24);
                self.out.push(Line::from(Span::styled(
                    "─".repeat(w),
                    Style::default().fg(self.t.dim),
                )));
                self.block_gap();
            }
            Event::TaskListMarker(done) => {
                let style = if done {
                    Style::default().fg(self.t.ok)
                } else {
                    Style::default().fg(self.t.dim)
                };
                self.inline
                    .push(Span::styled(if done { "☑ " } else { "☐ " }, style));
            }
            Event::Html(s) | Event::InlineHtml(s) => {
                // No HTML rendering — surface it verbatim as dim text. Block
                // HTML arrives with embedded/trailing newlines; keep its line
                // structure (wrap_spans treats '\n' as a zero-width word char
                // and ratatui drops it at render time, so the whole block
                // would otherwise mush into one line split at random columns).
                let style = Style::default().fg(self.t.dim);
                for (i, piece) in s.split('\n').enumerate() {
                    if i > 0 {
                        self.break_line();
                    }
                    if !piece.is_empty() {
                        self.inline.push(Span::styled(piece.to_string(), style));
                    }
                }
            }
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.flush_inline(),
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush_inline();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_lang = Some(lang);
                self.code_buf.clear();
            }
            Tag::List(start) => {
                self.flush_inline();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush_inline();
                self.marker_pending = true;
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } => self.link += 1,
            Tag::Image { .. } => {
                let style = Style::default().fg(self.t.dim);
                self.inline.push(Span::styled("🖼 ", style));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_inline();
                self.block_gap();
            }
            TagEnd::Heading(_) => {
                self.flush_heading();
                self.block_gap();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.quote = self.quote.saturating_sub(1);
                self.block_gap();
            }
            TagEnd::CodeBlock => {
                self.flush_code_block();
                self.block_gap();
            }
            TagEnd::List(_) => {
                self.flush_inline();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.block_gap();
                }
            }
            TagEnd::Item => {
                self.flush_inline();
                self.marker_pending = false;
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = self.link.saturating_sub(1),
            _ => {}
        }
    }

    /// The style for plain inline text given the active emphasis/link nesting.
    fn inline_style(&self) -> Style {
        let mut s = Style::default();
        if self.bold > 0 || self.heading.is_some() {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link > 0 {
            s = s.add_modifier(Modifier::UNDERLINED).fg(self.t.accent);
        }
        if let Some(level) = self.heading {
            // Level-aware heading color: top levels in accent, deeper in fg.
            if (level as u8) <= HeadingLevel::H2 as u8 {
                s = s.fg(self.t.accent);
            }
        }
        s
    }

    /// The (first line, continuation) prefixes for the current block context:
    /// quote gutters plus list indent/marker.
    fn prefixes(&mut self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let mut first: Vec<Span<'static>> = Vec::new();
        let gutter_style = Style::default().fg(self.t.dim);
        for _ in 0..self.quote {
            first.push(Span::styled("▎ ", gutter_style));
        }
        let mut rest = first.clone();
        if !self.lists.is_empty() {
            let depth = self.lists.len() - 1;
            let indent = "  ".repeat(depth);
            if !indent.is_empty() {
                first.push(Span::raw(indent.clone()));
                rest.push(Span::raw(indent));
            }
            let marker = if self.marker_pending {
                self.marker_pending = false;
                match self.lists.last_mut().expect("non-empty list stack") {
                    Some(n) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    None => "• ".to_string(),
                }
            } else {
                String::new()
            };
            // Continuation lines align under the item text, past the marker.
            let marker_w = display_width(&marker).max(2);
            rest.push(Span::raw(" ".repeat(marker_w)));
            if marker.is_empty() {
                first.push(Span::raw(" ".repeat(marker_w)));
            } else {
                first.push(Span::styled(marker, Style::default().fg(self.t.accent)));
            }
        }
        (first, rest)
    }

    /// Wrap and emit the accumulated inline spans under the current prefixes.
    fn flush_inline(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        let (first, rest) = self.prefixes();
        let prefix_w = first
            .iter()
            .map(|s| display_width(&s.content))
            .sum::<usize>();
        let avail = self.width.saturating_sub(prefix_w).max(4);
        for (i, line) in wrap_spans(&spans, avail).into_iter().enumerate() {
            let mut out: Vec<Span<'static>> = if i == 0 { first.clone() } else { rest.clone() };
            out.extend(line.spans);
            self.out.push(Line::from(out));
        }
    }

    /// A line break inside a block: flush what's accumulated, keep the block
    /// context (the next flush uses continuation prefixes since the marker was
    /// consumed).
    fn break_line(&mut self) {
        if self.inline.is_empty() {
            // Preserve an intentionally blank line.
            let (first, _) = self.prefixes();
            self.out.push(Line::from(first));
            return;
        }
        self.flush_inline();
    }

    /// A heading: `#`-marker prefix (level-aware) + bold accent content.
    fn flush_heading(&mut self) {
        let Some(level) = self.heading.take() else {
            return;
        };
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        let marker = format!("{} ", "#".repeat(level as usize));
        let marker_style = Style::default()
            .fg(self.t.accent)
            .add_modifier(Modifier::BOLD);
        let avail = self.width.saturating_sub(display_width(&marker)).max(4);
        for (i, line) in wrap_spans(&spans, avail).into_iter().enumerate() {
            let mut out = vec![if i == 0 {
                Span::styled(marker.clone(), marker_style)
            } else {
                Span::raw(" ".repeat(display_width(&marker)))
            }];
            out.extend(line.spans);
            self.out.push(Line::from(out));
        }
    }

    /// A fenced code block: dim top border carrying the language tag, a dim
    /// `│` gutter per code line (hard-split at width, spaces preserved), and a
    /// dim bottom border. Monochrome by design — no highlighting engine.
    fn flush_code_block(&mut self) {
        let lang = self.code_lang.take().unwrap_or_default();
        let code = std::mem::take(&mut self.code_buf);
        let dim = Style::default().fg(self.t.dim);
        // The first-line prefix carries a pending list marker (consumed here);
        // every following line uses the continuation prefix, or the marker
        // would repeat down the whole block.
        let (first, rest) = self.prefixes();
        let prefix_w = first
            .iter()
            .map(|s| display_width(&s.content))
            .sum::<usize>();
        let avail = self.width.saturating_sub(prefix_w + 2).max(4);

        let top = if lang.is_empty() {
            "╭────".to_string()
        } else {
            format!("╭─ {lang} ─")
        };
        let mut line = first;
        line.push(Span::styled(top, dim));
        self.out.push(Line::from(line));

        for code_line in code.trim_end_matches('\n').split('\n') {
            // Expand tabs: ratatui filters control chars at render time (a
            // kept tab would draw as nothing) and unicode_width counts them
            // 0 — tab-indented code (Go, Makefiles) would render flush-left.
            let code_line = code_line.replace('\t', "    ");
            for piece in hard_split(&code_line, avail) {
                let mut line = rest.clone();
                line.push(Span::styled("│ ", dim));
                line.push(Span::styled(piece, Style::default().fg(self.t.fg)));
                self.out.push(Line::from(line));
            }
        }

        let mut line = rest;
        line.push(Span::styled("╰────", dim));
        self.out.push(Line::from(line));
    }

    /// A blank separator line between blocks (deduplicated).
    fn block_gap(&mut self) {
        if self.out.last().is_some_and(|l| l.width() > 0) {
            self.out.push(Line::raw(""));
        }
    }
}

/// Split a single (code) line into display-width chunks, preserving spaces —
/// no word wrapping, so code indentation survives.
fn hard_split(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_w + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += w;
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::default()
    }

    fn text_of(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn plain_paragraphs_keep_their_blank_separation() {
        let lines = markdown_lines("first para\n\nsecond para", &theme(), 40);
        assert_eq!(text_of(&lines), vec!["first para", "", "second para"]);
    }

    #[test]
    fn soft_breaks_stay_real_line_breaks() {
        // Chat text: a single \n must not join into one paragraph.
        let lines = markdown_lines("line one\nline two", &theme(), 40);
        assert_eq!(text_of(&lines), vec!["line one", "line two"]);
    }

    #[test]
    fn fenced_code_block_in_and_out() {
        let t = theme();
        let md = "before\n\n```rust\nfn main() {}\nlet x = 1;\n```\n\nafter";
        let lines = markdown_lines(md, &t, 40);
        let texts = text_of(&lines);
        assert_eq!(
            texts,
            vec![
                "before",
                "",
                "╭─ rust ─",
                "│ fn main() {}",
                "│ let x = 1;",
                "╰────",
                "",
                "after",
            ]
        );
        // The language tag / borders are dim; the code body is plain fg.
        let dim = Style::default().fg(t.dim);
        assert_eq!(lines[2].spans[0].style, dim, "top border is dim");
        assert_eq!(lines[3].spans[0].style, dim, "gutter is dim");
        assert_eq!(
            lines[3].spans[1].style,
            Style::default().fg(t.fg),
            "code text uses the plain fg role"
        );
    }

    #[test]
    fn unlabelled_fence_gets_a_plain_border() {
        let lines = markdown_lines("```\nx\n```", &theme(), 40);
        assert_eq!(text_of(&lines), vec!["╭────", "│ x", "╰────"]);
    }

    #[test]
    fn code_block_opening_a_list_item_takes_the_marker_once() {
        // The item marker lands on the top border only; code lines and the
        // bottom border align under it with the continuation prefix.
        let md = "1. ```\n   only code\n   ```\n2. after";
        let lines = markdown_lines(md, &theme(), 40);
        assert_eq!(
            text_of(&lines),
            vec!["1. ╭────", "   │ only code", "   ╰────", "", "2. after"]
        );
    }

    #[test]
    fn nested_lists_indent_two_per_level() {
        let md = "- top\n  - inner\n    - deepest\n- second";
        let lines = markdown_lines(md, &theme(), 40);
        assert_eq!(
            text_of(&lines),
            vec!["• top", "  • inner", "    • deepest", "• second"]
        );
    }

    #[test]
    fn ordered_lists_number_their_items() {
        let md = "1. one\n2. two\n3. three";
        let lines = markdown_lines(md, &theme(), 40);
        assert_eq!(text_of(&lines), vec!["1. one", "2. two", "3. three"]);
    }

    #[test]
    fn list_item_wrap_aligns_under_the_marker() {
        let md = "- alpha beta gamma delta";
        let lines = markdown_lines(md, &theme(), 14);
        assert_eq!(text_of(&lines), vec!["• alpha beta", "  gamma delta"]);
    }

    #[test]
    fn inline_code_has_its_own_span_with_code_styling() {
        let t = theme();
        let lines = markdown_lines("run `cargo test` now", &t, 40);
        assert_eq!(text_of(&lines), vec!["run cargo test now"]);
        let spans = &lines[0].spans;
        let code = spans
            .iter()
            .find(|s| s.content.as_ref() == "cargo test")
            .expect("inline code is its own span");
        assert_eq!(code.style.fg, Some(t.warn), "inline code fg role");
        assert_eq!(code.style.bg, Some(t.selection_bg), "inline code bg role");
        // The surrounding text is unstyled.
        assert_eq!(spans[0].style, Style::default());
    }

    #[test]
    fn heading_is_bold_accent_and_level_aware() {
        let t = theme();
        let lines = markdown_lines("## Section", &t, 40);
        assert_eq!(text_of(&lines), vec!["## Section"]);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content.as_ref(), "## ");
        assert_eq!(spans[0].style.fg, Some(t.accent));
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].style.fg, Some(t.accent), "h2 content in accent");

        // h3 keeps bold but drops the accent fg on content.
        let lines = markdown_lines("### Sub", &t, 40);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content.as_ref(), "### ");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].style.fg, None);
    }

    #[test]
    fn bold_and_italic_set_modifiers() {
        let lines = markdown_lines("a **bold** and *italic* word", &theme(), 60);
        let spans = &lines[0].spans;
        let bold = spans.iter().find(|s| s.content.as_ref() == "bold").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let italic = spans
            .iter()
            .find(|s| s.content.as_ref() == "italic")
            .unwrap();
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn blockquote_draws_a_dim_gutter() {
        let t = theme();
        let lines = markdown_lines("> quoted text", &t, 40);
        assert_eq!(text_of(&lines), vec!["▎ quoted text"]);
        assert_eq!(lines[0].spans[0].style, Style::default().fg(t.dim));
    }

    #[test]
    fn link_label_is_underlined() {
        let t = theme();
        let lines = markdown_lines("see [the docs](https://x.dev) here", &t, 60);
        assert_eq!(text_of(&lines), vec!["see the docs here"]);
        let label = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("docs"))
            .unwrap();
        assert!(label.style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(label.style.fg, Some(t.accent));
    }

    #[test]
    fn wide_chars_wrap_by_display_width_not_char_count() {
        // 6 CJK glyphs = 12 columns; at width 10 only 5 glyphs fit per line
        // (each glyph is 2 wide — 6 would fit by char count).
        let lines = markdown_lines("宽字符测试啊", &theme(), 10);
        assert_eq!(text_of(&lines), vec!["宽字符测试", "啊"]);
    }

    #[test]
    fn html_block_keeps_its_line_structure() {
        // Multi-line HTML must not mush into one giant "word" that hard-splits
        // mid-tag; each source line stays its own rendered line.
        let md = "<div>\n<span>hello world</span>\n</div>";
        let lines = markdown_lines(md, &theme(), 30);
        assert_eq!(
            text_of(&lines),
            vec!["<div>", "<span>hello world</span>", "</div>"]
        );
        // And no line carries a raw newline for ratatui to silently drop.
        for l in text_of(&lines) {
            assert!(!l.contains('\n'), "no embedded newline in {l:?}");
        }
    }

    #[test]
    fn inline_html_stays_inline() {
        let lines = markdown_lines("a <br> b", &theme(), 40);
        assert_eq!(text_of(&lines), vec!["a <br> b"]);
    }

    #[test]
    fn code_block_tabs_expand_to_spaces() {
        // Tabs are control chars: ratatui filters them at render time and
        // unicode_width counts them 0, so kept tabs would erase indentation.
        let md = "```go\n\tfmt.Println(1)\n```";
        let lines = markdown_lines(md, &theme(), 40);
        let texts = text_of(&lines);
        assert_eq!(texts[1], "│     fmt.Println(1)", "tab became four spaces");
        assert!(!texts[1].contains('\t'), "no raw tab survives");
    }

    #[test]
    fn long_code_lines_hard_split_preserving_indent() {
        let md = "```\n    indented_very_long_line_of_code\n```";
        let lines = markdown_lines(md, &theme(), 20);
        let texts = text_of(&lines);
        assert_eq!(texts[0], "╭────");
        assert!(texts[1].starts_with("│     indented"), "got {:?}", texts[1]);
        assert!(texts[1].chars().count() <= 20);
        assert!(texts[2].starts_with("│ "), "continuation keeps the gutter");
    }
}
