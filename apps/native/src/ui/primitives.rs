//! Shared v2 primitives: the redesign's buttons, pills, meters, rows, menus
//! and dialogs, each built only from a [`Palette`] and its [`Tokens`].
//!
//! Rules (see `docs/design-v2.md`):
//! - Every measure and colour comes from `p` (`p.tokens.*`, `p.roles.*`,
//!   palette colours). No literal sizes, colours or alphas in here.
//! - Primitives return plain `Div` / `Stateful<Div>`; callers add
//!   `.on_click(cx.listener(…))`, children and `.debug_selector(…)`, so
//!   adopting one never changes a screen's event wiring or test selectors.
//! - Variants are explicit enums ([`ButtonVariant`], [`Intent`]), never
//!   inferred from an element id.
//!
//! Screens adopt these behind `Workspace::design()`; until each area moves
//! onto them, nothing in v1 calls this module.
#![allow(dead_code)]

use super::chrome::{ControlTextStyle, interactive_control};
use super::*;
use gpui::{AnyElement, BoxShadow, ElementId, Hsla, relative};
use gpui_component::tooltip::Tooltip;
use wks_native::tokens::Shadow;

/// What a button does, which decides how loud it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonVariant {
    /// The one action a surface exists for: filled with the primary colour.
    Primary,
    /// Other actions: a tonal wash of the text colour.
    Secondary,
    /// Quiet actions in toolbars and rows: no fill until hovered.
    Ghost,
    /// Stops or removes something: an error wash, error text.
    Danger,
}

/// The meaning a pill, badge, meter or dot carries; its colour follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Neutral,
    Accent,
    Info,
    Success,
    Warning,
    Danger,
    /// An agent working.
    Busy,
}

impl Intent {
    /// The palette colour (0xRRGGBB) for this intent.
    pub(crate) fn color(self, p: &Palette) -> u32 {
        match self {
            Self::Neutral => p.roles.text_secondary,
            Self::Accent => p.accent,
            Self::Info => p.roles.info,
            Self::Success => p.success,
            Self::Warning => p.warning,
            Self::Danger => p.error,
            Self::Busy => p.busy,
        }
    }
}

/// `color` at `alpha`, for washes behind a tone (`p.tokens.wash.*`).
pub(crate) fn wash(color: u32, alpha: f32) -> Hsla {
    Hsla::from(rgb(color)).opacity(alpha)
}

/// A wash of the text colour: the fill of tonal controls on any surface.
pub(crate) fn tonal(p: &Palette, alpha: f32) -> Hsla {
    wash(p.text, alpha)
}

/// One elevation level as a GPUI shadow in the palette's shadow colour.
pub(crate) fn elevation(p: &Palette, level: Shadow) -> Vec<BoxShadow> {
    if level.opacity <= 0. {
        return Vec::new();
    }
    vec![BoxShadow {
        color: wash(p.shadow, p.shadow_opacity * level.opacity),
        offset: gpui::point(px(0.), px(level.offset_y)),
        blur_radius: px(level.blur),
        spread_radius: px(level.spread),
    }]
}

/// (rest, hover, pressed) backgrounds and (rest, hover) text for a variant.
fn button_colors(variant: ButtonVariant, enabled: bool, p: &Palette) -> ([Hsla; 3], [u32; 2]) {
    let w = p.tokens.wash;
    let solid = |c: u32| Hsla::from(rgb(c));
    if !enabled {
        let bg = match variant {
            ButtonVariant::Ghost => gpui::transparent_black(),
            ButtonVariant::Primary => solid(p.selected),
            ButtonVariant::Secondary | ButtonVariant::Danger => tonal(p, w.disabled),
        };
        return ([bg; 3], [p.disabled; 2]);
    }
    match variant {
        ButtonVariant::Primary => (
            [
                solid(p.primary),
                solid(p.primary_hover),
                solid(p.primary_pressed),
            ],
            [p.on_primary; 2],
        ),
        ButtonVariant::Secondary => (
            [tonal(p, w.rest), tonal(p, w.hover), tonal(p, w.pressed)],
            [p.text; 2],
        ),
        ButtonVariant::Ghost => (
            [
                gpui::transparent_black(),
                solid(p.roles.hover),
                solid(p.roles.pressed),
            ],
            [p.roles.text_secondary, p.text],
        ),
        ButtonVariant::Danger => (
            [
                wash(p.error, w.tone_rest),
                wash(p.error, w.tone_hover),
                wash(p.error, w.tone_pressed),
            ],
            [p.error; 2],
        ),
    }
}

