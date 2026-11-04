//! Design tokens: every non-colour measure the native client draws with.
//!
//! [`Tokens::V1`] reproduces the shipped look exactly; each value names the
//! literal it replaces (fields v1 never draws carry the v2 value and say so).
//! [`Tokens::V2`] is the redesign's set, selected by the `design_v2` setting
//! (Settings → Appearance → New design (preview)): a 12/13/14/15/17/20/24
//! type scale at weights 400/500/600 with tabular numerals for figures, two control
//! heights (28/36), two radii (8 for controls, full for chips and badges),
//! quiet elevation and narrower reading widths. Designers tune V2 here and
//! nowhere else. Per-theme colours live on
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
    /// Section headings (v2: 16).
    pub heading: f32,
    /// Card, row and dialog titles (v2: 14).
    pub subheading: f32,
    /// Emphasised body: decision headlines, the composer's input (v2: 15;
    /// v1 never draws it and carries the v2 value).
    pub lead: f32,
    /// Body copy and control labels on pages.
    pub body: f32,
    /// Descriptions, metadata, notices, button labels.
    pub meta: f32,
    /// Captions, chips and hints.
    pub caption: f32,
    /// Keycaps and compact meta (v2: 12, the floor; nothing is uppercase).
    pub overline: f32,
    /// gpui-component's base size before the reader's text-size setting
    /// replaces it (`configure_theme`).
    pub ui: f32,
    /// Code size before the reader's text-size setting replaces it.
    pub mono: f32,
    /// Line height for wrapped notices and body copy, as a multiple.
    pub line_height: f32,
}

/// Font weights as CSS numbers (`FontWeight(t.weight.medium)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// Body copy.
    pub regular: f32,
    /// Labels, buttons, chips.
    pub medium: f32,
    /// Headings and titles.
    pub semibold: f32,
    /// Page titles (v1 draws them bold; v2 caps weights at 600).
    pub title: f32,
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
    /// Between page sections and around hero states.
    pub huge: f32,
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
    /// The edge of a "waiting on you" container: attention at this alpha
    /// (v2: 0.35; v1 never draws it).
    pub attention_edge: f32,
    /// A control that yields to a pending decision (the composer while
    /// something waits on the user), as an opacity (v2: 0.6; v1: 1).
    pub yielded: f32,
}

/// Border widths in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strokes {
    pub hairline: f32,
    /// The reserved keyboard focus ring (`interactive_control`).
    pub focus: f32,
    /// The attention stripe on a "waiting on you" container's left edge
    /// (v2: 3; v1 never draws it).
    pub stripe: f32,
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
    /// Wider reading measures (Settings → reading width); `chat_width` is
    /// the comfortable one. `reading_full` is larger than any window.
    pub reading_wide: f32,
    pub reading_full: f32,
    /// Shared max width of full-page content (`page_view`'s `CHAT_WIDTH`).
    pub page_width: f32,
    /// Space above a page header, below the caption (`page_scroller`).
    pub page_top: f32,
    /// Side padding of a page (`page_scroller`'s 24px).
    pub page_gutter: f32,
    /// Sidebar session rows: comfortable (two lines) and compact (one).
    pub sidebar_row: f32,
    pub sidebar_row_compact: f32,
    /// The sidebar's single footer row, at most.
    pub footer_height: f32,
    /// Gap between the rail and its flyout (`portrait::FLYOUT_GAP`).
    pub flyout_gap: f32,
    /// The sidebar's default width (`Settings::default().sidebar_width`).
    pub sidebar_width: f32,
    /// Large buttons and inputs (v1: `chrome::pill_button`; v2: 36).
    pub control_height: f32,
    /// Small buttons in rows, toolbars and cards (v2: 28).
    pub control_height_sm: f32,
    /// Round icon buttons (`chrome::icon_button`).
    pub icon_button: f32,
    /// Status dots (`ui::status_dot`).
    pub status_dot: f32,
    /// The fixed box a status indicator (dot, pulse, spinner) sits in.
    pub status_box: f32,
    /// A fixed status column, so titles after it share one left edge.
    pub status_column: f32,
    /// Monogram avatar tiles: in rows, and on cards and headers.
    pub avatar: f32,
    pub avatar_lg: f32,
    /// Height of the fade under floating chrome (title capsule, Jump to latest).
    pub fade: f32,
    /// Meter track thickness.
    pub meter_height: f32,
    /// List rows (menus, pickers, file lists).
    pub row_height: f32,
    /// Popover and menu widths.
    pub menu_width: f32,
    /// Dialog width.
    pub dialog_width: f32,
    // area-pages
    /// Settings: the category rail's width and the whole page's max width
    /// (rail plus a content column as wide as `page_width`).
    pub settings_rail: f32,
    pub settings_width: f32,
    /// Settings → Appearance: a theme preview tile's minimum width.
    pub theme_tile: f32,
}

