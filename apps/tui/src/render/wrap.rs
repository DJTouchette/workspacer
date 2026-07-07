//! Span-aware, display-width-aware wrapping for transcript text.
//!
//! Replaces the old char-count greedy wrap for everything the chat renderer
//! draws: wide glyphs (CJK, emoji) count their real column width so wrapped
//! lines never overflow the pane, and styled spans survive wrapping — a word
//! that crosses a style boundary (e.g. `code`,) still wraps as one unit.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Display width of a string in terminal columns (wide glyphs count 2,
/// zero-width combining marks count 0).
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Truncate `s` to at most `max` display columns, appending `…` when it's cut
/// — the display-width sibling of `crate::types::truncate` (which counts
/// chars). For lines that render without wrapping: wide glyphs (CJK, emoji)
/// count their real column width, so the result never overflows the pane.
pub fn truncate_width(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1); // one column reserved for the ellipsis
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// One styled run of characters that wraps as a unit boundary: either a run of
/// spaces or a run of non-space characters, from a single source span.
struct Atom {
    text: String,
    style: Style,
    space: bool,
}

/// Greedy word-wrap over styled spans to `width` display columns.
///
/// Words are maximal runs of non-space characters and may cross span
/// boundaries (the pieces keep their own styles). Leading spaces are dropped
/// at wrap points; tokens wider than a line hard-split at display width.
/// Always returns at least one (possibly empty) line.
pub fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);

    // Tokenize into alternating space / non-space atoms, preserving style.
    let mut atoms: Vec<Atom> = Vec::new();
    for sp in spans {
        let mut cur = String::new();
        let mut cur_space = None::<bool>;
        for ch in sp.content.chars() {
            let is_space = ch == ' ';
            if cur_space != Some(is_space) && !cur.is_empty() {
                atoms.push(Atom {
                    text: std::mem::take(&mut cur),
                    style: sp.style,
                    space: cur_space.unwrap_or(false),
                });
            }
            cur_space = Some(is_space);
            cur.push(ch);
        }
        if !cur.is_empty() {
            atoms.push(Atom {
                text: cur,
                style: sp.style,
                space: cur_space.unwrap_or(false),
            });
        }
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0usize;
    // Space atoms held back until the next word commits to the same line.
    let mut pending_space: Vec<(String, Style)> = Vec::new();

    let flush =
        |lines: &mut Vec<Line<'static>>, cur: &mut Vec<Span<'static>>, cur_w: &mut usize| {
            lines.push(Line::from(std::mem::take(cur)));
            *cur_w = 0;
        };

    let mut i = 0;
    while i < atoms.len() {
        if atoms[i].space {
            pending_space.push((atoms[i].text.clone(), atoms[i].style));
            i += 1;
            continue;
        }
        // Collect the whole word: consecutive non-space atoms (across spans).
        let mut word: Vec<(String, Style)> = Vec::new();
        let mut word_w = 0usize;
        while i < atoms.len() && !atoms[i].space {
            word_w += display_width(&atoms[i].text);
            word.push((atoms[i].text.clone(), atoms[i].style));
            i += 1;
        }
        let space_w: usize = if cur_w == 0 {
            0 // leading spaces are dropped at a line start
        } else {
            pending_space.iter().map(|(s, _)| display_width(s)).sum()
        };

        if cur_w + space_w + word_w <= width {
            if cur_w > 0 {
                for (s, st) in pending_space.drain(..) {
                    push_span(&mut cur, s, st);
                }
            } else {
                pending_space.clear();
            }
            cur_w += space_w + word_w;
            for (s, st) in word {
                push_span(&mut cur, s, st);
            }
            continue;
        }

        // Doesn't fit — break the line (spaces at the break vanish).
        pending_space.clear();
        if cur_w > 0 {
            flush(&mut lines, &mut cur, &mut cur_w);
        }
        if word_w <= width {
            cur_w = word_w;
            for (s, st) in word {
                push_span(&mut cur, s, st);
            }
            continue;
        }
        // Token wider than a line: hard-split at display width, styled per char.
        for (s, st) in word {
            for ch in s.chars() {
                let w = ch.width().unwrap_or(0);
                if cur_w + w > width && cur_w > 0 {
                    flush(&mut lines, &mut cur, &mut cur_w);
                }
                let mut buf = String::new();
                buf.push(ch);
                push_span(&mut cur, buf, st);
                cur_w += w;
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(Line::from(cur));
    }
    lines
}

/// Append `text` to `spans`, merging into the last span when styles match (so
/// hard-split output doesn't degenerate into one span per character).
fn push_span(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(&text);
            return;
        }
    }
    spans.push(Span::styled(text, style));
}