/// Apply a variant's fill, text and hover/press states to a control.
fn button_states(
    control: Stateful<Div>,
    variant: ButtonVariant,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let ([rest, hover, pressed], [fg, fg_hover]) = button_colors(variant, enabled, p);
    control.bg(rest).text_color(rgb(fg)).when(enabled, |d| {
        d.hover_text_style(move |s| s.bg(hover).text_color(rgb(fg_hover)))
            .active_text_style(move |s| s.bg(pressed).text_color(rgb(fg_hover)))
    })
}

/// A labelled button, with an optional leading icon. Add `.on_click`.
///
/// ```ignore
/// primitives::button("save", "Save", None, ButtonVariant::Primary, dirty, &p)
///     .debug_selector(|| "save".into())
///     .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
/// ```
pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let control = interactive_control(div().id(id.into()), *p, enabled)
        .h(px(t.layout.control_height))
        .px(px(t.space.lg))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(t.space.sm))
        .flex_shrink_0()
        .rounded(px(t.radius.control))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight::MEDIUM)
        .when_some(icon, |d, icon| d.child(Icon::new(icon).size(px(t.icon.md))))
        .child(label.into());
    button_states(control, variant, enabled, p)
}

/// A square icon-only button with a tooltip naming it. Add `.on_click`.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    variant: ButtonVariant,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let control = interactive_control(div().id(id.into()), *p, enabled)
        .size(px(t.layout.icon_button))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(t.radius.control))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(Icon::new(icon).size(px(t.icon.md)));
    button_states(control, variant, enabled, p)
}

/// A static status label: tone-coloured text on its tone's wash, fully
/// rounded. For states ("Running", "Failed"), not actions.
pub(crate) fn pill(label: impl Into<SharedString>, intent: Intent, p: &Palette) -> Div {
    let t = p.tokens;
    let color = intent.color(p);
    div()
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .flex_shrink_0()
        .px(px(t.space.md))
        .py(px(t.space.xxs))
        .rounded(px(t.radius.full))
        .bg(wash(color, t.wash.tone_rest))
        .text_size(px(t.text.caption))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(label.into())
}

/// A toggleable filter or choice chip. Add `.on_click`.
pub(crate) fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let (bg, fg) = if selected {
        (Hsla::from(rgb(p.selected)), p.text)
    } else {
        (tonal(p, t.wash.rest), p.roles.text_secondary)
    };
    let hover = if selected {
        Hsla::from(rgb(p.selected))
    } else {
        tonal(p, t.wash.hover)
    };
    interactive_control(div().id(id.into()), *p, enabled)
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .flex_shrink_0()
        .px(px(t.space.md))
        .py(px(t.space.xxs))
        .rounded(px(t.radius.full))
        .bg(bg)
        .text_size(px(t.text.caption))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(if enabled { fg } else { p.disabled }))
        .when(enabled, |d| {
            d.hover_text_style(move |s| s.bg(hover).text_color(rgb(p.text)))
        })
        .child(label.into())
}

/// A small solid count or flag ("3", "New"): the tone colour filled, the
/// base colour as text.
pub(crate) fn badge(label: impl Into<SharedString>, intent: Intent, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .min_w(px(t.text.caption + t.space.sm))
        .px(px(t.space.xs))
        .rounded(px(t.radius.full))
        .bg(rgb(intent.color(p)))
        .text_size(px(t.text.overline))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.base))
        .child(label.into())
}

/// A horizontal meter: `fraction` (clamped to 0–1) of the track filled in
/// the intent's colour. Give it a width (`.w(…)` or `.flex_1()`).
pub(crate) fn meter(fraction: f32, intent: Intent, p: &Palette) -> Div {
    let t = p.tokens;
    let fraction = if fraction.is_finite() {
        fraction.clamp(0., 1.)
    } else {
        0.
    };
    div()
        .h(px(t.layout.meter_height))
        .w_full()
        .rounded(px(t.radius.full))
        .bg(tonal(p, t.wash.hover))
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded(px(t.radius.full))
                .bg(rgb(intent.color(p))),
        )
}

/// The centred placeholder for an empty list or page: icon, title and an
/// optional line of help. Add actions with `.child(button(…))`.
pub(crate) fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(t.space.md))
        .py(px(t.space.xxxl))
        .px(px(t.space.xl))
        .text_center()
        .child(
            div()
                .size(px(t.icon.xl + t.space.xl))
                .rounded(px(t.radius.full))
                .bg(tonal(p, t.wash.rest))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.roles.text_secondary))
                .child(Icon::new(icon).size(px(t.icon.xl))),
        )
        .child(
            div()
                .text_size(px(t.text.heading))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(title.into()),
        )
        .when_some(body, |d, body| {
            d.child(
                div()
                    .max_w(px(t.layout.dialog_width))
                    .text_size(px(t.text.meta))
                    .line_height(relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(body),
            )
        })
}