/// Durations of looping indicators, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// One beat of the Working pulse (v1: the brand spinner's 1600).
    pub pulse_ms: f32,
}

/// Every non-colour token. Copy, so it rides along on [`Palette`].
///
/// [`Palette`]: crate::appearance::Palette
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub design: Design,
    pub text: TypeScale,
    pub weight: Weights,
    pub space: Spacing,
    pub icon: IconSizes,
    pub radius: Radii,
    pub elevation: Elevation,
    pub wash: Washes,
    pub stroke: Strokes,
    pub layout: Layout,
    pub motion: Motion,
    // area-panels
    pub panels: Panels,
}

// area-panels
/// Measures and tints of the side panels: the Changes dock and its diff,
/// the file viewer and the terminal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panels {
    /// One diff line (and one slot of the stacked diff list).
    pub diff_row: f32,
    /// Diff line-number gutter, per side.
    pub diff_gutter: f32,
    /// The +/− marker column.
    pub diff_marker: f32,
    /// The accent bar on a changed line's leading edge.
    pub diff_bar: f32,
    /// Background of an added or removed line: its tone at this alpha.
    pub diff_line_alpha: f32,
    /// The changed words inside a line (and its gutter): a stronger wash.
    pub diff_emphasis_alpha: f32,
    /// Docked side panel: narrowest, default share of the free width, widest.
    pub dock_min: f32,
    pub dock_share: f32,
    pub dock_max: f32,
    /// The grab strip on a dock's leading edge.
    pub dock_handle: f32,
    /// A panel's header bar.
    pub header_height: f32,
    /// Terminal line height as a multiple of its font size.
    pub terminal_line: f32,
    /// The terminal cursor's fill while focused.
    pub cursor_alpha: f32,
    /// The Changes page's file column.
    pub files_width: f32,
    /// How long a docked panel takes to slide in, in milliseconds.
    pub slide_ms: f32,
}

impl Tokens {
    /// The shipped look. Changing a value here changes the default app.
    pub const V1: Self = Self {
        design: Design::V1,
        text: TypeScale {
            title: 22.,
            title_short: 18.,
            heading: 15.,
            subheading: 14.,
            lead: 15.,
            body: 13.,
            meta: 12.,
            caption: 11.,
            overline: 10.,
            ui: 15.,
            mono: 13.,
            line_height: 1.45,
        },
        weight: Weights {
            regular: 400.,
            medium: 500.,
            semibold: 600.,
            title: 700.,
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
            huge: 48.,
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
            attention_edge: 0.35,
            yielded: 1.,
        },
        stroke: Strokes {
            hairline: 1.,
            focus: 2.,
            stripe: 3.,
        },
        layout: Layout {
            chat_width: 900.,
            chat_gutter: 20.,
            // V1 has one reading measure.
            reading_wide: 900.,
            reading_full: 900.,
            page_width: 900.,
            page_top: 24.,
            page_gutter: 24.,
            // V1's two-line rows; v1 has no compact density or footer row.
            sidebar_row: 64.,
            sidebar_row_compact: 28.,
            footer_height: 44.,
            rail_width: 56.,
            rail_row: 44.,
            rail_session: 30.,
            rail_child: 22.,
            flyout_gap: 6.,
            sidebar_width: 304.,
            control_height: 32.,
            control_height_sm: 28.,
            icon_button: 28.,
            status_dot: 6.,
            // Unused by v1 (it shows the brand spinner and ragged pills).
            status_box: 12.,
            status_column: 112.,
            avatar: 24.,
            avatar_lg: 32.,
            // `DOCK_FADE`.
            fade: 12.,
            meter_height: 4.,
            row_height: 30.,
            menu_width: 240.,
            dialog_width: 440.,
            // area-pages: `settings.rs` (rail 200, page 920, tiles 148).
            settings_rail: 200.,
            settings_width: 920.,
            theme_tile: 148.,
        },
        motion: Motion { pulse_ms: 1600. },
        // area-panels: v1 draws none of these but `diff_row`,
        // `diff_gutter`, `diff_marker`, `terminal_line` and `cursor_alpha`
        // (the shipped review and terminal literals); the rest carry v2.
        panels: Panels {
            diff_row: 20.,
            diff_gutter: 44.,
            diff_marker: 16.,
            diff_bar: 2.,
            diff_line_alpha: 0.14,
            diff_emphasis_alpha: 0.32,
            dock_min: 360.,
            dock_share: 0.5,
            dock_max: 760.,
            dock_handle: 6.,
            header_height: 36.,
            terminal_line: 1.35,
            cursor_alpha: 0.55,
            files_width: 300.,
            slide_ms: 240.,
        },
    };

