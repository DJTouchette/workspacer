//! Code highlighting colors that follow the native appearance. Dark and Light
//! use the same GitHub Default palettes as the desktop chat (shiki
//! github-dark-default / github-light-default); Nord uses the Nord syntax set.
//! The Omarchy ports use their themes' editor conventions (Tokyo Night,
//! Catppuccin, Gruvbox Material, Everforest) with Omarchy's colors.toml hues;
//! Latte's tokens are darkened slightly to stay readable on its code blocks.
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

const TOKYO_NIGHT: Syntax = Syntax {
    foreground: "#c0caf5",
    keyword: "#bb9af7",
    string: "#9ece6a",
    escape: "#89ddff",
    comment: "#6a73a0",
    function: "#7aa2f7",
    constant: "#ff9e64",
    kind: "#2ac3de",
    tag: "#f7768e",
    heading: "#7aa2f7",
};

const CATPPUCCIN_MOCHA: Syntax = Syntax {
    foreground: "#cdd6f4",
    keyword: "#cba6f7",
    string: "#a6e3a1",
    escape: "#f5c2e7",
    comment: "#9399b2",
    function: "#89b4fa",
    constant: "#fab387",
    kind: "#f9e2af",
    tag: "#f38ba8",
    heading: "#89b4fa",
};

const GRUVBOX_MATERIAL: Syntax = Syntax {
    foreground: "#d4be98",
    keyword: "#ea6962",
    string: "#a9b665",
    escape: "#e78a4e",
    comment: "#928374",
    function: "#89b482",
    constant: "#d3869b",
    kind: "#d8a657",
    tag: "#e78a4e",
    heading: "#7daea3",
};

const EVERFOREST: Syntax = Syntax {
    foreground: "#d3c6aa",
    keyword: "#e67e80",
    string: "#a7c080",
    escape: "#e69875",
    comment: "#859289",
    function: "#83c092",
    constant: "#d699b6",
    kind: "#dbbc7f",
    tag: "#e69875",
    heading: "#7fbbb3",
};

const CATPPUCCIN_LATTE: Syntax = Syntax {
    foreground: "#4c4f69",
    keyword: "#8839ef",
    string: "#3d7f26",
    escape: "#b8418e",
    comment: "#6c6f85",
    function: "#1e66f5",
    constant: "#c0480a",
    kind: "#9a5e0a",
    tag: "#d20f39",
    heading: "#1e66f5",
};

fn syntax(appearance: Appearance) -> &'static Syntax {
    match appearance {
        Appearance::Dark => &GITHUB_DARK,
        Appearance::Light => &GITHUB_LIGHT,
        Appearance::Nord => &NORD,
        Appearance::TokyoNight => &TOKYO_NIGHT,
        Appearance::Catppuccin => &CATPPUCCIN_MOCHA,
        Appearance::Gruvbox => &GRUVBOX_MATERIAL,
        Appearance::Everforest => &EVERFOREST,
        Appearance::CatppuccinLatte => &CATPPUCCIN_LATTE,
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
    let base = if appearance.is_dark() {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
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

    fn hex(color: &str) -> u32 {
        u32::from_str_radix(color.trim_start_matches('#'), 16).unwrap()
    }

    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let c = ((color >> shift) & 0xff) as f64 / 255.;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn over(rgba: u32, under: u32) -> u32 {
        let alpha = (rgba & 0xff) as f64 / 255.;
        let mix = |shift: u32| {
            let top = ((rgba >> (shift + 8)) & 0xff) as f64;
            let bottom = ((under >> shift) & 0xff) as f64;
            ((top * alpha + bottom * (1. - alpha)).round() as u32) << shift
        };
        mix(16) | mix(8) | mix(0)
    }

    /// Highlighted code stays readable on its block and the chat, and still
    /// reads once a selection tints the background beneath the glyphs.
    #[test]
    fn omarchy_syntax_tokens_read_on_code_blocks_and_under_selection() {
        for appearance in &Appearance::ALL[3..] {
            let p = appearance.palette();
            let s = syntax(*appearance);
            for (name, color) in [
                ("foreground", s.foreground),
                ("keyword", s.keyword),
                ("string", s.string),
                ("escape", s.escape),
                ("comment", s.comment),
                ("function", s.function),
                ("constant", s.constant),
                ("type", s.kind),
                ("tag", s.tag),
                ("heading", s.heading),
            ] {
                let (plain, selected) = if name == "comment" {
                    (3., 2.2)
                } else {
                    (4., 3.)
                };
                for (surface, fill) in [("code block", p.code_block), ("chat", p.chat)] {
                    let ratio = contrast(hex(color), fill);
                    assert!(
                        ratio >= plain,
                        "{appearance:?} {name} on {surface}: {ratio:.2}"
                    );
                    let ratio = contrast(hex(color), over(p.selection, fill));
                    assert!(
                        ratio >= selected,
                        "{appearance:?} selected {name} on {surface}: {ratio:.2}"
                    );
                }
            }
        }
        // The shipped themes keep their published palettes; selected code
        // must at least stay visible on them.
        for appearance in &Appearance::ALL[..3] {
            let p = appearance.palette();
            let s = syntax(*appearance);
            for color in [s.foreground, s.keyword, s.string, s.function, s.comment] {
                let ratio = contrast(hex(color), over(p.selection, p.code_block));
                assert!(ratio >= 1.8, "{appearance:?} selected {color}: {ratio:.2}");
            }
        }
    }
}