/// Plain-text convenience over [`wrap_spans`]: greedy width-aware word wrap
/// returning bare strings. Drop-in replacement for the old char-count wrap.
pub fn wrap_plain(s: &str, width: usize) -> Vec<String> {
    wrap_spans(&[Span::raw(s.to_string())], width)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|sp| sp.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn plain_wrap_matches_greedy_word_wrap() {
        assert_eq!(wrap_plain("a bb ccc dddd", 6), vec!["a bb", "ccc", "dddd"]);
        assert_eq!(wrap_plain("", 10), vec![""], "empty input yields one line");
    }

    #[test]
    fn long_token_hard_splits_at_display_width() {
        assert_eq!(
            wrap_plain("abcdefghij", 4),
            vec!["abcd", "efgh", "ij"],
            "an over-wide token splits at the column width"
        );
    }

    #[test]
    fn wide_chars_count_two_columns() {
        // Each CJK glyph is 2 columns, so only 2 fit on a width-5 line.
        assert_eq!(wrap_plain("你好世界", 5), vec!["你好", "世界"]);
        // Hard-splitting a mixed-width token packs by columns: a+b+你+c fills
        // exactly 5 columns, so only d spills.
        assert_eq!(wrap_plain("ab你cd", 5), vec!["ab你c", "d"]);
        // A wide glyph never straddles the boundary: after 5 of 6 columns the
        // next glyph (2 wide) moves whole to the next line.
        assert_eq!(wrap_plain("abcde你", 6), vec!["abcde", "你"]);
    }

    #[test]
    fn truncate_width_passes_short_strings_through() {
        assert_eq!(truncate_width("hello", 10), "hello");
        assert_eq!(truncate_width("hello", 5), "hello", "exact fit is kept");
        assert_eq!(truncate_width("", 4), "");
    }

    #[test]
    fn truncate_width_cuts_at_display_columns_with_ellipsis() {
        assert_eq!(truncate_width("abcdefgh", 5), "abcd…");
        assert!(display_width(&truncate_width("abcdefgh", 5)) <= 5);
    }

    #[test]
    fn truncate_width_counts_wide_glyphs_as_two_columns() {
        // 4 CJK glyphs = 8 columns; at max 5 only two glyphs (4) + '…' fit.
        assert_eq!(truncate_width("宽字符测", 5), "宽字…");
        assert!(display_width(&truncate_width("宽字符测", 5)) <= 5);
        // A wide glyph never straddles the budget: 3 columns fit one glyph,
        // the next (2 wide) would exceed the 2-column budget and is dropped.
        assert_eq!(truncate_width("宽字符测", 3), "宽…");
    }

    #[test]
    fn spans_survive_wrapping_with_styles_intact() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let code = Style::default().fg(Color::Yellow);
        let spans = vec![
            Span::styled("hello ", bold),
            Span::styled("world", code),
            Span::raw("!"),
        ];
        let lines = wrap_spans(&spans, 20);
        assert_eq!(texts(&lines), vec!["hello world!"]);
        let line = &lines[0];
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].style, bold);
        assert_eq!(line.spans[1].style, code);
        assert_eq!(line.spans[1].content.as_ref(), "world");
    }

    #[test]
    fn word_crossing_a_span_boundary_wraps_as_one_unit() {
        // "`config`," is one wrap unit even though the comma is a second span.
        let code = Style::default().fg(Color::Yellow);
        let spans = vec![
            Span::raw("edit "),
            Span::styled("config", code),
            Span::raw(", please"),
        ];
        // width 11 fits "edit " (5) but not "config," (7) after it.
        let lines = wrap_spans(&spans, 11);
        assert_eq!(texts(&lines), vec!["edit", "config,", "please"]);
        // The code span kept its style on its own line.
        assert_eq!(lines[1].spans[0].style, code);
        assert_eq!(lines[1].spans[0].content.as_ref(), "config");
    }
}