    /// The redesign: calm by default, dense on demand (`docs/design-v2-direction.md`).
    pub const V2: Self = Self {
        design: Design::V2,
        text: TypeScale {
            // r2-systems: the 12/13/14/15/17/20/24 scale (R2.2).
            title: 24.,
            title_short: 20.,
            heading: 17.,
            subheading: 14.,
            lead: 15.,
            body: 13.,
            meta: 12.,
            caption: 12.,
            overline: 12.,
            ui: 14.,
            mono: 13.,
            line_height: 1.5,
        },
        weight: Weights {
            regular: 400.,
            medium: 500.,
            semibold: 600.,
            title: 600.,
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
            huge: 48.,
        },
        icon: IconSizes {
            xs: 12.,
            sm: 14.,
            md: 16.,
            lg: 20.,
            xl: 24.,
        },
        radius: Radii {
            xs: 4.,
            sm: 6.,
            control: 8.,
            panel: 12.,
            composer: 16.,
            full: 9999.,
        },
        elevation: Elevation {
            raised: Shadow {
                offset_y: 1.,
                blur: 2.,
                spread: 0.,
                opacity: 0.35,
            },
            floating: Shadow {
                offset_y: 8.,
                blur: 24.,
                spread: -6.,
                opacity: 1.,
            },
            modal: Shadow {
                offset_y: 16.,
                blur: 48.,
                spread: -12.,
                opacity: 1.25,
            },
            glow_blur: 16.,
            glow_spread: 0.,
            glow_alpha: 0.3,
            scrim_alpha: 0.5,
        },
        wash: Washes {
            disabled: 0.04,
            rest: 0.06,
            hover: 0.09,
            pressed: 0.13,
            tone_rest: 0.12,
            tone_hover: 0.18,
            tone_pressed: 0.24,
            attention_edge: 0.35,
            yielded: 0.6,
        },
        stroke: Strokes {
            hairline: 1.,
            focus: 2.,
            stripe: 3.,
        },
        layout: Layout {
            chat_width: 760.,
            chat_gutter: 24.,
            reading_wide: 1040.,
            reading_full: 9999.,
            page_width: 880.,
            page_top: 40.,
            page_gutter: 32.,
            sidebar_row: 44.,
            sidebar_row_compact: 28.,
            footer_height: 44.,
            rail_width: 56.,
            rail_row: 44.,
            rail_session: 30.,
            rail_child: 22.,
            flyout_gap: 6.,
            sidebar_width: 304.,
            control_height: 36.,
            control_height_sm: 28.,
            icon_button: 28.,
            status_dot: 6.,
            status_box: 12.,
            status_column: 112.,
            avatar: 24.,
            avatar_lg: 32.,
            fade: 32.,
            meter_height: 4.,
            row_height: 32.,
            menu_width: 240.,
            dialog_width: 440.,
            // area-pages
            settings_rail: 192.,
            settings_width: 1104.,
            theme_tile: 160.,
        },
        motion: Motion { pulse_ms: 1400. },
        // area-panels
        panels: Panels {
            diff_row: 22.,
            diff_gutter: 40.,
            diff_marker: 16.,
            diff_bar: 2.,
            diff_line_alpha: 0.10,
            diff_emphasis_alpha: 0.30,
            dock_min: 420.,
            dock_share: 0.5,
            dock_max: 1100.,
            dock_handle: 6.,
            header_height: 44.,
            terminal_line: 1.4,
            cursor_alpha: 0.7,
            files_width: 300.,
            slide_ms: 240.,
        },
    };
}

