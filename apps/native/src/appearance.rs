//! Native appearance preferences; independent of the hub's shared config.yaml.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::tokens::{Design, Tokens};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    Nord,
    /// Ported from Omarchy's stock themes (`/usr/share/omarchy/themes/<slug>/
    /// colors.toml`); slugs match Omarchy's so the names stay recognizable.
    TokyoNight,
    Catppuccin,
    Gruvbox,
    Everforest,
    CatppuccinLatte,
}

/// One theme's colours, plus the design's [`Tokens`] so anything handed a
/// palette can also read the type scale, spacing, radii and elevation.
/// [`Appearance::palette`] is the shipped (v1) look; use
/// [`Appearance::palette_for`] with the workspace's [`Design`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub base: u32,
    pub surface: u32,
    pub chat: u32,
    pub selected: u32,
    pub border: u32,
    pub text: u32,
    /// Chat body copy; one step below `text` so bold/italic read as emphasis.
    pub prose: u32,
    pub muted: u32,
    pub disabled: u32,
    pub accent: u32,
    pub primary: u32,
    pub primary_hover: u32,
    pub primary_pressed: u32,
    pub on_primary: u32,
    pub warning: u32,
    pub error: u32,
    pub success: u32,
    pub busy: u32,
    pub user: u32,
    pub code_inline: u32,
    pub code_block: u32,
    pub code_header: u32,
    /// Text-selection highlight as 0xRRGGBBAA. Translucent so selected text
    /// and syntax colors stay readable on every surface it crosses.
    pub selection: u32,
    pub shadow: u32,
    pub shadow_opacity: f32,
    pub control_radius: f32,
    pub panel_radius: f32,
    pub composer_radius: f32,
    /// Extra semantic colours the redesign's primitives use. Derived from the
    /// colours above in v1, where nothing shipped reads them.
    pub roles: Roles,
    pub tokens: Tokens,
}

/// Colour roles beyond the v1 set, as 0xRRGGBB. V1 derives each from an
/// existing colour; v2 themes may set their own (`Appearance::v2_colors`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roles {
    /// Background of a hovered row or quiet control.
    pub hover: u32,
    /// Background of a pressed row or quiet control.
    pub pressed: u32,
    /// A surface raised above `surface` (menus, popovers, dialogs).
    pub raised: u32,
    /// Secondary text: labels and descriptions, one step below `text`.
    pub text_secondary: u32,
    /// Tertiary text: timestamps and hints. Not the disabled colour.
    pub text_tertiary: u32,
    /// A border that must read on `surface` and `raised` alike.
    pub border_strong: u32,
    /// The keyboard focus ring.
    pub focus: u32,
    /// Informational tone (neither success nor warning).
    pub info: u32,
    /// The 1px edge of a card resting on the page. Light themes need it to
    /// separate white cards from a near-white canvas.
    pub card_border: u32,
    /// Provider glyphs and model names: neutral, never brand colour (v2).
    pub provider: u32,
    /// Monogram avatar fills: a curated ramp of eight hues at one OKLCH
    /// lightness per theme (R3.6), picked by a stable hash of the name
    /// (`Palette::avatar_fill`). Text on them is `avatar_text`, ≥ 4.5:1.
    pub avatar: [u32; AVATAR_TONES],
    /// The monogram's colour on every avatar fill.
    pub avatar_text: u32,
    /// The five status roles. Colour AND shape encode status; nothing else
    /// may borrow these hues (blue/accent means "interactive" only).
    pub status: StatusRoles,
}

/// The status roles of `design-v2-direction.md` R2.1, as 0xRRGGBB. Each
/// pairs with a shape in `ui::primitives::status_indicator`:
/// attention = filled dot (+ amber stripe on its container), fault = filled
/// dot / ring with bar, working = pulsing ring, done = check / filled dot,
/// idle = hollow dot. In v1 they alias the shipped status colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusRoles {
    /// Blocked on the user: approval, question, plan, a failure needing a
    /// decision. Amber. Also readable as text (≥ 4.5:1 in Light).
    pub attention: u32,
    /// Failed, stalled ("Not responding"), offline, error. Red.
    pub fault: u32,
    /// Actively running. Cyan/teal, never the accent hue.
    pub working: u32,
    /// Finished successfully. Muted green.
    pub done: u32,
    /// Nothing to do / ended. Tertiary grey.
    pub idle: u32,
}

/// How many avatar fills a palette carries (R3.6: a curated 8-hue ramp).
pub const AVATAR_TONES: usize = 8;

/// The avatar hues in OKLCH degrees: blue, sky, teal, green, olive, sand,
/// clay, rose. No purple (280–335°), which v2 retires.
const AVATAR_HUES: [f32; AVATAR_TONES] = [255., 225., 190., 150., 115., 80., 45., 15.];

/// The avatar ramp's chroma per mode: muted, so tiles never shout.
const AVATAR_CHROMA_DARK: f32 = 0.06;
const AVATAR_CHROMA_LIGHT: f32 = 0.05;

/// The eight avatar fills for a theme whose monogram colour is `text`: one
/// OKLCH lightness for every hue, as close to `start` as AA allows (every
/// fill ≥ 4.6:1 against `text`, a hair over the 4.5 floor).
fn avatar_ramp(text: u32, dark: bool, start: f32) -> [u32; AVATAR_TONES] {
    let chroma = if dark {
        AVATAR_CHROMA_DARK
    } else {
        AVATAR_CHROMA_LIGHT
    };
    let step = if dark { -0.01 } else { 0.01 };
    let mut lightness = start;
    loop {
        let ramp = AVATAR_HUES.map(|hue| oklch(lightness, chroma, hue));
        let fits = ramp.iter().all(|&fill| contrast(text, fill) >= 4.6);
        if fits || !(0.05..=0.98).contains(&lightness) {
            return ramp;
        }
        lightness += step;
    }
}

impl Roles {
    /// Placeholder in the palette literals; `palette()` derives the real values.
    const UNSET: Self = Self {
        hover: 0,
        pressed: 0,
        raised: 0,
        text_secondary: 0,
        text_tertiary: 0,
        border_strong: 0,
        focus: 0,
        info: 0,
        card_border: 0,
        provider: 0,
        avatar: [0; AVATAR_TONES],
        avatar_text: 0,
        status: StatusRoles {
            attention: 0,
            fault: 0,
            working: 0,
            done: 0,
            idle: 0,
        },
    };

