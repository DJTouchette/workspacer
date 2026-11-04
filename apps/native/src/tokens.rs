//! Design tokens: every non-colour measure the native client draws with.
//!
//! [`Tokens::V1`] reproduces the shipped look exactly; each value names the
//! literal it replaces. [`Tokens::V2`] is the redesign's set, selected by the
//! `design_v2` setting (Settings → Appearance → New design (preview)). It
//! starts as a copy of V1 so the preview is safe to switch on; designers tune
//! V2 here and nowhere else. Per-theme colours live on
//! [`crate::appearance::Palette`], which carries one of these sets in its
//! `tokens` field, so any function that takes a `Palette` reads tokens too.
//!
//! Rule for v2 work: every v2 style reads a token, never a literal. A missing
//! measure is added here (to both sets) first. See `docs/design-v2.md`.

/// Which design the client draws: the shipped one or the redesign preview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Design {
    #[default]
    V1,
    V2,
}

impl Design {
    /// The `design_v2` setting as a design.
    pub fn from_flag(v2: bool) -> Self {
        if v2 { Self::V2 } else { Self::V1 }
    }

    pub fn is_v2(self) -> bool {
        self == Self::V2
    }

    pub fn tokens(self) -> Tokens {
        match self {
            Self::V1 => Tokens::V1,
            Self::V2 => Tokens::V2,
        }
    }
}

/// Type sizes in logical px (before the interface-size zoom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    /// Page titles (Settings, Projects, Changes…); `title_short` in short windows.
    pub title: f32,
    pub title_short: f32,
    /// Card and section headings.
    pub heading: f32,
    /// Body copy and control labels on pages.
    pub body: f32,
    /// Descriptions, metadata, notices, button labels.
    pub meta: f32,
    /// Captions, chips and hints.
    pub caption: f32,
    /// Uppercase overlines, keycaps.
    pub overline: f32,
    /// gpui-component's base size before the reader's text-size setting
    /// replaces it (`configure_theme`).
    pub ui: f32,
    /// Code size before the reader's text-size setting replaces it.
    pub mono: f32,
    /// Line height for wrapped notices and body copy, as a multiple.
    pub line_height: f32,
}

/// Spacing steps in logical px. V1 has no spacing tokens; these are the
/// px literals the client already repeats, named so v2 can reuse them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
    pub xxxl: f32,
}

/// Icon sizes in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconSizes {
    /// Spinners and inline glyphs in captions.
    pub xs: f32,
    /// Quiet buttons, chips.
    pub sm: f32,
    /// Buttons, notices, icon buttons.
    pub md: f32,
    pub lg: f32,
    /// Empty-state and hero glyphs.
    pub xl: f32,
}

/// Corner radii in logical px. `full` is any radius larger than the shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    /// Keycaps and tiny tags.
    pub xs: f32,
    /// Inline code, small chips.
    pub sm: f32,
    /// Buttons, inputs, rows (the palette's `control_radius`).
    pub control: f32,
    /// Cards and panels (the palette's `panel_radius`).
    pub panel: f32,
    /// The floating composer (the palette's `composer_radius`).
    pub composer: f32,
    pub full: f32,
}

/// One drop shadow, coloured by the palette's `shadow` / `shadow_opacity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    /// Multiplies the palette's `shadow_opacity`.
    pub opacity: f32,
}

/// Elevation levels, lowest first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    /// Cards resting on a page (V1 draws none).
    pub raised: Shadow,
    /// Composer, menus, popovers (`chrome::floating_shadow`).
    pub floating: Shadow,
    /// Dialogs over a scrim.
    pub modal: Shadow,
    /// The tone glow around the title island (`chrome::tone_glow`).
    pub glow_blur: f32,
    pub glow_spread: f32,
    pub glow_alpha: f32,
    /// Scrim behind dialogs, as an alpha of the palette's shadow colour.
    pub scrim_alpha: f32,
}

/// Alphas for washes of the text colour (`chrome::tonal`) and of tone colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Washes {
    /// A disabled tonal control.
    pub disabled: f32,
    /// A tonal control at rest.
    pub rest: f32,
    pub hover: f32,
    pub pressed: f32,
    /// A tone colour (error, warning…) behind its own text: danger buttons, pills.
    pub tone_rest: f32,
    pub tone_hover: f32,
    pub tone_pressed: f32,
}

/// Border widths in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strokes {
    pub hairline: f32,
    /// The reserved keyboard focus ring (`interactive_control`).
    pub focus: f32,
}