// area-transcript
/// Transcript measures: the conversation's own rhythm on top of the shared
/// scale. Read with `p.tokens.transcript()`; V1 names what the shipped
/// transcript draws, V2 is the redesign's (compact tool rows, hugging user
/// bubbles, quieter prose rhythm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranscriptTokens {
    /// User bubbles at most this share of the reading column.
    pub user_max: f32,
    /// One tool-call row (v2: CLI-dense, the small control height).
    pub tool_row: f32,
    /// Steps a work card shows before folding the rest behind "n earlier steps".
    pub visible_steps: usize,
    /// Vertical room around one transcript row (half above, half below).
    pub row_gap: f32,
    /// Extra room above a user turn: the turn boundary.
    pub turn_gap: f32,
    /// Tallest an opened tool's input/output gets before it scrolls.
    pub details_max: f32,
    /// A long dispatch brief's folded height.
    pub brief_max: f32,
    /// Prose line height, as a multiple.
    pub prose_line_height: f32,
    /// Between Markdown paragraphs, in rems of the reading size (v1 only).
    pub paragraph_gap: f32,
    /// r3-transcript: between Markdown paragraphs, in px (the 8px rhythm:
    /// 16). v2 converts it to rems of the reader's text size.
    pub paragraph_space: f32,
    /// Added to the reading size for Markdown headings h1, h2, h3, h4+.
    pub heading_steps: [f32; 4],
    /// Inline image previews.
    pub image_width: f32,
    pub image_height: f32,
    // r2-transcript
    /// The fluid column (content, gutters excluded): never narrower than
    /// `reading_min` while the pane allows, growing with the window up to
    /// `reading_max` (Comfortable) or `reading_wide` (Wide).
    pub reading_min: f32,
    pub reading_max: f32,
    pub reading_wide: f32,
    /// r3-transcript: one content width. Prose, tool rows, cards, tables and
    /// code all share the column, so there is no separate prose measure.
    /// Room above a Markdown heading that follows other blocks (32: the
    /// block before it keeps its `paragraph_space`, the rest is added), and
    /// below it (12).
    pub heading_space: f32,
    pub heading_gap: f32,
    /// An event row's leading icon column (bare glyph, no tile).
    pub event_icon: f32,
    /// Room between hairline-divided items inside one container.
    pub inset_row: f32,
}

impl Tokens {
    /// The transcript's measures for this design.
    pub const fn transcript(&self) -> TranscriptTokens {
        match self.design {
            Design::V1 => TranscriptTokens {
                user_max: 0.85,
                tool_row: 32.,
                visible_steps: 6,
                row_gap: 8.,
                turn_gap: 0.,
                details_max: 520.,
                brief_max: 132.,
                prose_line_height: 1.6,
                paragraph_gap: 0.5,
                paragraph_space: 16.,
                heading_steps: [9., 5., 2., 0.],
                image_width: 320.,
                image_height: 180.,
                // r2-transcript: v1 never draws these (v2 values).
                reading_min: 680.,
                reading_max: 1100.,
                reading_wide: 1280.,
                heading_space: 24.,
                heading_gap: 8.,
                event_icon: 16.,
                inset_row: 36.,
            },
            Design::V2 => TranscriptTokens {
                user_max: 0.7,
                tool_row: 28.,
                visible_steps: 4,
                row_gap: 6.,
                turn_gap: 16.,
                details_max: 480.,
                brief_max: 120.,
                prose_line_height: 1.6,
                paragraph_gap: 0.7,
                // r3-transcript: the 8px rhythm (16 between paragraphs,
                // 32 above a heading, 12 below it).
                paragraph_space: 16.,
                // r3-transcript: headings from the type scale over the 15px
                // reading size (`lead`): h1 20 (`title_short`), h2 17
                // (`heading`), h3+ 15 semibold.
                heading_steps: [5., 2., 0., 0.],
                image_width: 360.,
                image_height: 220.,
                // r2-transcript; r3-transcript: one capped column for
                // prose, tool rows, cards, tables and code.
                reading_min: 680.,
                reading_max: 800.,
                reading_wide: 1100.,
                heading_space: 32.,
                heading_gap: 12.,
                event_icon: 16.,
                inset_row: 36.,
            },
        }
    }
}

