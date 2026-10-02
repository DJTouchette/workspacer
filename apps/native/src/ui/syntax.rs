//! Code highlighting colors that follow the native appearance. Dark and Light
//! use the same GitHub Default palettes as the desktop chat (shiki
//! github-dark-default / github-light-default); Nord uses the Nord syntax set.
use gpui_component::highlighter::{HighlightTheme, SyntaxColors};
use std::sync::Arc;
use wks_native::appearance::Appearance;

/// Tree-sitter capture → color for each appearance. Captures left out render
/// in the editor foreground, as plain identifiers do in the desktop chat.
struct Syntax {
    foreground: &'static str,
    keyword: &'static str,
    string: &'static str,
    escape: &'static str,
    comment: &'static str,
    function: &'static str,
    constant: &'static str,
    kind: &'static str,
    tag: &'static str,
    heading: &'static str,
}

const GITHUB_DARK: Syntax = Syntax {
    foreground: "#e6edf3",
    keyword: "#ff7b72",
    string: "#a5d6ff",
    escape: "#7ee787",
    comment: "#8b949e",
    function: "#d2a8ff",
    constant: "#79c0ff",
    kind: "#ffa657",
    tag: "#7ee787",
    heading: "#79c0ff",
};

const GITHUB_LIGHT: Syntax = Syntax {
    foreground: "#1f2328",
    keyword: "#cf222e",
    string: "#0a3069",
    escape: "#116329",
    comment: "#6e7781",
    function: "#8250df",
    constant: "#0550ae",
    kind: "#953800",
    tag: "#116329",
    heading: "#0550ae",
};

const NORD: Syntax = Syntax {
    foreground: "#d8dee9",
    keyword: "#81a1c1",
    string: "#a3be8c",
    escape: "#ebcb8b",
    comment: "#616e88",
    function: "#88c0d0",
    constant: "#b48ead",
    kind: "#8fbcbb",
    tag: "#81a1c1",
    heading: "#88c0d0",
};

fn syntax(appearance: Appearance) -> &'static Syntax {
    match appearance {
        Appearance::Dark => &GITHUB_DARK,
        Appearance::Light => &GITHUB_LIGHT,
        Appearance::Nord => &NORD,
    }
}

fn colors(s: &Syntax) -> SyntaxColors {
    let c = |color: &str| serde_json::json!({ "color": color });
    serde_json::from_value(serde_json::json!({
        "attribute": c(s.constant),
        "boolean": c(s.constant),
        "comment": { "color": s.comment, "font_style": "italic" },
        "comment.doc": { "color": s.comment, "font_style": "italic" },
        "constant": c(s.constant),
        "constructor": c(s.kind),
        "embedded": c(s.foreground),
        "emphasis": { "color": s.foreground, "font_style": "italic" },
        "emphasis.strong": { "color": s.foreground, "font_weight": 700 },
        "enum": c(s.kind),
        "function": c(s.function),
        "keyword": c(s.keyword),
        "label": c(s.constant),
        "link_text": c(s.string),
        "link_uri": { "color": s.constant, "font_style": "italic" },
        "number": c(s.constant),
        "operator": c(s.keyword),
        "preproc": c(s.keyword),
        "property": c(s.constant),
        "punctuation": c(s.foreground),
        "punctuation.bracket": c(s.foreground),
        "punctuation.delimiter": c(s.foreground),
        "punctuation.list_marker": c(s.kind),
        "punctuation.special": c(s.keyword),
        "string": c(s.string),
        "string.escape": c(s.escape),
        "string.regex": c(s.string),
        "string.special": c(s.string),
        "string.special.symbol": c(s.constant),
        "tag": c(s.tag),
        "tag.doctype": c(s.keyword),
        "text.literal": c(s.string),
        "title": { "color": s.heading, "font_weight": 600 },
        "type": c(s.kind),
        "variable.special": c(s.constant),
        "variant": c(s.constant),
    }))
    .expect("static syntax palette is valid")
}

pub(super) fn highlight_theme(appearance: Appearance) -> Arc<HighlightTheme> {
    let base = if appearance == Appearance::Light {
        HighlightTheme::default_light()
    } else {
        HighlightTheme::default_dark()
    };
    let s = syntax(appearance);
    let mut theme = (*base).clone();
    theme.name = format!("Workspacer {}", appearance.label());
    theme.style.syntax = colors(s);
    theme.style.editor_foreground = serde_json::from_value(serde_json::json!(s.foreground)).ok();
    Arc::new(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_appearance_colors_keywords_strings_and_comments_distinctly() {
        for appearance in Appearance::ALL {
            let theme = highlight_theme(appearance);
            let color = |name| theme.style(name).and_then(|s| s.color).unwrap();
            assert_ne!(color("keyword"), color("string"), "{appearance:?}");
            assert_ne!(color("string"), color("comment"), "{appearance:?}");
            assert_ne!(color("function"), color("keyword"), "{appearance:?}");
        }
        assert_ne!(
            highlight_theme(Appearance::Dark).style("keyword"),
            highlight_theme(Appearance::Light).style("keyword")
        );
    }
}