/// An uppercase overline naming a group of rows or cards.
pub(crate) fn section_label(label: impl Into<SharedString>, p: &Palette) -> Div {
    let t = p.tokens;
    let label: SharedString = label.into();
    div()
        .px(px(t.space.xs))
        .pb(px(t.space.xs))
        .text_size(px(t.text.overline))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.roles.text_tertiary))
        .child(SharedString::from(label.to_uppercase()))
}

/// The raised surface for grouped content, padded. `chrome::card` is v1's.
pub(crate) fn card(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.md))
        .p(px(t.space.xl))
        .rounded(px(t.radius.panel))
        .bg(rgb(p.surface))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.border))
        .shadow(elevation(p, t.elevation.raised))
}

/// A card's heading line: title, and an optional description below it.
pub(crate) fn card_header(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.xxs))
        .child(
            div()
                .text_size(px(t.text.heading))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(title.into()),
        )
        .when_some(description, |d, text| {
            d.child(
                div()
                    .text_size(px(t.text.meta))
                    .line_height(relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(text),
            )
        })
}

/// A selectable row in a list, menu or picker. Add children and `.on_click`.
pub(crate) fn list_row(
    id: impl Into<ElementId>,
    selected: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let (hover, pressed) = (p.roles.hover, p.roles.pressed);
    interactive_control(div().id(id.into()), *p, enabled)
        .w_full()
        .min_h(px(t.layout.row_height))
        .px(px(t.space.md))
        .flex()
        .items_center()
        .gap(px(t.space.md))
        .rounded(px(t.radius.control))
        .text_size(px(t.text.body))
        .text_color(rgb(if enabled { p.text } else { p.disabled }))
        .when(selected, |d| d.bg(rgb(p.selected)))
        .when(enabled && !selected, |d| {
            d.hover_text_style(move |s| s.bg(rgb(hover)))
                .active_text_style(move |s| s.bg(rgb(pressed)))
        })
}

/// The track of a segmented control; fill it with [`segment`]s.
pub(crate) fn segmented(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.xxs))
        .p(px(t.space.xxs))
        .rounded(px(t.radius.control))
        .bg(tonal(p, t.wash.rest))
}

/// One choice of a [`segmented`] control. Add `.on_click`.
pub(crate) fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    interactive_control(div().id(id.into()), *p, true)
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .h(px(t.layout.control_height - 2. * t.space.xxs))
        .px(px(t.space.lg))
        .rounded(px(t.radius.control - t.space.xxs))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight::MEDIUM)
        .when(selected, |d| {
            d.bg(rgb(p.roles.raised))
                .text_color(rgb(p.text))
                .shadow(elevation(p, t.elevation.raised))
        })
        .when(!selected, |d| {
            d.text_color(rgb(p.roles.text_secondary))
                .hover_text_style(move |s| s.text_color(rgb(p.text)))
        })
        .child(label.into())
}

/// A floating surface: popovers, hover cards, menus. Position it yourself
/// (`deferred(anchored()…)` or absolute) and add children.
pub(crate) fn popover(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .p(px(t.space.md))
        .rounded(px(t.radius.panel))
        .bg(rgb(p.roles.raised))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.roles.border_strong))
        .shadow(elevation(p, t.elevation.floating))
}

/// A menu: a popover of [`menu_item`]s at the menu width.
pub(crate) fn menu(p: &Palette) -> Div {
    let t = p.tokens;
    popover(p)
        .w(px(t.layout.menu_width))
        .p(px(t.space.xs))
        .gap(px(t.space.xxs))
}

/// One menu action; `danger` colours it as destructive. Add `.on_click`.
pub(crate) fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    danger: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let color = if !enabled {
        p.disabled
    } else if danger {
        p.error
    } else {
        p.text
    };
    list_row(id, false, enabled, p)
        .text_color(rgb(color))
        .when_some(icon, |d, icon| {
            d.child(
                Icon::new(icon)
                    .size(px(t.icon.md))
                    .text_color(rgb(if danger {
                        color
                    } else {
                        p.roles.text_secondary
                    })),
            )
        })
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
}

/// A hairline between groups of menu items.
pub(crate) fn menu_separator(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .h(px(t.stroke.hairline))
        .mx(px(t.space.xs))
        .my(px(t.space.xxs))
        .bg(rgb(p.border))
}

/// The dimmed layer behind a dialog, filling its (absolute) parent. Give it
/// an id and `.on_click` to dismiss, and the [`dialog`] as its child.
pub(crate) fn scrim(p: &Palette) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(wash(p.shadow, p.tokens.elevation.scrim_alpha))
}

/// A dialog panel: [`dialog_header`], body, then [`dialog_footer`].
pub(crate) fn dialog(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.xl))
        .w(px(t.layout.dialog_width))
        .max_w_full()
        .p(px(t.space.xxl))
        .rounded(px(t.radius.panel))
        .bg(rgb(p.roles.raised))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.roles.border_strong))
        .shadow(elevation(p, t.elevation.modal))
}