// r2-shell
/// The shell's own measures (sidebar inbox, rail, palette, tool rail). Read
/// with `p.tokens.shell()`; v1 draws none of them, so both designs carry
/// the v2 values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellTokens {
    /// An Idle/Ended session: one line.
    pub row_single: f32,
    /// A Needs you / Working session: title, then its reason or activity.
    pub row_double: f32,
    /// A global destination in the sidebar's navigation list.
    pub nav_row: f32,
    /// Idle sessions shown before "Show n more".
    pub idle_visible: usize,
    /// Rail tiles before "+n".
    pub rail_visible: usize,
    /// The usage meter in the footer.
    pub meter_width: f32,
    /// The command palette: width, its list's tallest, distance from the top.
    pub palette_width: f32,
    pub palette_list: f32,
    pub palette_top: f32,
    /// Rows the palette lists per scope before it stops.
    pub palette_limit: usize,
    // r3-shell
    /// The conversation's header strip (R3.3: 36–40): title on the left,
    /// the session toolbar on the right, the transcript scrolling beneath.
    pub header: f32,
    /// The toolbar shows its labels from this conversation width; below
    /// it, icons with tooltips.
    pub toolbar_labels_at: f32,
    /// "Jump to latest": its gap above the composer/decision stack and the
    /// scrim behind it (R3.3: 12, ≥ 32).
    pub jump_gap: f32,
    pub jump_scrim: f32,
}

impl Tokens {
    /// The shell's measures.
    pub const fn shell(&self) -> ShellTokens {
        ShellTokens {
            row_single: 30.,
            row_double: 46.,
            nav_row: 30.,
            idle_visible: 8,
            rail_visible: 8,
            meter_width: 48.,
            palette_width: 600.,
            palette_list: 420.,
            palette_top: 96.,
            palette_limit: 40,
            header: 40.,
            toolbar_labels_at: 1100.,
            jump_gap: 12.,
            jump_scrim: 40.,
        }
    }
}

// r3-fleet
/// The Fleet view's measures (R3.7): a responsive grid of session tiles.
/// v2 only; v1 never draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleetTokens {
    /// A tile is never narrower than this: the grid drops a column first.
    pub tile_min: f32,
    /// Every tile is at least this tall, so a row of them lines up whether
    /// or not one carries an ask.
    pub tile_height: f32,
    /// At most this many columns (3×3 at the widest).
    pub columns_max: usize,
    /// The grid's widest, centred in the page.
    pub grid_max: f32,
    /// The latest event lines a tile shows.
    pub event_lines: usize,
    /// One event line's height.
    pub event_row: f32,
}

impl Tokens {
    /// The Fleet view's measures.
    pub const fn fleet(&self) -> FleetTokens {
        FleetTokens {
            tile_min: 320.,
            tile_height: 136.,
            columns_max: 3,
            grid_max: 1440.,
            event_lines: 3,
            event_row: 22.,
        }
    }
}

