//! Design tokens: the one place native sizes, radii and washes are chosen.
//!
//! Colors come from the active [`Palette`] (built-in or custom theme); every
//! measurement here is theme-independent. UI code names a token instead of
//! writing a number, so two screens that mean the same thing look the same.
//! `tokens_are_used_instead_of_literals` (below) fails the build when a raw
//! font size, icon size or corner radius appears outside this file.
//!
//! See `apps/native/DESIGN_SYSTEM.md` for when to use which token, and
//! Settings → Appearance → Design system for a live gallery of them.

/// Type scale for chrome. Chat prose follows the reader's text-size setting
/// instead; these keep pages, cards and controls consistent.
pub(super) mod scale {
    /// Page titles (Settings, Projects, Changes…); `TITLE_SHORT` in short windows.
    pub const TITLE: f32 = 22.;
    pub const TITLE_SHORT: f32 = 18.;
    /// A section's title inside a page (the Settings category heading).
    pub const SUBTITLE: f32 = 18.;
    /// Card and section headings.
    pub const HEADING: f32 = 15.;
    /// Row titles: a session in the sidebar, a preference, a question.
    pub const LABEL: f32 = 14.;
    /// Body copy and control labels on pages.
    pub const BODY: f32 = 13.;
    /// Descriptions, metadata, notices and button labels.
    pub const META: f32 = 12.;
    /// Captions, chips and hints.
    pub const CAPTION: f32 = 11.;
    /// Uppercase overlines.
    pub const OVERLINE: f32 = 10.;
}

/// Icon sizes. Pair an icon with the text it sits beside: `SM` with
/// `META`, `MD` with `BODY`, `LG` with `LABEL`/`HEADING`.
pub(super) mod icon_size {
    /// Chips, captions and dense metadata rows.
    pub const XS: f32 = 11.;
    /// Quiet buttons and metadata.
    pub const SM: f32 = 12.;
    /// Button icons and notice tones.
    pub const MD: f32 = 13.;
    /// Navigation rows and title actions.
    pub const LG: f32 = 14.;
    /// Standalone icons in headers and empty states' small marks.
    pub const XL: f32 = 16.;
    /// Empty-state and hero marks.
    pub const HERO: f32 = 20.;
}

/// Corner radii that do not come from the palette. Surfaces use the
/// palette's `control_radius`, `panel_radius` and `composer_radius` (which a
/// custom theme can change); these are for small marks inside them.
pub(super) mod radius {
    /// Progress bars, carets and hairline marks.
    pub const HAIRLINE: f32 = 1.;
    /// Checkbox-sized marks and swatches inside previews.
    pub const MARK: f32 = 3.;
    /// Keycaps, small tags and preview blocks.
    pub const TAG: f32 = 4.;
    /// Inline badges that sit inside a control.
    pub const BADGE: f32 = 8.;
    /// A segmented group that may wrap onto two lines: round like a pill
    /// on one line, without turning into a lozenge on two.
    pub const SEGMENTED_GROUP: f32 = 18.;
}

/// Control geometry. Every button and segmented option of one size shares
/// a height, so a row of mixed controls lines up.
pub(super) mod control {
    /// Icon-only buttons, segmented options and compact chips.
    pub const HEIGHT_SM: f32 = 28.;
    /// Text buttons (primary, secondary, danger) and pill actions.
    pub const HEIGHT: f32 = 32.;
}

/// Washes of the palette's text color (see `chrome::tonal`) for tonal
/// controls. Rest → hover → pressed always step up in this order.
pub(super) mod alpha {
    /// A disabled tonal control, or a list row under the pointer: present,
    /// barely there.
    pub const FAINT: f32 = 0.04;
    /// Quiet fills: chips, inline code, a chosen list row.
    pub const SUBTLE: f32 = 0.06;
    /// A secondary button at rest.
    pub const REST: f32 = 0.07;
    /// A secondary button under the pointer.
    pub const HOVER: f32 = 0.12;
    /// A secondary button being pressed.
    pub const PRESSED: f32 = 0.17;
    /// Error/success/warning washes behind toned text (danger buttons, badges).
    pub const TONE: f32 = 0.08;
    pub const TONE_HOVER: f32 = 0.14;
    pub const TONE_PRESSED: f32 = 0.2;
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Raw numbers in these calls bypass the system. Allowed only here (the
    /// source of truth) and in tests, which may measure exact geometry.
    const GUARDED: [&str; 4] = ["text_size(px(", "rounded(px(", ".size(px(", "tonal(p, "];

    fn literal_after(line: &str, pattern: &str) -> bool {
        line.match_indices(pattern).any(|(at, _)| {
            line[at + pattern.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
    }

    fn scan_file(path: &Path, offenders: &mut Vec<String>) {
        let source = std::fs::read_to_string(path).unwrap();
        // Unit tests at the bottom of a file may pin exact geometry.
        let body = source.split("#[cfg(test)]").next().unwrap_or(&source);
        // An icon's size is guarded when the chain started at `Icon::new`
        // or an svg; layout boxes (`div().size(px(28.))`) are not icons.
        let mut in_icon = false;
        for (ix, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.contains("Icon::new(") || trimmed.contains("svg()") {
                in_icon = true;
            } else if !trimmed.starts_with('.') {
                in_icon = false;
            }
            for pattern in GUARDED {
                let guarded = pattern != ".size(px(" || in_icon;
                if guarded && literal_after(line, pattern) {
                    offenders.push(format!("{}:{}: {}", path.display(), ix + 1, trimmed));
                }
            }
        }
    }

    fn scan(dir: &Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    scan(&path, offenders);
                }
            } else if name.ends_with(".rs") && name != "tokens.rs" && name != "tests.rs" {
                scan_file(&path, offenders);
            }
        }
    }

    #[test]
    fn tokens_are_used_instead_of_literals() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        scan(&root.join("ui"), &mut offenders);
        scan_file(&root.join("ui.rs"), &mut offenders);
        assert!(
            offenders.is_empty(),
            "Use a design token (src/ui/tokens.rs) instead of a raw size:\n{}",
            offenders.join("\n")
        );
    }
}