/// Fixed control and layout measures in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The chat column's reading measure (`ui::CHAT_WIDTH`).
    pub chat_width: f32,
    /// Gutter either side of the chat column (`chat_column`'s `px_5`).
    pub chat_gutter: f32,
    /// The tall layout's session rail (`portrait::RAIL_WIDTH`).
    pub rail_width: f32,
    /// Rail rows and marks (`sidebar::RAIL_ROW` / `RAIL_SESSION` / `RAIL_CHILD`).
    pub rail_row: f32,
    pub rail_session: f32,
    pub rail_child: f32,
    /// Gap between the rail and its flyout (`portrait::FLYOUT_GAP`).
    pub flyout_gap: f32,
    /// The sidebar's default width (`Settings::default().sidebar_width`).
    pub sidebar_width: f32,
    /// Pill and composer buttons (`chrome::pill_button`).
    pub control_height: f32,
    /// Round icon buttons (`chrome::icon_button`).
    pub icon_button: f32,
    /// Status dots (`ui::status_dot`).
    pub status_dot: f32,
    /// Meter track thickness.
    pub meter_height: f32,
    /// List rows (menus, pickers, file lists).
    pub row_height: f32,
    /// Popover and menu widths.
    pub menu_width: f32,
    /// Dialog width.
    pub dialog_width: f32,
}

/// Every non-colour token. Copy, so it rides along on [`Palette`].
///
/// [`Palette`]: crate::appearance::Palette
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub design: Design,
    pub text: TypeScale,
    pub space: Spacing,
    pub icon: IconSizes,
    pub radius: Radii,
    pub elevation: Elevation,
    pub wash: Washes,
    pub stroke: Strokes,
    pub layout: Layout,
}

impl Tokens {
    /// The shipped look. Changing a value here changes the default app.
    pub const V1: Self = Self {
        design: Design::V1,
        text: TypeScale {
            title: 22.,
            title_short: 18.,
            heading: 15.,
            body: 13.,
            meta: 12.,
            caption: 11.,
            overline: 10.,
            ui: 15.,
            mono: 13.,
            line_height: 1.45,
        },
        space: Spacing {
            xxs: 2.,
            xs: 4.,
            sm: 6.,
            md: 8.,
            lg: 12.,
            xl: 16.,
            xxl: 24.,
            xxxl: 32.,
        },
        icon: IconSizes {
            xs: 11.,
            sm: 12.,
            md: 13.,
            lg: 14.,
            xl: 20.,
        },
        radius: Radii {
            xs: 4.,
            sm: 6.,
            control: 12.,
            panel: 16.,
            composer: 24.,
            full: 9999.,
        },
        elevation: Elevation {
            raised: Shadow {
                offset_y: 0.,
                blur: 0.,
                spread: 0.,
                opacity: 0.,
            },
            floating: Shadow {
                offset_y: 6.,
                blur: 20.,
                spread: -4.,
                opacity: 1.,
            },
            modal: Shadow {
                offset_y: 6.,
                blur: 20.,
                spread: -4.,
                opacity: 1.,
            },
            glow_blur: 22.,
            glow_spread: 2.,
            glow_alpha: 0.45,
            scrim_alpha: 0.4,
        },
        wash: Washes {
            disabled: 0.04,
            rest: 0.07,
            hover: 0.12,
            pressed: 0.17,
            tone_rest: 0.08,
            tone_hover: 0.14,
            tone_pressed: 0.2,
        },
        stroke: Strokes {
            hairline: 1.,
            focus: 2.,
        },
        layout: Layout {
            chat_width: 900.,
            chat_gutter: 20.,
            rail_width: 56.,
            rail_row: 44.,
            rail_session: 30.,
            rail_child: 22.,
            flyout_gap: 6.,
            sidebar_width: 304.,
            control_height: 32.,
            icon_button: 28.,
            status_dot: 6.,
            meter_height: 4.,
            row_height: 30.,
            menu_width: 240.,
            dialog_width: 440.,
        },
    };

    /// The redesign. A copy of V1 until designers tune it.
    pub const V2: Self = Self {
        design: Design::V2,
        ..Self::V1
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_selects_the_token_set() {
        assert_eq!(Design::from_flag(false).tokens(), Tokens::V1);
        assert_eq!(Design::from_flag(true).tokens(), Tokens::V2);
        assert!(Design::from_flag(true).is_v2());
    }
}
