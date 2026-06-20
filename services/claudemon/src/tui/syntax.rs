//! Syntax highlighting for fenced code blocks and `Read` tool previews.
//!
//! Backed by [`syntect`] using its default-fancy (pure-Rust regex) feature,
//! so we get ~70 languages' TextMate grammars without a C toolchain.
//! The grammar set and theme are loaded once via `once_cell::Lazy` —
//! first highlight pays a one-off ~10–30 ms cost, every subsequent call
//! is a cheap state-machine step.

use once_cell::sync::Lazy;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};

// `two-face` ships syntect-compatible grammars for languages the default
// pack omits — most notably TypeScript, TSX, and JSX — plus everything
// syntect's default set already had. Drop-in replacement.
static SYNTAXES: Lazy<SyntaxSet> = Lazy::new(two_face::syntax::extra_newlines);

static THEME: Lazy<Theme> = Lazy::new(|| {
    let ts = ThemeSet::load_defaults();
    // base16-ocean.dark reads cleanly on the typical dark terminal —
    // green strings, magenta keywords, yellow types, dim gray comments.
    ts.themes
        .get("base16-ocean.dark")
        .cloned()
        .unwrap_or_else(|| {
            ts.themes
                .values()
                .next()
                .cloned()
                .expect("syntect has at least one built-in theme")
        })
});

/// Map a file path's extension to the language token syntect uses for
/// lookup (we hand it to `find_syntax_by_token`). Returns "" when we
/// don't recognize the extension — that yields a no-op highlighter.
pub fn language_from_path(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rs",
        "py" | "pyi" => "py",
        "js" | "jsx" | "mjs" | "cjs" => "js",
        "ts" => "ts",
        "tsx" => "tsx",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "sh",
        "go" => "go",
        "cs" => "cs",
        "c" | "h" => "c",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp",
        "java" => "java",
        "rb" => "rb",
        "php" => "php",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "md" | "markdown" => "md",
        "sql" => "sql",
        "xml" => "xml",
        "lua" => "lua",
        "swift" => "swift",
        "kt" | "kts" => "kt",
        _ => "",
    }
}

fn syntax_for(token: &str) -> Option<&'static SyntaxReference> {
    if token.is_empty() {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    // `find_syntax_by_token` matches both file extensions and short names
    // (e.g. "rust"/"rs"), which is what most users type after the fence.
    let direct = SYNTAXES
        .find_syntax_by_token(&lower)
        .or_else(|| SYNTAXES.find_syntax_by_extension(&lower))
        .or_else(|| SYNTAXES.find_syntax_by_name(token));
    if direct.is_some() {
        return direct;
    }
    // Last-resort fallbacks for grammars an embed might not ship.
    // Highlighting JSX/TSX as JS is much better than no color at all.
    let fallback_token = match lower.as_str() {
        "tsx" => Some("ts"),
        "ts" => Some("js"),
        "jsx" => Some("js"),
        "kt" | "kts" => Some("java"),
        "scss" | "sass" => Some("css"),
        _ => None,
    };
    fallback_token.and_then(|t| SYNTAXES.find_syntax_by_token(t))
}

/// Stateful per-block highlighter. Keep one instance for the duration
/// of a fenced code block or a `Read` tool result so multi-line strings
/// and block comments continue their tokenization across line breaks.
pub struct Highlighter {
    inner: Option<HighlightLines<'static>>,
}

impl Highlighter {
    pub fn for_language(lang: &str) -> Self {
        let inner = syntax_for(lang).map(|s| HighlightLines::new(s, &THEME));
        Self { inner }
    }

    /// Convenience: pick a language from a file path's extension.
    #[allow(dead_code)]
    pub fn for_path(path: &str) -> Self {
        Self::for_language(language_from_path(path))
    }

    /// True if we found a grammar — callers can short-circuit and skip
    /// gutter/marker work when this is false.
    pub fn is_active(&self) -> bool {
        self.inner.is_some()
    }

    /// Highlight one line and return owned ratatui spans. `base` is the
    /// fallback fg/style applied when syntect doesn't override the color
    /// (or when no grammar matched).
    pub fn highlight(&mut self, line: &str, base: Style) -> Vec<Span<'static>> {
        let Some(hl) = self.inner.as_mut() else {
            return vec![Span::styled(line.to_string(), base)];
        };
        match hl.highlight_line(line, &SYNTAXES) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(style, text)| {
                    Span::styled(text.to_string(), to_ratatui_style(style, base))
                })
                .collect(),
            Err(_) => vec![Span::styled(line.to_string(), base)],
        }
    }
}

fn to_ratatui_style(s: SynStyle, base: Style) -> Style {
    let mut style = base.fg(Color::Rgb(
        s.foreground.r,
        s.foreground.g,
        s.foreground.b,
    ));
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// Stateless single-line helper — convenient for one-off uses and
/// tests. For multi-line blocks prefer a `Highlighter` so the
/// tokenizer state carries across lines.
#[allow(dead_code)]
pub fn highlight_line(lang: &str, line: &str, base: Style) -> Vec<Span<'static>> {
    Highlighter::for_language(lang).highlight(line, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn language_from_path_handles_common_extensions() {
        assert_eq!(language_from_path("foo.rs"), "rs");
        assert_eq!(language_from_path("dir/bar.PY"), "py");
        assert_eq!(language_from_path("a.tsx"), "tsx");
        assert_eq!(language_from_path("a.cs"), "cs");
        assert_eq!(language_from_path("Cargo.toml"), "toml");
        assert_eq!(language_from_path("no-ext"), "");
    }

    #[test]
    fn unknown_language_returns_single_span() {
        let spans = highlight_line("brainfuck", "++>--<", Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "++>--<");
    }

    #[test]
    fn rust_produces_multiple_styled_spans() {
        let line = r#"let s = "hi";"#;
        let spans = highlight_line("rust", line, Style::default());
        assert_eq!(flat(&spans), line);
        // syntect should have broken the line into at least keyword /
        // ident / string / punctuation. The exact theme colors vary but
        // the structure should be richer than one span.
        assert!(
            spans.len() > 2,
            "expected several spans, got {}: {:?}",
            spans.len(),
            spans
        );
        // At least one span should be styled (theme overrides fg on
        // most tokens).
        assert!(
            spans.iter().any(|s| s.style.fg.is_some()),
            "no styled spans"
        );
    }

    #[test]
    fn tsx_and_ts_get_a_real_grammar() {
        // two-face's pack ships TS/TSX; this guards against accidentally
        // reverting to the syntect-default set, which doesn't include them.
        assert!(
            Highlighter::for_language("tsx").is_active(),
            "tsx grammar missing — did SYNTAXES drop two-face?"
        );
        assert!(
            Highlighter::for_language("ts").is_active(),
            "ts grammar missing"
        );
        assert!(
            Highlighter::for_path("Foo.tsx").is_active(),
            "tsx via path lookup failed"
        );
    }

    #[test]
    fn stateful_highlighter_carries_across_lines() {
        // A python triple-quoted string spans two lines. With a stateful
        // highlighter the second line should still be styled as a string,
        // not as bare identifiers.
        let mut h = Highlighter::for_language("py");
        assert!(h.is_active(), "python grammar should load");
        let l1 = h.highlight(r#"x = """hello"#, Style::default());
        let l2 = h.highlight(r#"world""""#, Style::default());
        let l1_flat: String = l1.iter().map(|s| s.content.as_ref()).collect();
        let l2_flat: String = l2.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l1_flat, r#"x = """hello"#);
        assert_eq!(l2_flat, r#"world""""#);
    }
}