// r3-foundation
/// The status language and signal measures (R3.1, R3.2): the 8px status
/// glyph, the attention bar, keycap hints and the warning/fault thresholds
/// of meters. Read with `p.tokens.signals()`; v1 draws none of them, so both
/// designs carry the v2 values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalTokens {
    /// The status glyph's drawn size: every state, everywhere (R3.1: 8).
    pub glyph: f32,
    /// The glyph's line width (rings, arcs, the check, the ✕, the dash).
    pub glyph_stroke: f32,
    /// The Working arc's sweep, in degrees of its ring.
    pub arc_degrees: f32,
    /// One turn of the Working arc, in milliseconds.
    pub spin_ms: f32,
    /// The Working ring's track behind the arc, as an alpha of its colour.
    pub track_alpha: f32,
    /// The one attention treatment: a bar this wide on the container's
    /// leading edge, clipped by its radius (R3.1: 3).
    pub attention_bar: f32,
    /// A status badge on an avatar's corner: the glyph sits on a disc of the
    /// surface colour this much wider than the glyph on each side.
    pub badge_ring: f32,
    /// Meters (usage, context) turn the attention colour at this share and
    /// the fault colour at `fault_at` (R3.5: 80%, 100%).
    pub warn_at: f32,
    pub fault_at: f32,
    /// A keycap hint's text, as an alpha of the label colour it trails.
    pub hint_alpha: f32,
}

impl Tokens {
    /// The status language's measures.
    pub const fn signals(&self) -> SignalTokens {
        SignalTokens {
            glyph: 8.,
            glyph_stroke: 1.5,
            arc_degrees: 100.,
            spin_ms: 1000.,
            track_alpha: 0.25,
            attention_bar: 3.,
            badge_ring: 2.,
            warn_at: 0.8,
            fault_at: 1.,
            hint_alpha: 0.65,
        }
    }
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

    #[test]
    fn v2_holds_the_direction_scale() {
        let t = Tokens::V2;
        // R2.2: 12 / 13 / 14 / 15 / 17 / 20 / 24, nothing smaller.
        let mut sizes = vec![
            t.text.title,
            t.text.title_short,
            t.text.heading,
            t.text.subheading,
            t.text.lead,
            t.text.body,
            t.text.meta,
            t.text.caption,
            t.text.overline,
            t.text.ui,
            t.text.mono,
        ];
        sizes.sort_by(f32::total_cmp);
        sizes.dedup();
        assert_eq!(sizes, [12., 13., 14., 15., 17., 20., 24.]);
        for w in [
            t.weight.regular,
            t.weight.medium,
            t.weight.semibold,
            t.weight.title,
        ] {
            assert!([400., 500., 600.].contains(&w), "weight {w}");
        }
        assert_eq!(
            (t.layout.control_height_sm, t.layout.control_height),
            (28., 36.)
        );
        assert_eq!(t.radius.control, 8.);
        assert!(t.radius.full > t.layout.control_height);
        assert!(t.layout.footer_height <= 44.);
        assert!(t.layout.chat_width < t.layout.reading_wide);
        assert!(t.layout.reading_wide < t.layout.reading_full);
        assert!(t.layout.status_dot < t.layout.status_box);
        // r3-foundation: one 8px glyph inside the status box.
        let sig = t.signals();
        assert_eq!(sig.glyph, 8.);
        assert!(sig.glyph < t.layout.status_box);
        assert_eq!(sig.attention_bar, t.stroke.stripe);
        assert!(sig.warn_at < sig.fault_at);
    }

    // area-transcript
    #[test]
    fn v2_transcript_is_denser_and_hugs() {
        let (v1, v2) = (Tokens::V1.transcript(), Tokens::V2.transcript());
        assert!(v2.user_max <= 0.7 && v2.user_max < v1.user_max);
        assert_eq!(v2.tool_row, Tokens::V2.layout.control_height_sm);
        assert!(v2.visible_steps < v1.visible_steps);
        assert!(v2.heading_steps.windows(2).all(|w| w[0] >= w[1]));
        // r3-transcript: headings come from the type scale, the rhythm is on
        // the 8px grid, and one capped column holds everything.
        let t = Tokens::V2;
        assert_eq!(t.text.lead + v2.heading_steps[0], t.text.title_short);
        assert_eq!(t.text.lead + v2.heading_steps[1], t.text.heading);
        assert_eq!(
            (v2.heading_space, v2.heading_gap, v2.paragraph_space),
            (32., 12., 16.)
        );
        assert!((1.5..=1.7).contains(&v2.prose_line_height));
        assert!(v2.reading_min < v2.reading_max && v2.reading_max < v2.reading_wide);
    }
}