    /// The v1 mapping: every role is an existing palette colour.
    fn derive(p: &Palette) -> Self {
        Self {
            hover: p.selected,
            pressed: p.border,
            raised: p.surface,
            text_secondary: p.muted,
            text_tertiary: p.disabled,
            border_strong: p.border,
            focus: p.accent,
            info: p.accent,
            card_border: p.border,
            provider: p.muted,
            avatar: [p.selected; AVATAR_TONES],
            avatar_text: p.text,
            status: StatusRoles {
                attention: p.warning,
                fault: p.error,
                working: p.busy,
                done: p.success,
                idle: p.disabled,
            },
        }
    }
}

/// The v2 colours of one theme. Everything else in the v2 palette is derived
/// from these in [`Appearance::palette_v2`].
struct V2Colors {
    /// Sidebar and window chrome.
    base: u32,
    /// The conversation and page canvas.
    chat: u32,
    /// Cards, code blocks, inputs on cards.
    surface: u32,
    /// Menus, popovers, dialogs, tooltips.
    raised: u32,
    /// The selected row: always a neutral fill, never a tint of the accent.
    selected: u32,
    hover: u32,
    pressed: u32,
    /// Dividers inside cards and between rows.
    border: u32,
    /// Inputs, popovers, secondary buttons.
    border_strong: u32,
    card_border: u32,
    /// Text tiers: primary, chat prose, secondary, tertiary.
    text: u32,
    prose: u32,
    secondary: u32,
    tertiary: u32,
    /// The one accent: links, focus, selection tint.
    accent: u32,
    /// Primary buttons and the text on them.
    primary: u32,
    on_primary: u32,
    /// Status roles (design-v2-direction R2.1), used only for status:
    /// attention amber (blocked on the user), fault red (failed, stalled,
    /// offline), working cyan/teal (distinct from the accent), done muted
    /// green. Idle is `tertiary`.
    attention: u32,
    fault: u32,
    done: u32,
    working: u32,
    /// The user's message bubble and inline code.
    user: u32,
    code_inline: u32,
}

/// `a` moved `t` (0–1) of the way to `b`, per channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// An OKLCH colour (lightness 0–1, chroma, hue in degrees) as 0xRRGGBB,
/// clipped to sRGB.
pub fn oklch(lightness: f32, chroma: f32, hue: f32) -> u32 {
    let (a, b) = {
        let h = hue.to_radians();
        (chroma * h.cos(), chroma * h.sin())
    };
    let l_ = lightness + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = lightness - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = lightness - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let linear = [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ];
    let byte = |v: f32| {
        let v = v.clamp(0., 1.);
        let v = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round().clamp(0., 255.) as u32
    };
    byte(linear[0]) << 16 | byte(linear[1]) << 8 | byte(linear[2])
}

impl Palette {
    /// The avatar tile colour for `name`: one of the theme's eight curated
    /// fills (`roles.avatar`), picked by a stable hash of the name, so a
    /// project keeps its colour across runs and machines.
    pub fn avatar_fill(&self, name: &str) -> u32 {
        self.roles.avatar[crate::format::avatar_index(name, AVATAR_TONES)]
    }

    /// The monogram colour on any avatar fill (≥ 4.5:1 on all eight).
    pub fn avatar_text(&self) -> u32 {
        self.roles.avatar_text
    }
}