/// A dialog's title and optional description.
pub(crate) fn dialog_header(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.xs))
        .child(
            div()
                .text_size(px(t.text.title_short))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(title.into()),
        )
        .when_some(description, |d, text| {
            d.child(
                div()
                    .text_size(px(t.text.body))
                    .line_height(relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(text),
            )
        })
}

/// A dialog's action row, right-aligned: secondary actions, then primary.
pub(crate) fn dialog_footer(p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(p.tokens.space.md))
}

/// A keyboard key ("⌘K", "Esc") as a keycap.
pub(crate) fn kbd(label: impl Into<SharedString>, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex_shrink_0()
        .px(px(t.space.xs))
        .rounded(px(t.radius.xs))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.border))
        .bg(rgb(p.surface))
        .text_size(px(t.text.overline))
        .font_family(mono_font())
        .text_color(rgb(p.roles.text_secondary))
        .child(label.into())
}

/// A status dot in the intent's colour.
pub(crate) fn status_dot(intent: Intent, p: &Palette) -> Div {
    let size = p.tokens.layout.status_dot;
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(p.tokens.radius.full))
        .bg(rgb(intent.color(p)))
}

/// Every primitive once, for tests and design review.
pub(crate) fn gallery(p: &Palette) -> AnyElement {
    use ButtonVariant as B;
    let variants = [B::Primary, B::Secondary, B::Ghost, B::Danger];
    let intents = [
        Intent::Neutral,
        Intent::Accent,
        Intent::Info,
        Intent::Success,
        Intent::Warning,
        Intent::Danger,
        Intent::Busy,
    ];
    let t = p.tokens;
    let row = || div().flex().flex_wrap().items_center().gap(px(t.space.md));
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.lg))
        .p(px(t.space.xl))
        .bg(rgb(p.chat))
        .child(section_label("Buttons", p))
        .child(
            row().children(variants.into_iter().enumerate().map(|(ix, v)| {
                button(("primitive-button", ix), format!("{v:?}"), None, v, true, p)
                    .debug_selector(move || format!("primitive-button-{v:?}"))
            })),
        )
        .child(
            row().children(variants.into_iter().enumerate().map(|(ix, v)| {
                icon_button(
                    ("primitive-icon", ix),
                    IconName::Plus,
                    "Add",
                    v,
                    ix % 2 == 0,
                    p,
                )
            })),
        )
        .child(row().children(intents.into_iter().map(|i| pill(format!("{i:?}"), i, p))))
        .child(row().children(intents.into_iter().map(|i| badge("3", i, p))))
        .child(row().children(intents.into_iter().map(|i| status_dot(i, p))))
        .child(
            row()
                .child(chip("primitive-chip-on", "Selected", true, true, p))
                .child(chip("primitive-chip-off", "Filter", false, true, p))
                .child(kbd("Esc", p)),
        )
        .child(
            div()
                .w(px(t.layout.menu_width))
                .child(meter(0.6, Intent::Accent, p)),
        )
        .child(
            segmented(p)
                .debug_selector(|| "primitive-segmented".into())
                .child(segment("primitive-segment-a", "Auto", true, p))
                .child(segment("primitive-segment-b", "Wide", false, p)),
        )
        .child(
            card(p)
                .debug_selector(|| "primitive-card".into())
                .child(card_header("Card", Some("A description.".into()), p))
                .child(list_row("primitive-row", true, true, p).child("Selected row"))
                .child(list_row("primitive-row-2", false, true, p).child("Row")),
        )
        .child(
            menu(p)
                .debug_selector(|| "primitive-menu".into())
                .child(menu_item(
                    "primitive-item",
                    "Open",
                    Some(IconName::ExternalLink),
                    false,
                    true,
                    p,
                ))
                .child(menu_separator(p))
                .child(menu_item(
                    "primitive-delete",
                    "Delete",
                    Some(IconName::Delete),
                    true,
                    true,
                    p,
                )),
        )
        .child(
            div().relative().h(px(t.layout.dialog_width)).child(
                scrim(p).child(
                    dialog(p)
                        .debug_selector(|| "primitive-dialog".into())
                        .child(dialog_header("Dialog", Some("Body copy.".into()), p))
                        .child(
                            dialog_footer(p)
                                .child(button(
                                    "primitive-cancel",
                                    "Cancel",
                                    None,
                                    B::Secondary,
                                    true,
                                    p,
                                ))
                                .child(button("primitive-ok", "OK", None, B::Primary, true, p)),
                        ),
                ),
            ),
        )
        .child(
            empty_state(
                IconName::Inbox,
                "Nothing here",
                Some("Empty states explain why.".into()),
                p,
            )
            .debug_selector(|| "primitive-empty".into()),
        )
        .into_any_element()
}