impl Appearance {
    pub const ALL: [Self; 8] = [
        Self::Dark,
        Self::Light,
        Self::Nord,
        Self::TokyoNight,
        Self::Catppuccin,
        Self::Gruvbox,
        Self::Everforest,
        Self::CatppuccinLatte,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Nord => "Nord",
            Self::TokyoNight => "Tokyo Night",
            Self::Catppuccin => "Catppuccin Mocha",
            Self::Gruvbox => "Gruvbox",
            Self::Everforest => "Everforest",
            Self::CatppuccinLatte => "Catppuccin Latte",
        }
    }

    /// Light palettes take the component library's light mode, light
    /// switch thumbs and darker generated project colors.
    pub fn is_dark(self) -> bool {
        !matches!(self, Self::Light | Self::CatppuccinLatte)
    }

    /// The shipped (v1) palette, with [`Tokens::V1`].
    pub fn palette(self) -> Palette {
        let mut p = self.v1_colors();
        p.roles = Roles::derive(&p);
        p
    }

    /// This theme under `design`: the shipped palette, or the redesign's.
    pub fn palette_for(self, design: Design) -> Palette {
        match design {
            Design::V1 => self.palette(),
            Design::V2 => self.palette_v2(),
        }
    }

    /// The redesign's palette: per theme, so all eight keep working. Keeps
    /// v1's selection and shadow, takes every other colour from
    /// [`Self::v2_colors`], and derives the rest so the rules hold in every
    /// theme: `muted` is secondary text and `disabled` tertiary (v1 code
    /// drawn under v2 inherits the hierarchy), `busy` is the running blue
    /// (no purple), code blocks sit on the card surface, providers are
    /// tertiary grey. Radii follow [`Tokens::V2`] so the palette's radius
    /// fields and the token set never disagree.
    pub fn palette_v2(self) -> Palette {
        let mut p = self.v1_colors();
        let c = self.v2_colors();
        p.tokens = Tokens::V2;
        p.control_radius = p.tokens.radius.control;
        p.panel_radius = p.tokens.radius.panel;
        p.composer_radius = p.tokens.radius.composer;
        p.base = c.base;
        p.chat = c.chat;
        p.surface = c.surface;
        p.selected = c.selected;
        p.border = c.border;
        p.text = c.text;
        p.prose = c.prose;
        p.muted = c.secondary;
        p.disabled = c.tertiary;
        p.accent = c.accent;
        p.primary = c.primary;
        // Hover and press move away from the label colour, so the label
        // only gains contrast while the button is touched.
        let away = if luminance(c.on_primary) > 0.5 {
            0x000000
        } else {
            0xffffff
        };
        p.primary_hover = mix(c.primary, away, 0.1);
        p.primary_pressed = mix(c.primary, away, 0.2);
        p.on_primary = c.on_primary;
        p.warning = c.attention;
        p.error = c.fault;
        p.success = c.done;
        p.busy = c.working;
        p.user = c.user;
        p.code_inline = c.code_inline;
        p.code_block = c.surface;
        p.code_header = c.raised;
        let dark = self.is_dark();
        // r3-foundation: a curated 8-hue ramp at one lightness per theme,
        // as near the mode's tone as the theme's text allows at AA.
        let avatar = avatar_ramp(c.text, dark, if dark { 0.42 } else { 0.88 });
        p.roles = Roles {
            hover: c.hover,
            pressed: c.pressed,
            raised: c.raised,
            text_secondary: c.secondary,
            text_tertiary: c.tertiary,
            border_strong: c.border_strong,
            focus: c.accent,
            info: c.working,
            card_border: c.card_border,
            provider: c.tertiary,
            avatar,
            avatar_text: c.text,
            status: StatusRoles {
                attention: c.attention,
                fault: c.fault,
                working: c.working,
                done: c.done,
                idle: c.tertiary,
            },
        };
        p
    }

    /// The v2 colours per theme: quiet near-neutral surfaces, crisp borders,
    /// one accent and four status hues. Contrast floors are enforced in
    /// `tests::v2_meets_the_direction_floors`.
    fn v2_colors(self) -> V2Colors {
        match self {
            Self::Dark => V2Colors {
                base: 0x111214,
                chat: 0x0e0f11,
                surface: 0x17181b,
                raised: 0x1d1e22,
                selected: 0x24252a,
                hover: 0x1b1c20,
                pressed: 0x27282d,
                border: 0x25262b,
                border_strong: 0x33353b,
                card_border: 0x25262b,
                text: 0xededef,
                prose: 0xd2d3d7,
                secondary: 0xa5a6ac,
                tertiary: 0x8b8d95,
                accent: 0x6aa1f8,
                primary: 0x2563d8,
                on_primary: 0xffffff,
                attention: 0xe9b464,
                fault: 0xf27a7a,
                done: 0x72b48c,
                working: 0x45c4c8,
                user: 0x1d1e22,
                code_inline: 0x202125,
            },
            // Off-white canvas, white cards with a real 1px edge, amber-700
            // attention text and a neutral grey selection.
            Self::Light => V2Colors {
                base: 0xf6f6f7,
                chat: 0xfcfcfc,
                surface: 0xffffff,
                raised: 0xffffff,
                selected: 0xebebee,
                hover: 0xf1f1f3,
                pressed: 0xe6e6e9,
                border: 0xe7e7ea,
                border_strong: 0xd9d9de,
                card_border: 0xdedee3,
                text: 0x18181b,
                prose: 0x2c2c31,
                secondary: 0x54555c,
                tertiary: 0x66676f,
                accent: 0x1f5fd6,
                primary: 0x2563eb,
                on_primary: 0xffffff,
                attention: 0xb45309,
                fault: 0xc81e1e,
                done: 0x2f7a4f,
                working: 0x0d7a80,
                user: 0xf1f1f3,
                code_inline: 0xefeff1,
            },
            // Polar night surfaces with frost as the one accent: the primary
            // is Nord8 with nord0 text, bright enough to never read disabled.
            Self::Nord => V2Colors {
                base: 0x2e3440,
                chat: 0x292e39,
                surface: 0x343a47,
                raised: 0x3b4252,
                selected: 0x3f4655,
                hover: 0x393f4c,
                pressed: 0x454c5c,
                border: 0x414857,
                border_strong: 0x4c566a,
                card_border: 0x414857,
                text: 0xeceff4,
                prose: 0xd8dee9,
                secondary: 0xcbd1db,
                tertiary: 0xaeb6c3,
                accent: 0x81a1c1,
                primary: 0x81a1c1,
                on_primary: 0x1b1f27,
                attention: 0xebcb8b,
                fault: 0xe8a4aa,
                done: 0xa3be8c,
                working: 0x8fbcbb,
                user: 0x3b4252,
                code_inline: 0x3b4252,
            },
            Self::TokyoNight => V2Colors {
                base: 0x16161e,
                chat: 0x1a1b26,
                surface: 0x1f2030,
                raised: 0x24283b,
                selected: 0x292e42,
                hover: 0x232536,
                pressed: 0x2c3148,
                border: 0x292d3e,
                border_strong: 0x343a52,
                card_border: 0x292d3e,
                text: 0xc8d0f5,
                prose: 0xb0b8dc,
                secondary: 0xa8b0d6,
                tertiary: 0x8e96ba,
                accent: 0x7aa2f7,
                primary: 0x7aa2f7,
                on_primary: 0x16161e,
                attention: 0xe0af68,
                fault: 0xf7768e,
                done: 0x9ece6a,
                working: 0x73daca,
                user: 0x24283b,
                code_inline: 0x24283b,
            },
            Self::Catppuccin => V2Colors {
                base: 0x181825,
                chat: 0x1e1e2e,
                surface: 0x24273a,
                raised: 0x2a2b3d,
                selected: 0x313244,
                hover: 0x2a2b3d,
                pressed: 0x383a4e,
                border: 0x313244,
                border_strong: 0x45475a,
                card_border: 0x313244,
                text: 0xcdd6f4,
                prose: 0xbac2de,
                secondary: 0xadb5d0,
                tertiary: 0x979db4,
                accent: 0x89b4fa,
                primary: 0x89b4fa,
                on_primary: 0x1e1e2e,
                attention: 0xf9e2af,
                fault: 0xf38ba8,
                done: 0x8fc48b,
                working: 0x94e2d5,
                user: 0x2a2b3d,
                code_inline: 0x313244,
            },
            Self::Gruvbox => V2Colors {
                base: 0x1d2021,
                chat: 0x282828,
                surface: 0x2e2c2b,
                raised: 0x32302f,
                selected: 0x3a3735,
                hover: 0x32302f,
                pressed: 0x423e3b,
                border: 0x3c3836,
                border_strong: 0x504945,
                card_border: 0x3c3836,
                text: 0xddc7a1,
                prose: 0xd4be98,
                secondary: 0xcbb999,
                tertiary: 0xb0a08c,
                accent: 0x7daea3,
                primary: 0x7daea3,
                on_primary: 0x1d2021,
                attention: 0xd8a657,
                fault: 0xec7670,
                done: 0xa9b665,
                working: 0x89b482,
                user: 0x32302f,
                code_inline: 0x3a3735,
            },
            // Everforest's blue is the accent; green stays success only.
            Self::Everforest => V2Colors {
                base: 0x272e33,
                chat: 0x2d353b,
                surface: 0x323c41,
                raised: 0x374145,
                selected: 0x3d484d,
                hover: 0x374145,
                pressed: 0x434f54,
                border: 0x414b50,
                border_strong: 0x4f5b58,
                card_border: 0x414b50,
                text: 0xe0d5bc,
                prose: 0xd3c6aa,
                secondary: 0xc7c9b8,
                tertiary: 0xb4b7aa,
                accent: 0x7fbbb3,
                primary: 0x7fbbb3,
                on_primary: 0x232a2e,
                attention: 0xdbbc7f,
                fault: 0xf2949a,
                done: 0xa7c080,
                working: 0x83c092,
                user: 0x374145,
                code_inline: 0x3d484d,
            },
            Self::CatppuccinLatte => V2Colors {
                base: 0xe6e9ef,
                chat: 0xeff1f5,
                surface: 0xfbfbfd,
                raised: 0xffffff,
                selected: 0xd5d9e2,
                hover: 0xe4e7ed,
                pressed: 0xcdd1db,
                border: 0xd8dce4,
                border_strong: 0xc6cad5,
                card_border: 0xd3d7e0,
                text: 0x35384f,
                prose: 0x4c4f69,
                secondary: 0x4c4f66,
                tertiary: 0x5a5d72,
                accent: 0x1e66f5,
                primary: 0x1a5fe8,
                on_primary: 0xffffff,
                attention: 0x9a5400,
                fault: 0xc8102f,
                done: 0x3d7a3a,
                working: 0x157f86,
                user: 0xe4e7ed,
                code_inline: 0xe4e7ed,
            },
        }
    }

    /// Native semantic palettes. Dark keeps chrome close to the conversation
    /// surface so selection and content, rather than panel fills, lead the eye.
    fn v1_colors(self) -> Palette {
        match self {
            Self::Dark => Palette {
                base: 0x101113,
                surface: 0x17191c,
                chat: 0x0d0e10,
                selected: 0x23262b,
                border: 0x26292e,
                text: 0xe6e8ec,
                prose: 0xc9ccd2,
                muted: 0x92979f,
                disabled: 0x595e66,
                accent: 0x60a5fa,
                primary: 0x3b82f6,
                primary_hover: 0x2563eb,
                primary_pressed: 0x1d4ed8,
                on_primary: 0xffffff,
                warning: 0xe8b86d,
                error: 0xf87171,
                success: 0x43c59e,
                busy: 0xc084fc,
                user: 0x1b1e23,
                code_inline: 0x1e2024,
                code_block: 0x08090b,
                code_header: 0x16181b,
                selection: 0x3b82f659,
                shadow: 0x000000,
                shadow_opacity: 0.32,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            Self::Light => Palette {
                base: 0xf4f4f5,
                surface: 0xffffff,
                chat: 0xfafafa,
                selected: 0xdbeafe,
                border: 0xe4e4e7,
                text: 0x18181b,
                prose: 0x27272a,
                muted: 0x65656e,
                disabled: 0xa1a1aa,
                accent: 0x1d4ed8,
                primary: 0x2563eb,
                primary_hover: 0x1d4ed8,
                primary_pressed: 0x1e40af,
                on_primary: 0xffffff,
                warning: 0x946000,
                error: 0xdc2626,
                success: 0x16a34a,
                busy: 0x9333ea,
                user: 0xe9eef9,
                code_inline: 0xeceef2,
                code_block: 0xf4f4f5,
                code_header: 0xeaeaed,
                selection: 0x2563eb38,
                shadow: 0x18181b,
                shadow_opacity: 0.10,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            Self::Nord => Palette {
                base: 0x2e3440,
                surface: 0x3b4252,
                chat: 0x252a33,
                selected: 0x4c566a,
                // nord2: one step above the card surface (nord1), so dividers
                // inside cards stay visible; nord1 itself vanished on cards.
                border: 0x434c5e,
                text: 0xeceff4,
                prose: 0xd8dee9,
                muted: 0x8eabc7,
                // Nord's brightened comment tone: nord3 (0x4c566a) fell to
                // ~1.9:1 on the chat surface, faintest of the three themes.
                disabled: 0x616e88,
                accent: 0x88c0d0,
                primary: 0x81a1c1,
                primary_hover: 0x7395b8,
                primary_pressed: 0x6589ad,
                on_primary: 0x2e3440,
                warning: 0xebcb8b,
                error: 0xbf616a,
                success: 0xa3be8c,
                busy: 0xb48ead,
                user: 0x303843,
                code_inline: 0x323947,
                code_block: 0x1f232b,
                code_header: 0x2b313c,
                selection: 0x88c0d03d,
                shadow: 0x171b22,
                shadow_opacity: 0.28,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            // Omarchy tokyo-night: background 1a1b26 is the conversation,
            // dark_background the chrome, lighter_background raised cards.
            Self::TokyoNight => Palette {
                base: 0x13141c,
                surface: 0x24283b,
                chat: 0x1a1b26,
                selected: 0x292e42,
                border: 0x2f344a,
                text: 0xc0caf5,
                prose: 0xa9b1d6,
                muted: 0x787fa6,
                disabled: 0x565f89,
                accent: 0x7aa2f7,
                primary: 0x7aa2f7,
                primary_hover: 0x6d95ea,
                primary_pressed: 0x5f87dc,
                on_primary: 0x1a1b26,
                warning: 0xe0af68,
                error: 0xf7768e,
                success: 0x9ece6a,
                busy: 0xbb9af7,
                user: 0x1f2335,
                code_inline: 0x24283b,
                code_block: 0x13141c,
                code_header: 0x1d1f2c,
                selection: 0x7aa2f738,
                shadow: 0x0e0e14,
                shadow_opacity: 0.34,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            // Omarchy catppuccin (Mocha). Selection is Overlay 2 at ~30%, as
            // Catppuccin's style guide specifies; busy is Mocha's mauve.
            Self::Catppuccin => Palette {
                base: 0x161622,
                surface: 0x313244,
                chat: 0x1e1e2e,
                selected: 0x45475a,
                border: 0x3e4053,
                text: 0xcdd6f4,
                prose: 0xbac2de,
                muted: 0x9399b2,
                disabled: 0x6c7086,
                accent: 0x89b4fa,
                primary: 0x89b4fa,
                primary_hover: 0x7aa7f0,
                primary_pressed: 0x6b99e4,
                on_primary: 0x1e1e2e,
                warning: 0xf9e2af,
                error: 0xf38ba8,
                success: 0xa6e3a1,
                busy: 0xcba6f7,
                user: 0x28283b,
                code_inline: 0x313244,
                code_block: 0x161622,
                code_header: 0x232334,
                selection: 0x9399b245,
                shadow: 0x101019,
                shadow_opacity: 0.34,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            // Omarchy gruvbox (the Material variant: d4be98 on 282828).
            Self::Gruvbox => Palette {
                base: 0x1e1e1e,
                surface: 0x3c3836,
                chat: 0x282828,
                selected: 0x504945,
                border: 0x48423f,
                text: 0xddc7a1,
                prose: 0xd4be98,
                muted: 0xa89984,
                disabled: 0x7c6f64,
                accent: 0x7daea3,
                primary: 0x7daea3,
                primary_hover: 0x6fa095,
                primary_pressed: 0x619287,
                on_primary: 0x1e1e1e,
                warning: 0xd8a657,
                error: 0xea6962,
                success: 0xa9b665,
                busy: 0xd3869b,
                user: 0x32302f,
                code_inline: 0x3c3836,
                code_block: 0x1e1e1e,
                code_header: 0x2a2827,
                selection: 0x7daea334,
                shadow: 0x161616,
                shadow_opacity: 0.36,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            // Omarchy everforest (dark medium). Headings brighten fg d3c6aa,
            // which is prose here; dark_foreground is too faint for metadata,
            // so quiet text uses Everforest's grey1.
            Self::Everforest => Palette {
                base: 0x21272c,
                surface: 0x343f44,
                chat: 0x2d353b,
                selected: 0x3d484d,
                border: 0x475258,
                text: 0xe0d5bc,
                prose: 0xd3c6aa,
                muted: 0x9da9a0,
                disabled: 0x859289,
                accent: 0x7fbbb3,
                primary: 0xa7c080,
                primary_hover: 0x99b273,
                primary_pressed: 0x8ba466,
                on_primary: 0x2d353b,
                warning: 0xdbbc7f,
                error: 0xe67e80,
                success: 0xa7c080,
                busy: 0xd699b6,
                user: 0x343f44,
                code_inline: 0x3a464c,
                code_block: 0x232a2e,
                code_header: 0x283035,
                selection: 0x7fbbb32e,
                shadow: 0x181d20,
                shadow_opacity: 0.32,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
            // Omarchy catppuccin-latte on its eff1f5 background. Text, green
            // and yellow are darkened from Latte's own values, which fall
            // short of 7:1 / 3:1 on these light surfaces.
            Self::CatppuccinLatte => Palette {
                base: 0xf3f4f7,
                surface: 0xfbfbfd,
                chat: 0xeff1f5,
                selected: 0xccd0da,
                border: 0xd3d6df,
                text: 0x41445e,
                prose: 0x4c4f69,
                muted: 0x6c6f85,
                disabled: 0x8c8fa1,
                accent: 0x1e66f5,
                primary: 0x1e66f5,
                primary_hover: 0x1a5ad9,
                primary_pressed: 0x164ebd,
                on_primary: 0xffffff,
                warning: 0xa65f00,
                error: 0xd20f39,
                success: 0x2f7d1f,
                busy: 0x8839ef,
                user: 0xe2e7f5,
                code_inline: 0xe3e6ec,
                code_block: 0xe6e9ef,
                code_header: 0xdce0e8,
                selection: 0x1e66f530,
                shadow: 0x4c4f69,
                shadow_opacity: 0.12,
                control_radius: 12.,
                panel_radius: 16.,
                composer_radius: 24.,
                roles: Roles::UNSET,
                tokens: Tokens::V1,
            },
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(&self)?)?;
        Ok(())
    }
}

pub fn preference_path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.home_dir().join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.home_dir().join(".config"))
    };
    Some(base.join("workspacer/native-theme.json"))
}

/// WCAG relative luminance of 0xRRGGBB.
pub fn luminance(color: u32) -> f64 {
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

/// WCAG contrast ratio between two 0xRRGGBB colours (1–21).
pub fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_text_stays_legible_on_the_chat_surface() {
        // Timestamps and durations use `disabled`; keep every theme readable.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            let ratio = contrast(p.disabled, p.chat);
            assert!(ratio >= 2.4, "{appearance:?} disabled contrast {ratio:.2}");
            assert!(
                contrast(p.muted, p.chat) > ratio,
                "{appearance:?} muted ≤ disabled"
            );
        }
    }

    /// `rgba` (0xRRGGBBAA) composited over an opaque `under`.
    fn over(rgba: u32, under: u32) -> u32 {
        let alpha = (rgba & 0xff) as f64 / 255.;
        let mix = |shift: u32| {
            let top = ((rgba >> (shift + 8)) & 0xff) as f64;
            let bottom = ((under >> shift) & 0xff) as f64;
            ((top * alpha + bottom * (1. - alpha)).round() as u32) << shift
        };
        mix(16) | mix(8) | mix(0)
    }

    #[test]
    fn selected_text_stays_readable_on_every_surface() {
        // Selection is painted under the glyphs, translucent, so text keeps
        // its own color; it must still read against the tinted background
        // and the tint must be visible against the surface it sits on.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            let alpha = p.selection & 0xff;
            assert!(
                (0x20..0xa0).contains(&alpha),
                "{appearance:?} selection alpha {alpha:#x}"
            );
            for (name, fill) in [
                ("chat", p.chat),
                ("code block", p.code_block),
                ("user", p.user),
                ("surface", p.surface),
            ] {
                let tinted = over(p.selection, fill);
                for (text_name, text) in [("text", p.text), ("prose", p.prose)] {
                    let ratio = contrast(text, tinted);
                    assert!(
                        ratio >= 4.5,
                        "{appearance:?} {text_name} on selected {name}: {ratio:.2}"
                    );
                }
                assert!(
                    contrast(tinted, fill) >= 1.15,
                    "{appearance:?} selection invisible on {name}: {:.2}",
                    contrast(tinted, fill)
                );
            }
        }
    }

    /// Shortfalls the original Dark/Light/Nord palettes shipped with, kept
    /// so this change does not restyle them. Each still has its own floor;
    /// the Omarchy ports have none of these exceptions.
    const BASELINE_EXCEPTIONS: [(Appearance, &str, f64); 3] = [
        // White on the brand blue button.
        (Appearance::Dark, "primary label", 3.6),
        (Appearance::Light, "success on base", 2.95),
        // Nord's aurora red on nord1 cards.
        (Appearance::Nord, "error on surface", 2.4),
    ];

    #[test]
    fn every_theme_meets_core_contrast() {
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            assert_eq!(
                appearance.is_dark(),
                luminance(p.chat) < 0.4,
                "{appearance:?} mode disagrees with its conversation surface"
            );
            let check = |name: String, ratio: f64, required: f64| match BASELINE_EXCEPTIONS
                .iter()
                .find(|(a, n, _)| *a == appearance && *n == name)
            {
                Some((_, _, floor)) => {
                    assert!(
                        ratio >= *floor,
                        "{appearance:?} {name}: {ratio:.2} < {floor}"
                    )
                }
                None => assert!(
                    ratio >= required,
                    "{appearance:?} {name}: {ratio:.2} < {required}"
                ),
            };
            for (name, fill) in [("chat", p.chat), ("surface", p.surface), ("base", p.base)] {
                for (tone, color, required) in [
                    ("text", p.text, 7.),
                    ("prose", p.prose, 4.5),
                    ("muted", p.muted, 3.),
                    ("accent", p.accent, 3.),
                    ("warning", p.warning, 3.),
                    ("error", p.error, 3.),
                    ("success", p.success, 3.),
                ] {
                    check(format!("{tone} on {name}"), contrast(color, fill), required);
                }
            }
            check(
                "primary label".into(),
                contrast(p.on_primary, p.primary),
                4.5,
            );
        }
    }

    #[test]
    fn omarchy_ports_keep_their_signature_colors() {
        // Spot-check against /usr/share/omarchy/themes/<slug>/colors.toml.
        for (appearance, background, accent, red, green) in [
            (
                Appearance::TokyoNight,
                0x1a1b26,
                0x7aa2f7,
                0xf7768e,
                0x9ece6a,
            ),
            (
                Appearance::Catppuccin,
                0x1e1e2e,
                0x89b4fa,
                0xf38ba8,
                0xa6e3a1,
            ),
            (Appearance::Gruvbox, 0x282828, 0x7daea3, 0xea6962, 0xa9b665),
            (
                Appearance::Everforest,
                0x2d353b,
                0x7fbbb3,
                0xe67e80,
                0xa7c080,
            ),
            // Latte's green (40a02b) is 2.9:1 on its base; darkened to pass.
            (
                Appearance::CatppuccinLatte,
                0xeff1f5,
                0x1e66f5,
                0xd20f39,
                0x2f7d1f,
            ),
        ] {
            let p = appearance.palette();
            assert_eq!(
                (p.chat, p.accent, p.error, p.success),
                (background, accent, red, green),
                "{appearance:?}"
            );
        }
        let slugs: Vec<_> = Appearance::ALL[3..]
            .iter()
            .map(|a| serde_json::to_string(a).unwrap())
            .collect();
        assert_eq!(
            slugs,
            [
                "\"tokyo-night\"",
                "\"catppuccin\"",
                "\"gruvbox\"",
                "\"everforest\"",
                "\"catppuccin-latte\""
            ]
        );
    }

    #[test]
    fn card_dividers_show_on_every_surface() {
        // Rows inside cards and panels are separated by `border` drawn on
        // `surface`; equal colors (Nord once) make every divider vanish.
        for appearance in Appearance::ALL {
            let p = appearance.palette();
            for (name, fill) in [("surface", p.surface), ("chat", p.chat), ("base", p.base)] {
                assert!(
                    contrast(p.border, fill) >= 1.1,
                    "{appearance:?} border on {name}: {:.2}",
                    contrast(p.border, fill)
                );
            }
        }
    }

    #[test]
    fn preference_round_trip_and_invalid_input() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("wks-theme-test-{}-{nonce}", std::process::id()));
        let path = dir.join("native-theme.json");
        assert_eq!(Appearance::load(&path).unwrap(), Appearance::Dark);
        for appearance in Appearance::ALL {
            appearance.save(&path).unwrap();
            assert_eq!(Appearance::load(&path).unwrap(), appearance);
        }
        std::fs::write(&path, b"\"unknown-theme\"").unwrap();
        assert!(Appearance::load(&path).is_err());
        std::fs::write(&path, b"{").unwrap();
        assert!(Appearance::load(&path).is_err());
        assert!(Appearance::Nord.save(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn design_v1_is_the_shipped_palette_and_v2_covers_every_theme() {
        for appearance in Appearance::ALL {
            let v1 = appearance.palette_for(Design::V1);
            assert_eq!(v1, appearance.palette(), "{appearance:?}");
            assert_eq!(v1.tokens, Tokens::V1);
            let v2 = appearance.palette_for(Design::V2);
            assert_eq!(v2.tokens.design, Design::V2, "{appearance:?}");
            assert_eq!(v2.control_radius, v2.tokens.radius.control);
            assert_eq!(v2.panel_radius, v2.tokens.radius.panel);
            assert_eq!(v2.composer_radius, v2.tokens.radius.composer);
        }
    }

    /// Hue (degrees) and HSL saturation of 0xRRGGBB.
    fn hue_saturation(color: u32) -> (f64, f64) {
        let [r, g, b] = [16, 8, 0].map(|s| ((color >> s) & 0xff) as f64 / 255.);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        let l = (max + min) / 2.;
        if d == 0. {
            return (0., 0.);
        }
        let s = d / (1. - (2. * l - 1.).abs());
        let h = if max == r {
            60. * ((g - b) / d).rem_euclid(6.)
        } else if max == g {
            60. * ((b - r) / d + 2.)
        } else {
            60. * ((r - g) / d + 4.)
        };
        (h, s)
    }

    /// The direction's floors (`docs/design-v2-direction.md` §1) in all
    /// eight themes.
    #[test]
    fn v2_meets_the_direction_floors() {
        for appearance in Appearance::ALL {
            let p = appearance.palette_v2();
            let r = p.roles;
            let floor = |name: &str, fg: u32, bg: u32, min: f64| {
                let ratio = contrast(fg, bg);
                assert!(ratio >= min, "{appearance:?} v2 {name}: {ratio:.2} < {min}");
            };
            assert_eq!(appearance.is_dark(), luminance(p.chat) < 0.4);
            for (surface, bg) in [
                ("chat", p.chat),
                ("surface", p.surface),
                ("base", p.base),
                ("raised", r.raised),
            ] {
                floor(&format!("text on {surface}"), p.text, bg, 4.5);
                floor(&format!("prose on {surface}"), p.prose, bg, 4.5);
                floor(
                    &format!("secondary on {surface}"),
                    r.text_secondary,
                    bg,
                    4.5,
                );
                // R2.2: meta text (tertiary) is ≥ 4.5:1 everywhere.
                floor(&format!("tertiary on {surface}"), r.text_tertiary, bg, 4.5);
                for (tone, color) in [
                    ("accent", p.accent),
                    ("attention", p.warning),
                    ("running", p.busy),
                    ("success", p.success),
                    ("error", p.error),
                ] {
                    floor(&format!("{tone} on {surface}"), color, bg, 3.);
                }
            }
            floor("secondary on selected", r.text_secondary, p.selected, 4.);
            floor("primary button on surface", p.primary, p.surface, 3.);
            floor("primary button on chat", p.primary, p.chat, 3.);
            floor("on-accent label", p.on_primary, p.primary, 4.5);
            floor(
                "on-accent label (hover)",
                p.on_primary,
                p.primary_hover,
                4.5,
            );
            floor(
                "on-accent label (pressed)",
                p.on_primary,
                p.primary_pressed,
                4.5,
            );
            for (name, fg, bg) in [
                ("border on surface", p.border, p.surface),
                ("border on chat", p.border, p.chat),
                ("card border on chat", r.card_border, p.chat),
                ("selected on base", p.selected, p.base),
                ("selected on chat", p.selected, p.chat),
            ] {
                floor(name, fg, bg, 1.1);
            }
            // Text tiers are a real hierarchy.
            assert!(contrast(p.text, p.chat) > contrast(r.text_secondary, p.chat));
            assert!(contrast(r.text_secondary, p.chat) > contrast(r.text_tertiary, p.chat));
            // Code blocks sit on the card surface; providers are neutral;
            // v1 code drawn under v2 inherits the tiers.
            assert_eq!(p.code_block, p.surface, "{appearance:?}");
            assert_eq!(r.provider, r.text_tertiary, "{appearance:?}");
            assert_eq!((p.muted, p.disabled), (r.text_secondary, r.text_tertiary));
            // One accent: links, focus and primary buttons share a hue, and
            // running is blue-ish; nothing is purple.
            let (accent_hue, _) = hue_saturation(p.accent);
            let (primary_hue, _) = hue_saturation(p.primary);
            assert!(
                (accent_hue - primary_hue).abs() < 12.,
                "{appearance:?} accent {accent_hue:.0} vs primary {primary_hue:.0}"
            );
            assert_eq!(r.focus, p.accent);
            for (name, color) in [
                ("accent", p.accent),
                ("primary", p.primary),
                ("running", p.busy),
                ("info", r.info),
            ]
            .into_iter()
            .chain(r.avatar.iter().map(|&c| ("avatar", c)))
            {
                let (hue, saturation) = hue_saturation(color);
                assert!(
                    saturation < 0.15 || !(255. ..=320.).contains(&hue),
                    "{appearance:?} {name} is purple ({hue:.0}°)"
                );
            }
            for (name, color) in [("selected", p.selected), ("hover", r.hover)] {
                let (_, saturation) = hue_saturation(color);
                // Neutral fills; the themed palettes keep a faint cast.
                let cap = if appearance == Appearance::Light {
                    0.1
                } else {
                    0.3
                };
                assert!(
                    saturation <= cap,
                    "{appearance:?} {name} saturation {saturation:.2}"
                );
            }
            // R2.1: the status roles are the palette's status colours, and
            // working is cyan/teal, never the accent's hue.
            let st = r.status;
            assert_eq!(
                (st.attention, st.fault, st.working, st.done, st.idle),
                (p.warning, p.error, p.busy, p.success, r.text_tertiary),
                "{appearance:?}"
            );
            let (working_hue, _) = hue_saturation(st.working);
            let gap = (working_hue - accent_hue).abs();
            assert!(
                gap.min(360. - gap) >= 25.,
                "{appearance:?} working {working_hue:.0}° too close to accent {accent_hue:.0}°"
            );
            // …and never reads as done: the themes whose only cyan is their
            // accent (Gruvbox, Everforest) take their aqua instead.
            let (done_hue, _) = hue_saturation(st.done);
            assert!(
                (working_hue - done_hue).abs() >= 25.,
                "{appearance:?} working {working_hue:.0}° too close to done {done_hue:.0}°"
            );
            // Five distinct roles.
            let all = [st.attention, st.fault, st.working, st.done, st.idle];
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert_ne!(a, b, "{appearance:?} status roles collide");
                }
            }
            // Avatar monograms read on every tile.
            for &tile in &r.avatar {
                floor("avatar text", p.text, tile, 4.5);
            }
        }
        // Light: cards get a visible edge on the canvas, amber is dark enough
        // to read as text, and the selected row is grey, not blue.
        let light = Appearance::Light.palette_v2();
        assert!(contrast(light.roles.card_border, light.surface) >= 1.25);
        assert!(contrast(light.warning, light.surface) >= 4.5);
        // R2.2: Light amber text sits near amber-700 (#B45309).
        assert_eq!(light.roles.status.attention, 0xb45309);
        // Nord: a primary that never reads as disabled, with a ≥ 4.5:1 label.
        let nord = Appearance::Nord.palette_v2();
        let (_, saturation) = hue_saturation(nord.primary);
        assert!(saturation >= 0.3, "Nord primary saturation {saturation:.2}");
        assert!(contrast(nord.primary, nord.surface) >= 4.);
        assert!(contrast(nord.on_primary, nord.primary) >= 4.5);
    }

    /// `fg` at `alpha` over `bg`, as an opaque colour.
    fn blend(fg: u32, alpha: f32, bg: u32) -> u32 {
        super::mix(bg, fg, alpha)
    }

    // r3-foundation
    /// R3.2 / R3.6: every theme's text, button, badge, status and avatar
    /// pairs, button fills included (Nord and Tokyo Night's Allow among them).
    #[test]
    fn every_theme_meets_the_round_three_contrast_pairs() {
        let w = Tokens::V2.wash;
        for appearance in Appearance::ALL {
            let p = appearance.palette_v2();
            let r = p.roles;
            let floor = |name: &str, fg: u32, bg: u32, min: f64| {
                let ratio = contrast(fg, bg);
                assert!(
                    ratio >= min,
                    "{appearance:?} {name}: {ratio:.2} < {min} (#{fg:06x} on #{bg:06x})"
                );
            };
            let surfaces = [
                ("chat", p.chat),
                ("surface", p.surface),
                ("base", p.base),
                ("raised", r.raised),
            ];
            // Three text greys, each AA on every surface and on a selected row.
            for (surface, bg) in surfaces.into_iter().chain([("selected", p.selected)]) {
                floor(&format!("text on {surface}"), p.text, bg, 4.5);
                floor(
                    &format!("secondary on {surface}"),
                    r.text_secondary,
                    bg,
                    4.5,
                );
                floor(&format!("tertiary on {surface}"), r.text_tertiary, bg, 4.5);
            }
            let greys = [p.text, r.text_secondary, r.text_tertiary];
            assert!(
                greys.windows(2).all(|g| g[0] != g[1]),
                "{appearance:?} text greys collapse"
            );
            for (surface, bg) in surfaces {
                // Button tiers: Primary's label on its fill (rest, hover,
                // pressed), Secondary and Tertiary text, Destructive's danger
                // text; the primary fill itself stands off the surface.
                for (state, fill) in [
                    ("rest", p.primary),
                    ("hover", p.primary_hover),
                    ("pressed", p.primary_pressed),
                ] {
                    floor(&format!("primary label ({state})"), p.on_primary, fill, 4.5);
                }
                floor(&format!("primary fill on {surface}"), p.primary, bg, 3.);
                floor(&format!("secondary button on {surface}"), p.text, bg, 4.5);
                floor(
                    &format!("tertiary button on {surface}"),
                    r.text_secondary,
                    bg,
                    4.5,
                );
                floor(&format!("destructive text on {surface}"), p.error, bg, 4.5);
                floor(
                    &format!("secondary edge on {surface}"),
                    r.border_strong,
                    bg,
                    1.2,
                );
                // Status words that carry their hue (Needs you, Failed, the red
                // chip) read as text; every glyph reads as a graphic.
                floor(
                    &format!("needs-you text on {surface}"),
                    r.status.attention,
                    bg,
                    4.5,
                );
                floor(
                    &format!("failed text on {surface}"),
                    r.status.fault,
                    bg,
                    4.5,
                );
                for (name, glyph) in [
                    ("working", r.status.working),
                    ("done", r.status.done),
                    ("idle/paused/offline", r.status.idle),
                ] {
                    floor(&format!("{name} glyph on {surface}"), glyph, bg, 3.);
                }
                // The neutral count badge: secondary text on its wash.
                floor(
                    &format!("neutral count on {surface}"),
                    r.text_secondary,
                    blend(p.text, w.pressed, bg),
                    4.5,
                );
            }
            // R4.3: the keycap inside the Primary button meets text contrast
            // (it is drawn in the label colour, no box).
            floor("keycap hint on primary", p.on_primary, p.primary, 4.5);
            // Off the fill it is tertiary text, AA on the card.
            floor("keycap hint on a card", r.text_tertiary, p.surface, 4.5);
            // Coloured count badges: the base colour on the status fill.
            floor("needs-you count", p.base, r.status.attention, 4.5);
            floor("failed count", p.base, r.status.fault, 4.5);
            // Avatars: eight distinct fills, AA monograms, one lightness.
            let mut fills = r.avatar.to_vec();
            fills.sort();
            fills.dedup();
            assert_eq!(
                fills.len(),
                AVATAR_TONES,
                "{appearance:?} avatar fills repeat"
            );
            for &tile in &r.avatar {
                floor("avatar monogram", p.avatar_text(), tile, 4.5);
            }
            let lum: Vec<f64> = r.avatar.iter().map(|&c| luminance(c)).collect();
            let (lo, hi) = lum
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
            assert!(
                contrast_of_luminance(hi, lo) < 1.6,
                "{appearance:?} avatar fills differ in weight"
            );
            // No tinted edges: borders are neutral greys (Light had peach).
            for (name, edge) in [
                ("border", p.border),
                ("border_strong", r.border_strong),
                ("card_border", r.card_border),
            ] {
                let (hue, saturation) = hue_saturation(edge);
                let warm = (10. ..=60.).contains(&hue);
                assert!(
                    saturation < 0.25 || !warm,
                    "{appearance:?} {name} reads peach (#{edge:06x})"
                );
            }
        }
        // Light is tuned on its own: neutral greys at AA, not v1's.
        let light = Appearance::Light.palette_v2();
        for edge in [
            light.border,
            light.roles.border_strong,
            light.roles.card_border,
        ] {
            assert!(hue_saturation(edge).1 < 0.1, "Light edge #{edge:06x}");
        }
    }

    fn contrast_of_luminance(a: f64, b: f64) -> f64 {
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}
