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

/// The meaning a pill, badge, meter or dot carries; its colour follows.
/// For a session's or job's state use [`StatusKind`] instead.
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

/// The three button tiers, plus the destructive variant (R3.2). Exactly
/// these; nothing styles its own button. One primary per region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonVariant {
    /// The one action a region exists for: filled with the primary colour.
    Primary,
    /// Every other peer action: a 1px `border_strong` edge, no fill (a
    /// neutral wash on hover), same height and radius as Primary.
    Secondary,
    /// Quiet actions in toolbars, rows and "Keep planning": text only, a
    /// neutral wash on hover.
    Tertiary,
    /// Stops or removes something: the Secondary shape with danger text.
    /// Put irreversible ones behind a confirm step or undo.
    Destructive,
}

/// The two control heights (`layout.control_height_sm` / `control_height`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonSize {
    /// 28: rows, cards, toolbars, dialogs.
    Small,
    /// 36: page-level and composer actions.
    Large,
}

impl ButtonSize {
    pub(crate) fn height(self, p: &Palette) -> f32 {
        match self {
            Self::Small => p.tokens.layout.control_height_sm,
            Self::Large => p.tokens.layout.control_height,
        }
    }

    fn padding(self, p: &Palette) -> f32 {
        let s = p.tokens.space;
        match self {
            Self::Small => s.md + s.xxs,
            Self::Large => s.lg + s.xxs,
        }
    }

    fn text(self, p: &Palette) -> f32 {
        match self {
            Self::Small => p.tokens.text.body,
            Self::Large => p.tokens.text.subheading,
        }
    }
}

/// The (rest, hover, pressed) colours and text of one button variant.
struct ButtonColors {
    bg: [Hsla; 3],
    fg: [u32; 2],
    /// A hairline ring drawn as a shadow (the border is the focus ring).
    ring: Option<u32>,
}

/// (rest, hover, pressed) backgrounds and (rest, hover) text for a variant.
fn button_colors(variant: ButtonVariant, enabled: bool, p: &Palette) -> ButtonColors {
    let w = p.tokens.wash;
    let solid = |c: u32| Hsla::from(rgb(c));
    let clear = gpui::transparent_black();
    if !enabled {
        let (bg, ring) = match variant {
            ButtonVariant::Tertiary => (clear, None),
            ButtonVariant::Primary => (solid(p.selected), None),
            ButtonVariant::Secondary | ButtonVariant::Destructive => (clear, Some(p.border)),
        };
        return ButtonColors {
            bg: [bg; 3],
            fg: [p.disabled; 2],
            ring,
        };
    }
    match variant {
        ButtonVariant::Primary => ButtonColors {
            bg: [
                solid(p.primary),
                solid(p.primary_hover),
                solid(p.primary_pressed),
            ],
            fg: [p.on_primary; 2],
            ring: None,
        },
        ButtonVariant::Secondary => ButtonColors {
            bg: [clear, tonal(p, w.hover), tonal(p, w.pressed)],
            fg: [p.text; 2],
            ring: Some(p.roles.border_strong),
        },
        ButtonVariant::Tertiary => ButtonColors {
            bg: [clear, solid(p.roles.hover), solid(p.roles.pressed)],
            fg: [p.roles.text_secondary, p.text],
            ring: None,
        },
        ButtonVariant::Destructive => ButtonColors {
            bg: [
                clear,
                wash(p.error, w.tone_rest),
                wash(p.error, w.tone_hover),
            ],
            fg: [p.error; 2],
            ring: Some(p.roles.border_strong),
        },
    }
}

/// The label colour of a button variant at rest: what its icons and keycap
/// hints follow.
pub(crate) fn button_label_color(variant: ButtonVariant, enabled: bool, p: &Palette) -> u32 {
    button_colors(variant, enabled, p).fg[0]
}

/// Which corners of a button are rounded: a whole button, or one half of a
/// split button.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Whole,
    Left,
    Right,
}

/// A 1px edge laid over the control's outer bounds (the reserved focus
/// border draws over it when focused). A child, not a shadow: GPUI does not
/// draw unblurred spread shadows.
fn ring(p: &Palette, color: u32, shape: Shape) -> Div {
    let t = p.tokens;
    let out = px(-t.stroke.focus);
    let r = px(t.radius.control);
    let edge = div()
        .absolute()
        .top(out)
        .bottom(out)
        .left(out)
        .right(out)
        .border(px(t.stroke.hairline))
        .border_color(rgb(color));
    match shape {
        Shape::Whole => edge.rounded(r),
        Shape::Left => edge.rounded_l(r),
        Shape::Right => edge.rounded_r(r),
    }
}

/// Apply a variant's fill, text and hover/press states to a control.
fn button_states(
    control: Stateful<Div>,
    variant: ButtonVariant,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    button_states_shaped(control, variant, enabled, Shape::Whole, p)
}

fn button_states_shaped(
    control: Stateful<Div>,
    variant: ButtonVariant,
    enabled: bool,
    shape: Shape,
    p: &Palette,
) -> Stateful<Div> {
    let ButtonColors {
        bg: [rest, hover, pressed],
        fg: [fg, fg_hover],
        ring: edge,
    } = button_colors(variant, enabled, p);
    control
        .relative()
        .bg(rest)
        .text_color(rgb(fg))
        .when_some(edge, |d, edge| d.child(ring(p, edge, shape)))
        .when(enabled, |d| {
            d.hover_text_style(move |s| s.bg(hover).text_color(rgb(fg_hover)))
                .active_text_style(move |s| s.bg(pressed).text_color(rgb(fg_hover)))
        })
}

/// A labelled button, with an optional leading icon. Add `.on_click`.
///
/// ```ignore
/// primitives::button("save", "Save", None, ButtonVariant::Primary, ButtonSize::Small, dirty, &p)
///     .debug_selector(|| "save".into())
///     .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
/// ```
pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    button_shaped(id, label, icon, variant, size, enabled, Shape::Whole, p)
}

#[allow(clippy::too_many_arguments)]
fn button_shaped(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    shape: Shape,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let r = px(t.radius.control);
    let control = interactive_control(div().id(id.into()), *p, enabled)
        .h(px(size.height(p)))
        .px(px(size.padding(p)))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(t.space.sm))
        .flex_shrink_0()
        .map(|d| match shape {
            Shape::Whole => d.rounded(r),
            Shape::Left => d.rounded_l(r),
            Shape::Right => d.rounded_r(r),
        })
        .text_size(px(size.text(p)))
        .font_weight(FontWeight(t.weight.medium))
        .when_some(icon, |d, icon| d.child(Icon::new(icon).size(px(t.icon.sm))))
        .child(label.into());
    button_states_shaped(control, variant, enabled, shape, p)
}

/// A square icon-only button (small height) with a tooltip naming it.
/// Add `.on_click`.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    variant: ButtonVariant,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    icon_button_with_shortcut(id, icon, tooltip, None, variant, enabled, p)
}

/// [`icon_button`] whose tooltip also shows its shortcut ("Ctrl K", "g n").
pub(crate) fn icon_button_with_shortcut(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    shortcut: Option<&'static str>,
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
        .tooltip(tooltip_view(tooltip, shortcut, p))
        .child(Icon::new(icon).size(px(t.icon.sm)));
    button_states(control, variant, enabled, p)
}

/// The two halves of a split button: the main action and the chevron that
/// opens its menu (Queue / Send now). Wire each half's `.on_click`, then
/// join them with [`split_button`].
pub(crate) fn split_button_parts(
    id: &'static str,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    p: &Palette,
) -> (Stateful<Div>, Stateful<Div>) {
    let t = p.tokens;
    let r = px(t.radius.control);
    let main = button_shaped(id, label, icon, variant, size, enabled, Shape::Left, p);
    let menu = interactive_control(div().id((id, 1usize)), *p, enabled)
        .h(px(size.height(p)))
        .w(px(size.height(p) - t.space.md))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded_r(r)
        .tooltip(tooltip_view("More options", None, p))
        .child(Icon::new(IconName::ChevronDown).size(px(t.icon.sm)));
    (
        main,
        button_states_shaped(menu, variant, enabled, Shape::Right, p),
    )
}

/// Join [`split_button_parts`] into one control, a hairline apart.
pub(crate) fn split_button(main: Stateful<Div>, menu: Stateful<Div>, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(p.tokens.stroke.hairline))
        .child(main)
        .child(menu)
}

/// A split button with its menu (R3.2): Deny with "Deny with note…", Send
/// with its other send modes. `menu` is `Some` while open: a [`menu`] of
/// [`menu_item`]s that hangs under the control's right edge, above
/// everything else. The caller owns the open flag, toggles it from the
/// chevron's `.on_click`, and closes it on choice or Esc.
///
/// ```ignore
/// let (deny, more) = split_button_parts("approval-deny", "Deny", None,
///     ButtonVariant::Destructive, ButtonSize::Small, true, &p);
/// split_button_with_menu(deny.on_click(…), more.on_click(…),
///     open.then(|| menu(&p).child(menu_item("deny-note", "Deny with note…", None, false, true, &p))),
///     &p)
/// ```
pub(crate) fn split_button_with_menu(
    main: Stateful<Div>,
    chevron: Stateful<Div>,
    menu: Option<Div>,
    p: &Palette,
) -> Div {
    let gap = p.tokens.space.xs;
    div()
        .relative()
        .flex_shrink_0()
        .child(split_button(main, chevron, p))
        .when_some(menu, |d, menu| {
            d.child(
                div()
                    .absolute()
                    .top(relative(1.))
                    .right_0()
                    .child(gpui::deferred(
                        gpui::anchored()
                            .anchor(gpui::Corner::TopRight)
                            .offset(gpui::point(px(0.), px(gap)))
                            .snap_to_window_with_margin(px(gap))
                            .child(menu),
                    )),
            )
        })
}

/// The "…" trigger for a row's or card's overflow menu: where Remove,
/// Discard and other rare or destructive actions live. Add `.on_click`.
pub(crate) fn overflow_trigger(id: impl Into<ElementId>, p: &Palette) -> Stateful<Div> {
    icon_button(
        id,
        IconName::Ellipsis,
        "More actions",
        ButtonVariant::Tertiary,
        true,
        p,
    )
}

/// A tooltip builder: the label, then the shortcut as keycaps ("Ctrl K",
/// or a sequence such as "g n"; spaces separate caps). Pass to `.tooltip`.
pub(crate) fn tooltip_view(
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
    p: &Palette,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let label: SharedString = label.into();
    let p = *p;
    move |window, cx| {
        let label = label.clone();
        Tooltip::element(move |_, _| {
            let t = p.tokens;
            div()
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .text_size(px(t.text.meta))
                .text_color(rgb(p.text))
                .child(label.clone())
                .when_some(shortcut, |d, keys| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.xxs))
                            .children(keys.split_whitespace().map(|key| kbd(key.to_owned(), &p))),
                    )
                })
        })
        .build(window, cx)
    }
}

/// Give a control that has no tooltip yet one, with an optional shortcut.
/// (GPUI allows one tooltip per element.)
pub(crate) fn with_tooltip(
    control: Stateful<Div>,
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
    p: &Palette,
) -> Stateful<Div> {
    control.tooltip(tooltip_view(label, shortcut, p))
}

/// One line of text that ends in an ellipsis when it does not fit and shows
/// the whole value in a tooltip; it never overlaps its neighbours. Give it a
/// text size and colour. For a character budget instead of the layout's,
/// cut with `wks_native::format::truncate_end` first.
pub(crate) fn truncated(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    div()
        .id(id.into())
        .min_w_0()
        .truncate()
        .tooltip(tooltip_view(text.clone(), None, p))
        .child(text)
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
        .font_weight(FontWeight(t.weight.medium))
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
        .font_weight(FontWeight(t.weight.medium))
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
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.base))
        .child(label.into())
}

/// What a count stands for, which decides whether it may be coloured
/// (R3.1): amber only for what is blocked on this user now, red only for a
/// named failure; every other count (scratch files, proposals, decisions on
/// other sessions, changes) is neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CountTone {
    Neutral,
    NeedsYou,
    Failed,
}

/// The one count badge ("3", "12", "99+"): tabular figures on a fully
/// rounded chip. Neutral is a quiet wash with secondary text; `NeedsYou`
/// and `Failed` fill with their status colour.
pub(crate) fn count_badge(label: impl Into<SharedString>, tone: CountTone, p: &Palette) -> Div {
    let t = p.tokens;
    let (bg, fg) = match tone {
        CountTone::Neutral => (tonal(p, t.wash.pressed), p.roles.text_secondary),
        CountTone::NeedsYou => (Hsla::from(rgb(p.roles.status.attention)), p.base),
        CountTone::Failed => (Hsla::from(rgb(p.roles.status.fault)), p.base),
    };
    tabular(
        div()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .min_w(px(t.text.caption + t.space.sm))
            .px(px(t.space.xs))
            .rounded(px(t.radius.full))
            .bg(bg)
            .text_size(px(t.text.overline))
            .font_weight(FontWeight(t.weight.semibold))
            .text_color(rgb(fg))
            .child(label.into()),
    )
}

/// A count for a badge: "99+" past 99.
pub(crate) fn count_text(n: usize) -> String {
    if n > 99 { "99+".into() } else { n.to_string() }
}

/// The colour a share of a limit takes (usage, context; `format::level`):
/// none below 80% (draw it neutral), attention from 80%, fault at 100%.
pub(crate) fn level_color(level: wks_native::format::Level, p: &Palette) -> Option<u32> {
    use wks_native::format::Level;
    match level {
        Level::Normal => None,
        Level::Warn => Some(p.roles.status.attention),
        Level::Fault => Some(p.roles.status.fault),
    }
}

/// [`level_color`] as a meter [`Intent`] (Neutral below 80%).
pub(crate) fn level_intent(level: wks_native::format::Level) -> Intent {
    use wks_native::format::Level;
    match level {
        Level::Normal => Intent::Neutral,
        Level::Warn => Intent::Warning,
        Level::Fault => Intent::Danger,
    }
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
                .font_weight(FontWeight(t.weight.semibold))
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

/// The one section header naming a group of rows or cards: the contract's
/// [`section_header`] (sentence case 12/600; no uppercase overline), flush
/// with the content edge so every page shares one left edge (r2-pages).
pub(crate) fn section_label(label: impl Into<SharedString>, p: &Palette) -> Div {
    let t = p.tokens;
    section_header(label, None, p).pb(px(t.space.xs))
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
        .border_color(rgb(p.roles.card_border))
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
                .text_size(px(t.text.subheading))
                .font_weight(FontWeight(t.weight.semibold))
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
        .h(px(t.layout.control_height_sm - 2. * t.space.xxs))
        .px(px(t.space.lg))
        .rounded(px(t.radius.control - t.space.xxs))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.medium))
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
                .font_weight(FontWeight(t.weight.semibold))
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

/// The status language (R3.1): every state a session, subagent, job,
/// machine, task or card can show, with one word, one colour AND one 8px
/// shape everywhere (sidebar, rail, capsule, palette, history, machines,
/// children, fleet):
///
/// | Kind | Word | Role | Glyph |
/// |---|---|---|---|
/// | Working | Working | working (teal) | ring with a rotating arc (static under reduced motion) |
/// | NeedsYou | Needs you | attention (amber) | filled dot |
/// | NotResponding | Not responding | fault (red) | hollow clock |
/// | Failed | Failed | fault (red) | ✕ |
/// | Idle | Idle | — | **no glyph** (show the age instead) |
/// | Done | Done | done (muted green) | check |
/// | Paused | Paused | idle (grey) | pause bars (machines, jobs) |
/// | Offline | Offline | idle (grey) | dash (machines, the hub) |
///
/// Retired as state words: "running" (for agents), "stalled", "Lost
/// heartbeat", "Queued", "Unknown" as amber. Blue is never a status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StatusKind {
    /// An agent or run in progress.
    Working,
    /// Blocked on this user now: an approval, a question, a plan.
    NeedsYou,
    /// Should be working but has gone quiet (no heartbeat): a fault, never
    /// also "Working".
    NotResponding,
    /// Broke; name what, and offer the way out.
    Failed,
    /// Ready and waiting for work, not started yet, or ended.
    Idle,
    /// Finished successfully.
    Done,
    /// Switched off or held (a machine's gate, a job).
    Paused,
    /// Not reachable (a machine, the hub).
    Offline,
}

impl StatusKind {
    pub(crate) const ALL: [Self; 8] = [
        Self::Working,
        Self::NeedsYou,
        Self::NotResponding,
        Self::Failed,
        Self::Idle,
        Self::Done,
        Self::Paused,
        Self::Offline,
    ];

    /// The canonical word for this state: the only one any surface uses.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
            Self::NotResponding => wks_native::format::NOT_RESPONDING,
            Self::Failed => "Failed",
            Self::Idle => "Idle",
            Self::Done => "Done",
            Self::Paused => "Paused",
            Self::Offline => "Offline",
        }
    }

    /// The glyph's colour: one of the palette's five status roles.
    pub(crate) fn color(self, p: &Palette) -> u32 {
        let r = p.roles.status;
        match self {
            Self::NeedsYou => r.attention,
            Self::Working => r.working,
            Self::Done => r.done,
            Self::Failed | Self::NotResponding => r.fault,
            Self::Idle | Self::Paused | Self::Offline => r.idle,
        }
    }

    /// The label's colour: only states that ask for something carry their
    /// hue (amber blocked on you, red broke); the rest are neutral text.
    pub(crate) fn text_color(self, p: &Palette) -> u32 {
        let r = p.roles.status;
        match self {
            Self::NeedsYou => r.attention,
            Self::Failed | Self::NotResponding => r.fault,
            Self::Working | Self::Done => p.roles.text_secondary,
            Self::Idle | Self::Paused | Self::Offline => p.roles.text_tertiary,
        }
    }

    /// Whether the user has to act (attention or fault): these belong in
    /// Needs you.
    pub(crate) fn needs_attention(self) -> bool {
        matches!(self, Self::NeedsYou | Self::Failed | Self::NotResponding)
    }

    /// Whether this state draws a glyph at all (Idle does not).
    pub(crate) fn has_glyph(self) -> bool {
        self != Self::Idle
    }
}

/// The 8px status glyph for `kind` (`signals().glyph`), or `None` for Idle,
/// which has no glyph. Drawn as vector strokes so every shape stays crisp at
/// any interface size. Working's arc turns once per `signals().spin_ms` and
/// holds still under reduced motion.
pub(crate) fn status_glyph(kind: StatusKind, p: &Palette) -> Option<Div> {
    status_glyph_sized(kind, p.tokens.signals().glyph, p)
}

/// [`status_glyph`] at another size (the avatar corner badge keeps 8).
pub(crate) fn status_glyph_sized(kind: StatusKind, size: f32, p: &Palette) -> Option<Div> {
    if !kind.has_glyph() {
        return None;
    }
    let sig = p.tokens.signals();
    let color = Hsla::from(rgb(kind.color(p)));
    let track = color.opacity(sig.track_alpha);
    // Where the Working arc starts, as a share of a turn (rests at the top).
    let phase = if kind == StatusKind::Working {
        super::motion::looping(std::time::Duration::from_millis(sig.spin_ms as u64), 0.)
    } else {
        0.
    };
    let glyph = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for (path, fill) in glyph_paths(kind, bounds, sig, phase) {
                window.paint_path(path, if fill == Paint::Track { track } else { color });
            }
        },
    )
    .size_full();
    Some(
        div()
            .size(px(size))
            .flex_shrink_0()
            .debug_selector(move || format!("status-glyph-{kind:?}"))
            .child(glyph),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paint {
    Ink,
    Track,
}

/// The vector paths of one glyph inside `bounds` (a square).
fn glyph_paths(
    kind: StatusKind,
    bounds: gpui::Bounds<gpui::Pixels>,
    sig: wks_native::tokens::SignalTokens,
    phase: f32,
) -> Vec<(gpui::Path<gpui::Pixels>, Paint)> {
    use gpui::{PathBuilder, point};
    let s = f32::from(bounds.size.width.min(bounds.size.height));
    let k = s / sig.glyph;
    let stroke = sig.glyph_stroke * k;
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let at = |x: f32, y: f32| point(px_raw(ox + x * s), px_raw(oy + y * s));
    let (c, r) = (0.5, (s - stroke) / 2. / s);
    // A point on the ring at `deg` clockwise from 12 o'clock.
    let on = |deg: f32, radius: f32| {
        let a = deg.to_radians();
        at(c + radius * a.sin(), c - radius * a.cos())
    };
    let radii = point(px_raw(r * s), px_raw(r * s));
    let ring = |b: &mut PathBuilder| {
        b.move_to(on(0., r));
        b.arc_to(radii, px_raw(0.), false, true, on(180., r));
        b.arc_to(radii, px_raw(0.), false, true, on(360., r));
    };
    let line = |points: &[(f32, f32)]| {
        let mut b = PathBuilder::stroke(px_raw(stroke));
        for (ix, (x, y)) in points.iter().enumerate() {
            if ix == 0 {
                b.move_to(at(*x, *y));
            } else {
                b.line_to(at(*x, *y));
            }
        }
        b
    };
    let mut out: Vec<(PathBuilder, Paint)> = Vec::new();
    match kind {
        StatusKind::Idle => {}
        StatusKind::NeedsYou => {
            let mut b = PathBuilder::fill();
            let full = point(px_raw(s / 2.), px_raw(s / 2.));
            b.move_to(at(c, 0.));
            b.arc_to(full, px_raw(0.), false, true, at(c, 1.));
            b.arc_to(full, px_raw(0.), false, true, at(c, 0.));
            b.close();
            out.push((b, Paint::Ink));
        }
        StatusKind::Working => {
            let mut track = PathBuilder::stroke(px_raw(stroke));
            ring(&mut track);
            out.push((track, Paint::Track));
            let start = phase * 360.;
            let mut arc = PathBuilder::stroke(px_raw(stroke));
            arc.move_to(on(start, r));
            arc.arc_to(
                radii,
                px_raw(0.),
                sig.arc_degrees > 180.,
                true,
                on(start + sig.arc_degrees, r),
            );
            out.push((arc, Paint::Ink));
        }
        StatusKind::NotResponding => {
            let mut b = PathBuilder::stroke(px_raw(stroke));
            ring(&mut b);
            out.push((b, Paint::Ink));
            // Clock hands: twelve and three o'clock.
            out.push((
                line(&[(c, c - r * 0.55), (c, c), (c + r * 0.45, c)]),
                Paint::Ink,
            ));
        }
        StatusKind::Failed => {
            let (a, z) = (stroke / s / 2. + 0.08, 0.92 - stroke / s / 2.);
            out.push((line(&[(a, a), (z, z)]), Paint::Ink));
            out.push((line(&[(z, a), (a, z)]), Paint::Ink));
        }
        StatusKind::Done => {
            out.push((line(&[(0.1, 0.55), (0.4, 0.82), (0.92, 0.22)]), Paint::Ink));
        }
        StatusKind::Paused => {
            out.push((line(&[(0.3, 0.12), (0.3, 0.88)]), Paint::Ink));
            out.push((line(&[(0.7, 0.12), (0.7, 0.88)]), Paint::Ink));
        }
        StatusKind::Offline => {
            out.push((line(&[(0.12, 0.5), (0.88, 0.5)]), Paint::Ink));
        }
    }
    out.into_iter()
        .filter_map(|(b, paint)| b.build().ok().map(|path| (path, paint)))
        .collect()
}

/// Window pixels as they are (inside a canvas, bounds are already zoomed).
fn px_raw(v: f32) -> gpui::Pixels {
    gpui::px(v)
}

/// The status glyph in its fixed `layout.status_box` square, so labels
/// after it share one left edge. Idle keeps the square, empty.
pub(crate) fn status_indicator(kind: StatusKind, p: &Palette) -> Div {
    div()
        .size(px(p.tokens.layout.status_box))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .children(status_glyph(kind, p))
}

/// A status as glyph + word. `label` overrides the word for a more specific
/// one ("Succeeded 3h ago", "Cancelled") while keeping the kind's glyph and
/// colour; prefer the canonical [`StatusKind::label`].
#[derive(Clone, Debug)]
pub(crate) struct StatusBadge {
    pub(crate) kind: StatusKind,
    pub(crate) label: Option<SharedString>,
}

impl StatusBadge {
    pub(crate) fn new(kind: StatusKind) -> Self {
        Self { kind, label: None }
    }

    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Div {
        let t = p.tokens;
        div()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .flex_shrink_0()
            .min_w_0()
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(self.kind.text_color(p)))
            .child(status_indicator(self.kind, p))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(self.label.unwrap_or_else(|| self.kind.label().into())),
            )
    }
}

/// [`StatusBadge`] with the vocabulary label.
pub(crate) fn status(kind: StatusKind, p: &Palette) -> Div {
    StatusBadge::new(kind).render(p)
}

/// A page's header: an optional back link, the title (22/600), a one-line
/// description and right-aligned actions. The back row's height is kept
/// even without a back link, so every page title lands at the same y; put
/// it in [`page_column`] for the shared top offset and max width.
pub(crate) fn page_header(
    back: Option<AnyElement>,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    actions: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(t.space.sm))
        .child(
            div()
                .h(px(t.layout.control_height_sm))
                .flex()
                .items_center()
                // Ghost padding hangs outside the column so the back label
                // lines up with the title.
                .ml(px(-(t.space.md + t.space.xxs)))
                .children(back),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(t.space.xl))
                .child(
                    div()
                        .debug_selector(|| "page-title".into())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.title))
                        .font_weight(FontWeight(t.weight.title))
                        .text_color(rgb(p.text))
                        .child(title.into()),
                )
                .when(!actions.is_empty(), |d| {
                    d.child(
                        div()
                            .debug_selector(|| "page-actions".into())
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .children(actions),
                    )
                }),
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

/// A page header's back link ("Back to chat"). Add `.on_click`.
pub(crate) fn back_link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    button(
        id,
        label,
        Some(IconName::ArrowLeft),
        ButtonVariant::Tertiary,
        ButtonSize::Small,
        true,
        p,
    )
}

/// The shared full-page content column: `layout.page_width` wide at most,
/// centred, below the fixed `layout.page_top` offset.
pub(crate) fn page_column(p: &Palette) -> Div {
    let t = p.tokens.layout;
    div()
        .w_full()
        .max_w(px(t.page_width))
        .mx_auto()
        .pt(px(t.page_top))
        .flex()
        .flex_col()
        .gap(px(p.tokens.space.xxl))
}

/// One card anatomy for every list item and transcript card: a leading
/// icon (or avatar), the title with its status beside it, one meta line,
/// and actions on the right.
pub(crate) struct CardHead {
    leading: Option<Leading>,
    title: SharedString,
    status: Option<StatusBadge>,
    meta: Option<SharedString>,
    actions: Vec<AnyElement>,
}

enum Leading {
    Icon(IconName),
    Avatar(SharedString),
}

impl CardHead {
    pub(crate) fn new(title: impl Into<SharedString>) -> Self {
        Self {
            leading: None,
            title: title.into(),
            status: None,
            meta: None,
            actions: Vec::new(),
        }
    }

    pub(crate) fn icon(mut self, icon: IconName) -> Self {
        self.leading = Some(Leading::Icon(icon));
        self
    }

    /// A monogram tile for `name` instead of an icon.
    pub(crate) fn avatar(mut self, name: impl Into<SharedString>) -> Self {
        self.leading = Some(Leading::Avatar(name.into()));
        self
    }

    pub(crate) fn status(mut self, status: StatusBadge) -> Self {
        self.status = Some(status);
        self
    }

    pub(crate) fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub(crate) fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Div {
        let t = p.tokens;
        let leading = self.leading.map(|leading| match leading {
            Leading::Icon(icon) => div()
                .size(px(t.layout.avatar_lg))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(t.radius.control))
                .bg(tonal(p, t.wash.rest))
                .text_color(rgb(p.roles.text_secondary))
                .child(Icon::new(icon).size(px(t.icon.md))),
            Leading::Avatar(name) => avatar_tile_sized(&name, t.layout.avatar_lg, p),
        });
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.lg))
            .children(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(t.text.subheading))
                                    .font_weight(FontWeight(t.weight.semibold))
                                    .text_color(rgb(p.text))
                                    .child(self.title),
                            )
                            .children(self.status.map(|s| s.render(p))),
                    )
                    .when_some(self.meta, |d, meta| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(t.text.meta))
                                .text_color(rgb(p.roles.text_tertiary))
                                .child(meta),
                        )
                    }),
            )
            .when(!self.actions.is_empty(), |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .children(self.actions),
                )
            })
    }
}

/// A monogram avatar tile at `layout.avatar`: two letters on one of the
/// theme's eight curated fills (`Palette::avatar_fill`, picked by a stable
/// hash of the name; equal lightness, AA text). No emoji, no ring.
pub(crate) fn avatar_tile(name: &str, p: &Palette) -> Div {
    avatar_tile_sized(name, p.tokens.layout.avatar, p)
}

/// An avatar with its status as a corner badge (R3.6): the 8px status glyph
/// on a disc of `under` (the surface the avatar sits on) at the bottom
/// right. Idle (no glyph) and `None` draw the bare tile. The one way to show
/// a session's or machine's state on an avatar: never a coloured ring.
pub(crate) fn avatar_with_status(
    name: &str,
    size: f32,
    status: Option<StatusKind>,
    under: u32,
    p: &Palette,
) -> Div {
    let sig = p.tokens.signals();
    let disc = sig.glyph + 2. * sig.badge_ring;
    let out = -sig.badge_ring - p.tokens.stroke.hairline;
    div()
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .child(avatar_tile_sized(name, size, p))
        .children(status.and_then(|kind| status_glyph(kind, p)).map(|glyph| {
            div()
                .debug_selector(|| "avatar-status".into())
                .absolute()
                .right(px(out))
                .bottom(px(out))
                .size(px(disc))
                .rounded(px(p.tokens.radius.full))
                .bg(rgb(under))
                .flex()
                .items_center()
                .justify_center()
                .child(glyph)
        }))
}

/// [`avatar_tile`] at another size (`layout.avatar_lg` on cards).
pub(crate) fn avatar_tile_sized(name: &str, size: f32, p: &Palette) -> Div {
    use wks_native::format::monogram;
    let t = p.tokens;
    // r3-foundation: one of the theme's eight curated, equal-lightness fills.
    let fill = p.avatar_fill(name);
    div()
        .size(px(size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(t.radius.control))
        .bg(rgb(fill))
        .text_size(px(if size >= t.layout.avatar_lg {
            t.text.meta
        } else {
            t.text.overline
        }))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.avatar_text()))
        .child(monogram(name))
}

/// Which edge of floating chrome a [`fade_mask`] sits against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FadeEdge {
    /// Under a top bar (title capsule): solid at the top, clear below.
    Top,
    /// Above a bottom control (Jump to latest, the composer).
    Bottom,
}

/// A `layout.fade`-tall gradient from `color` (the surface beneath, usually
/// `p.chat`) to clear, so content scrolling under floating chrome fades out
/// instead of showing through or clipping. Absolute and full width: put it
/// in the floating chrome's (relative) container, behind its controls.
pub(crate) fn fade_mask(edge: FadeEdge, color: u32, p: &Palette) -> Div {
    let solid = Hsla::from(rgb(color));
    let (angle, mask) = match edge {
        FadeEdge::Top => (180., div().top_0()),
        FadeEdge::Bottom => (0., div().bottom_0()),
    };
    mask.absolute()
        .left_0()
        .w_full()
        .h(px(p.tokens.layout.fade))
        .bg(gpui::linear_gradient(
            angle,
            gpui::linear_color_stop(solid, 0.),
            gpui::linear_color_stop(solid.opacity(0.), 1.),
        ))
}

/// Every primitive once, for tests and design review.
pub(crate) fn gallery(p: &Palette) -> AnyElement {
    use ButtonVariant as B;
    let variants = [B::Primary, B::Secondary, B::Tertiary, B::Destructive];
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
    let buttons = |size: ButtonSize, base: usize| {
        row().children(variants.into_iter().enumerate().map(move |(ix, v)| {
            button(
                ("primitive-button", base + ix),
                format!("{v:?}"),
                (ix == 0).then_some(IconName::Plus),
                v,
                size,
                true,
                p,
            )
            .debug_selector(move || format!("primitive-button-{v:?}-{size:?}"))
        }))
    };
    let (split_main, split_menu) = split_button_parts(
        "primitive-split",
        "Queue",
        None,
        B::Primary,
        ButtonSize::Small,
        true,
        p,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.lg))
        .p(px(t.space.xl))
        .bg(rgb(p.chat))
        .child(
            page_header(
                Some(back_link("primitive-back", "Back to chat", p).into_any_element()),
                "Page title",
                Some("One line that says what this page is for.".into()),
                vec![
                    button(
                        "primitive-header-action",
                        "New job",
                        Some(IconName::Plus),
                        B::Primary,
                        ButtonSize::Large,
                        true,
                        p,
                    )
                    .into_any_element(),
                ],
                p,
            )
            .debug_selector(|| "primitive-page-header".into()),
        )
        .child(section_label("Buttons", p))
        .child(buttons(ButtonSize::Small, 0))
        .child(buttons(ButtonSize::Large, variants.len()))
        .child(
            row()
                .children(variants.into_iter().enumerate().map(|(ix, v)| {
                    icon_button(
                        ("primitive-icon", ix),
                        IconName::Plus,
                        "Add",
                        v,
                        ix % 2 == 0,
                        p,
                    )
                }))
                .child(
                    split_button(split_main, split_menu, p)
                        .debug_selector(|| "primitive-split".into()),
                )
                .child(
                    overflow_trigger("primitive-overflow", p)
                        .debug_selector(|| "primitive-overflow".into()),
                )
                .child(icon_button_with_shortcut(
                    "primitive-shortcut",
                    IconName::Search,
                    "Search sessions",
                    Some("Ctrl K"),
                    B::Tertiary,
                    true,
                    p,
                ))
                .child(with_tooltip(
                    button(
                        "primitive-jump",
                        "Next needs you",
                        None,
                        B::Tertiary,
                        ButtonSize::Small,
                        true,
                        p,
                    ),
                    "Jump to the next session that needs you",
                    Some("g n"),
                    p,
                )),
        )
        .child(section_label("Status", p))
        .child(row().children(StatusKind::ALL.into_iter().map(|kind| {
            status(kind, p).debug_selector(move || format!("primitive-status-{kind:?}"))
        })))
        .child(row().children(intents.into_iter().map(|i| pill(format!("{i:?}"), i, p))))
        .child(row().children(intents.into_iter().map(|i| badge("3", i, p))))
        .child(row().children(intents.into_iter().map(|i| status_dot(i, p))))
        // r3-foundation: the 8px glyphs alone, the count badges, the
        // attention frame, keycap hints and avatars with a status badge.
        .child(
            row().debug_selector(|| "primitive-glyphs".into()).children(
                StatusKind::ALL
                    .into_iter()
                    .filter_map(|kind| status_glyph(kind, p)),
            ),
        )
        .child(
            row()
                .debug_selector(|| "primitive-counts".into())
                .child(count_badge("4", CountTone::Neutral, p))
                .child(count_badge("2", CountTone::NeedsYou, p))
                .child(count_badge("1", CountTone::Failed, p))
                .child(count_badge(count_text(120), CountTone::Neutral, p))
                .child(keycap_hint("1", false, p)),
        )
        .child(
            attention_frame(t.radius.control, p)
                .debug_selector(|| "primitive-attention".into())
                .bg(rgb(p.surface))
                .pl(px(t.space.lg + t.signals().attention_bar))
                .pr(px(t.space.lg))
                .py(px(t.space.md))
                .child("Release checklist for 0.9 needs a branch name"),
        )
        .child(
            row()
                .debug_selector(|| "primitive-avatars".into())
                .children(
                    ["workspacer", "hub-rs", "inbox", "docs", "web", "native"]
                        .into_iter()
                        .map(|name| avatar_tile(name, p)),
                )
                .children(
                    [
                        ("Fleet", StatusKind::Working),
                        ("Release", StatusKind::NeedsYou),
                        ("Trace", StatusKind::NotResponding),
                        ("Notes", StatusKind::Idle),
                    ]
                    .into_iter()
                    .map(|(name, kind)| {
                        avatar_with_status(name, t.layout.avatar_lg, Some(kind), p.chat, p)
                    }),
                ),
        )
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
            div().w(px(t.layout.status_column)).child(
                truncated(
                    "primitive-truncated",
                    "A value far too long for its column",
                    p,
                )
                .debug_selector(|| "primitive-truncated".into())
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_secondary)),
            ),
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
                .child(
                    CardHead::new("Nightly tests")
                        .icon(IconName::Calendar)
                        .status(StatusBadge::new(StatusKind::Working))
                        .meta("Weekdays at 07:00 · next in 3h")
                        .action(button(
                            "primitive-card-run",
                            "Run now",
                            None,
                            B::Secondary,
                            ButtonSize::Small,
                            true,
                            p,
                        ))
                        .action(overflow_trigger("primitive-card-more", p))
                        .render(p)
                        .debug_selector(|| "primitive-card-head".into()),
                )
                .child(
                    CardHead::new("workspacer")
                        .avatar("workspacer")
                        .status(StatusBadge::new(StatusKind::NeedsYou))
                        .meta("3 sessions · 1 needs you")
                        .render(p),
                )
                .child(card_header("Card", Some("A description.".into()), p))
                .child(list_row("primitive-row", true, true, p).child("Selected row"))
                .child(list_row("primitive-row-2", false, true, p).child("Row")),
        )
        .child(
            div()
                .relative()
                .h(px(t.layout.fade * 2.))
                .rounded(px(t.radius.panel))
                .overflow_hidden()
                .bg(rgb(p.surface))
                .child(
                    fade_mask(FadeEdge::Top, p.chat, p).debug_selector(|| "primitive-fade".into()),
                ),
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
                                    ButtonSize::Small,
                                    true,
                                    p,
                                ))
                                .child(button(
                                    "primitive-ok",
                                    "OK",
                                    None,
                                    B::Primary,
                                    ButtonSize::Small,
                                    true,
                                    p,
                                )),
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
        // r2-systems: the component contract.
        .child(section_header("Contract", Some("6".into()), p))
        .child(
            row().debug_selector(|| "primitive-roles".into()).children(
                [B::Primary, B::Secondary, B::Destructive, B::Tertiary]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, v)| {
                        keyed_button(
                            ("primitive-role", ix),
                            (ix + 1).to_string(),
                            format!("{v:?}"),
                            v,
                            ButtonSize::Small,
                            true,
                            p,
                        )
                    }),
            ),
        )
        .child(
            table(
                vec!["Area".into(), "Status".into(), "Lines".into()],
                vec![vec![
                    "Tables".into_any_element(),
                    status(StatusKind::Done, p).into_any_element(),
                    "1,204".into_any_element(),
                ]],
                p,
            )
            .debug_selector(|| "primitive-table".into()),
        )
        .child(
            div()
                .debug_selector(|| "primitive-events".into())
                .child(event_group_header(
                    "primitive-events-head",
                    wks_native::format::worked_summary(134_000, 18),
                    true,
                    p,
                ))
                .child(
                    EventRow::new(IconName::SquareTerminal, "Ran")
                        .argument("cargo test")
                        .summary("212 passed")
                        .duration("42s")
                        .status(StatusKind::Done)
                        .render(p),
                )
                .child(
                    EventRow::new(IconName::File, "Read")
                        .argument("src/ui/primitives.rs")
                        .status(StatusKind::Working)
                        .render(p),
                ),
        )
        .child(
            page_grid(3, p)
                .debug_selector(|| "primitive-grid".into())
                .children((0..3).map(|ix| card(p).child(format!("Cell {ix}")))),
        )
        .child(
            waiting_card(false, p)
                .debug_selector(|| "primitive-waiting".into())
                .child(waiting_head(
                    headline(
                        vec![
                            Headline::Text("Run".into()),
                            Headline::Code("cargo test".into()),
                            Headline::Text("in native?".into()),
                        ],
                        p,
                    ),
                    Some("Run the tests before committing · This machine".into()),
                    Some("1/2".into()),
                    Vec::new(),
                    p,
                ))
                .child(inset(p).child("Inset well")),
        )
        .into_any_element()
}

// area-composer

/// The one keycap hint (R3.2): the key that also triggers a control, faint
/// and trailing ("Allow  1"), never tinted with a status or danger colour.
/// `on_fill` is true on a Primary fill, where the hint is the label colour
/// at `signals().hint_alpha`; elsewhere it is tertiary text. A hairline
/// keycap edge in the same colour, no fill.
pub(crate) fn keycap_hint(key: impl Into<SharedString>, on_fill: bool, p: &Palette) -> Div {
    let t = p.tokens;
    let sig = t.signals();
    let fg = if on_fill {
        wash(p.on_primary, sig.hint_alpha)
    } else {
        Hsla::from(rgb(p.roles.text_tertiary))
    };
    div()
        .flex_shrink_0()
        .min_w(px(t.text.overline + t.space.xs))
        .px(px(t.space.xs))
        .rounded(px(t.radius.xs))
        .flex()
        .items_center()
        .justify_center()
        .border(px(t.stroke.hairline))
        .border_color(fg.opacity(fg.a * t.wash.attention_edge))
        .text_size(px(t.text.overline))
        .line_height(relative(1.25))
        .font_family(mono_font())
        .font_weight(FontWeight(t.weight.regular))
        .text_color(fg)
        .child(key.into())
}

/// A [`button`] with the key that also triggers it as a trailing
/// [`keycap_hint`] ("Allow 1"), for decision strips where number keys pick
/// an action.
pub(crate) fn keyed_button(
    id: impl Into<ElementId>,
    key: impl Into<SharedString>,
    label: impl Into<SharedString>,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let on_fill = enabled && variant == ButtonVariant::Primary;
    let control = interactive_control(div().id(id.into()), *p, enabled)
        .h(px(size.height(p)))
        .pl(px(size.padding(p)))
        .pr(px(size.padding(p) - t.space.xs))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(t.space.md))
        .flex_shrink_0()
        .rounded(px(t.radius.control))
        .text_size(px(size.text(p)))
        .font_weight(FontWeight(t.weight.medium))
        .child(label.into())
        .child(keycap_hint(key, on_fill, p));
    button_states(control, variant, enabled, p)
}

/// A quiet chip inside a composer or toolbar: an optional leading element
/// (icon or glyph), a label and, when it opens something, a chevron. Ghost
/// at rest, a neutral wash on hover. Add `.on_click` for interactive chips.
pub(crate) fn toolbar_chip(
    id: impl Into<ElementId>,
    leading: Option<AnyElement>,
    label: impl Into<SharedString>,
    opens: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let (hover, pressed) = (p.roles.hover, p.roles.pressed);
    interactive_control(div().id(id.into()), *p, enabled && opens)
        .h(px(t.layout.icon_button))
        .min_w_0()
        .px(px(t.space.sm))
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .rounded(px(t.radius.control))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.medium))
        .text_color(rgb(if enabled {
            p.roles.text_secondary
        } else {
            p.disabled
        }))
        .when(enabled && opens, |d| {
            d.hover_text_style(move |s| s.bg(rgb(hover)).text_color(rgb(p.text)))
                .active_text_style(move |s| s.bg(rgb(pressed)))
        })
        .children(leading)
        .child(div().min_w_0().truncate().child(label.into()))
        .when(opens, |d| {
            d.child(
                Icon::new(IconName::ChevronDown)
                    .size(px(t.icon.xs))
                    .text_color(rgb(p.roles.text_tertiary)),
            )
        })
}

// r2-systems: the component contract (`docs/design-v2-direction.md` R2.2).
// Every v2 surface draws its buttons, cards, section headers, tables, event
// rows, avatars, figures, page grids and "waiting on you" containers with
// these; nothing styles its own.

/// `el` with tabular (fixed-width) numerals: times, counts and stats line up
/// and never jiggle as they tick.
pub(crate) fn tabular<E: Styled>(mut el: E) -> E {
    el.text_style()
        .get_or_insert_with(Default::default)
        .font_features = Some(gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
    el
}

/// The one inset: a recessed well inside a [`card`] (plan bodies, approval
/// details, code-like facts). Never a second card level.
pub(crate) fn inset(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.xs))
        .px(px(t.space.lg))
        .py(px(t.space.md))
        .rounded(px(t.radius.control))
        .bg(rgb(p.chat))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.roles.card_border))
}

/// The one section header: sentence case, 12/600, secondary, with an
/// optional count in tabular numerals. Lists, pages and settings all use it;
/// nothing is uppercase.
pub(crate) fn section_header(
    label: impl Into<SharedString>,
    count: Option<SharedString>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .min_w_0()
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.roles.text_secondary))
        .child(div().min_w_0().truncate().child(label.into()))
        .when_some(count, |d, count| {
            d.child(tabular(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(count),
            ))
        })
}

/// The one table style, shared by [`table`] and Markdown tables
/// (`ui::configure_theme_v2` maps gpui-component's table colours from it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TableStyle {
    /// The header row's fill and text.
    pub(crate) head_bg: u32,
    pub(crate) head_fg: u32,
    /// Body rows: one fill, hairline dividers, no zebra.
    pub(crate) row_bg: u32,
    pub(crate) divider: u32,
    pub(crate) hover: u32,
    pub(crate) cell_x: f32,
    pub(crate) cell_y: f32,
}

pub(crate) fn table_style(p: &Palette) -> TableStyle {
    let t = p.tokens;
    TableStyle {
        head_bg: p.base,
        head_fg: p.roles.text_secondary,
        row_bg: p.surface,
        divider: p.border,
        hover: p.roles.hover,
        cell_x: t.space.lg,
        cell_y: t.space.sm,
    }
}

/// A table: header cells, then rows of cells, with hairline row dividers
/// and no outer border (it sits inside a card or on the page). Text columns
/// are left-aligned; pass numbers already formatted (they get tabular
/// numerals).
pub(crate) fn table(headers: Vec<SharedString>, rows: Vec<Vec<AnyElement>>, p: &Palette) -> Div {
    let t = p.tokens;
    let s = table_style(p);
    let columns = headers.len().max(1) as u16;
    let cell = move || {
        div()
            .min_w_0()
            .px(px(s.cell_x))
            .py(px(s.cell_y))
            .flex()
            .items_center()
    };
    let head = div()
        .w_full()
        .grid()
        .grid_cols(columns)
        .bg(rgb(s.head_bg))
        .rounded_t(px(t.radius.control))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(s.head_fg))
        .children(headers.into_iter().map(move |h| cell().child(h)));
    div()
        .w_full()
        .flex()
        .flex_col()
        .text_size(px(t.text.body))
        .text_color(rgb(p.text))
        .child(head)
        .children(rows.into_iter().map(move |row| {
            tabular(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(columns)
                    .border_t(px(t.stroke.hairline))
                    .border_color(rgb(s.divider))
                    .children(row.into_iter().map(move |c| cell().child(c))),
            )
        }))
}

// r3-pages
/// A page-table column's width: a fixed measure from `tokens.pages()`, or
/// a share of what is left (the name column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Col {
    /// A measure that gives way (truncating) when the window is narrow.
    Fixed(f32),
    /// A measure that never gives way: the actions column.
    Keep(f32),
    Flex,
    /// Whatever its content needs (the actions column), right-aligned.
    Auto,
}

/// A dense page table (R3.6: Jobs, Machines, History): the card surface
/// with its hairline edge, clipped, holding a [`page_table_head`] and
/// [`page_table_row`]s.
pub(crate) fn page_table(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(t.radius.panel))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.roles.card_border))
        .bg(rgb(p.surface))
        .overflow_hidden()
}

/// One cell of a page table, sized by its column. Cells share the row's
/// gap, so a head and its rows line up when built from the same columns.
pub(crate) fn page_cell(col: Col) -> Div {
    let cell = div().min_w_0().flex().items_center();
    match col {
        Col::Fixed(w) => cell.w(px(w)).flex_shrink(),
        Col::Keep(w) => cell.w(px(w)).flex_shrink_0(),
        Col::Flex => cell.flex_1(),
        Col::Auto => cell.flex_shrink_0().justify_end(),
    }
}

/// A page table's header row: 12/600 secondary labels on `base`, the same
/// columns as its rows.
pub(crate) fn page_table_head(columns: &[(&'static str, Col)], p: &Palette) -> Div {
    let s = table_style(p);
    let t = p.tokens;
    page_table_row(p)
        .h(px(t.layout.row_height))
        .bg(rgb(s.head_bg))
        .border_t(px(0.))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(s.head_fg))
        .children(
            columns
                .iter()
                .map(|(label, col)| page_cell(*col).child(*label)),
        )
}

/// A page table's body row: `pages().row` tall at least, cells at the
/// table's padding, a hairline above. Add `.id(…)` for hover or clicks.
pub(crate) fn page_table_row(p: &Palette) -> Div {
    let s = table_style(p);
    let t = p.tokens;
    div()
        .w_full()
        .min_h(px(t.pages().row))
        .px(px(s.cell_x))
        .flex()
        .items_center()
        .gap(px(t.space.lg))
        .border_t(px(t.stroke.hairline))
        .border_color(rgb(s.divider))
        .text_size(px(t.text.body))
        .text_color(rgb(p.text))
}

/// One agent or tool event as a single row: icon, verb, primary argument,
/// result summary, and the status/duration right-aligned
/// (`docs/design-v2-direction.md` R2.2). Consecutive rows sit under one
/// [`event_group_header`].
pub(crate) struct EventRow {
    icon: IconName,
    verb: SharedString,
    argument: Option<SharedString>,
    summary: Option<SharedString>,
    status: Option<StatusKind>,
    duration: Option<SharedString>,
}

impl EventRow {
    pub(crate) fn new(icon: IconName, verb: impl Into<SharedString>) -> Self {
        Self {
            icon,
            verb: verb.into(),
            argument: None,
            summary: None,
            status: None,
            duration: None,
        }
    }

    /// The thing it acted on: a path, a command, a pattern (monospace).
    pub(crate) fn argument(mut self, argument: impl Into<SharedString>) -> Self {
        self.argument = Some(argument.into());
        self
    }

    /// What came of it: "3 lines", "1 match", "+12 −4".
    pub(crate) fn summary(mut self, summary: impl Into<SharedString>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub(crate) fn status(mut self, status: StatusKind) -> Self {
        self.status = Some(status);
        self
    }

    /// Already formatted (`format::duration`).
    pub(crate) fn duration(mut self, duration: impl Into<SharedString>) -> Self {
        self.duration = Some(duration.into());
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Div {
        let t = p.tokens;
        let failed = self.status.is_some_and(StatusKind::needs_attention);
        div()
            .w_full()
            .min_w_0()
            .h(px(t.layout.control_height_sm))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .text_size(px(t.text.body))
            .child(
                Icon::new(self.icon)
                    .size(px(t.icon.sm))
                    .flex_shrink_0()
                    .text_color(rgb(p.roles.text_tertiary)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(if failed { p.roles.status.fault } else { p.text }))
                    .child(self.verb),
            )
            .when_some(self.argument, |d, argument| {
                d.child(
                    div()
                        .min_w_0()
                        .flex_shrink()
                        .truncate()
                        .font_family(mono_font())
                        .text_color(rgb(p.roles.text_secondary))
                        .child(argument),
                )
            })
            .when_some(self.summary, |d, summary| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(summary),
                )
            })
            .child(div().flex_1())
            .when_some(self.duration, |d, duration| {
                d.child(tabular(
                    div()
                        .flex_shrink_0()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(duration),
                ))
            })
            .when_some(self.status, |d, kind| d.child(status_indicator(kind, p)))
    }
}

/// The header of a run of [`EventRow`]s: a chevron and "Worked 2m 14s · 18
/// tools" (`format::worked_summary`). Add `.on_click` to expand/collapse.
pub(crate) fn event_group_header(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    expanded: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let (hover, fg) = (p.roles.hover, p.text);
    tabular(
        interactive_control(div().id(id.into()), *p, true)
            .h(px(t.layout.control_height_sm))
            .px(px(t.space.xs))
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .rounded(px(t.radius.control))
            .text_size(px(t.text.body))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(p.roles.text_secondary))
            .hover_text_style(move |s| s.bg(rgb(hover)).text_color(rgb(fg)))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(t.icon.sm)),
            )
            .child(label.into()),
    )
}

/// A shared page grid: `columns` equal tracks at the page gutter, rows
/// stretched to their tallest card so actions line up. Put it in
/// [`page_column`] so every page shares one width and left edge.
pub(crate) fn page_grid(columns: u16, p: &Palette) -> Div {
    div()
        .w_full()
        .grid()
        .grid_cols(columns.max(1))
        .gap(px(p.tokens.space.lg))
}

/// The one "waiting on you" container, for approvals, questions, plans and
/// usage limits: the [`attention_frame`] at the panel radius on the card
/// surface. Fill it with a [`waiting_head`], the body, then a
/// [`waiting_actions`] row.
pub(crate) fn waiting_card(compact: bool, p: &Palette) -> Div {
    let t = p.tokens;
    let pad = if compact { t.space.md } else { t.space.lg };
    attention_frame(t.radius.panel, p)
        .w_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(if compact { t.space.sm } else { t.space.md }))
        .pl(px(pad + t.signals().attention_bar))
        .pr(px(pad))
        .py(px(pad))
        .bg(rgb(p.surface))
        .shadow(elevation(p, t.elevation.floating))
}

/// The one attention treatment (R3.1): a container with a neutral 1px
/// `border_strong` edge, rounded to `radius`, carrying an [`attention_bar`]
/// on its leading edge. Use it for whatever is blocked on this user: the
/// waiting card, a Needs-you sidebar row, a fleet tile with a pending ask.
/// Add the background, padding (leave `signals().attention_bar` extra on the
/// left) and children.
pub(crate) fn attention_frame(radius: f32, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .relative()
        .rounded(px(radius))
        .border(px(t.stroke.hairline))
        .border_color(rgb(p.roles.border_strong))
        .child(attention_bar(radius, p))
}

/// The attention bar alone: `signals().attention_bar` wide in the attention
/// colour, on the leading edge of a container rounded to `radius`, clipped
/// by that radius (it follows the corner's curve rather than floating inset).
/// Absolute: put it in a `.relative()` container with a hairline border, or
/// use [`attention_frame`]. Borderless containers: wrap it in
/// `div().absolute().inset_0()` instead.
pub(crate) fn attention_bar(radius: f32, p: &Palette) -> Div {
    let t = p.tokens;
    let bar = t.signals().attention_bar;
    let color = Hsla::from(rgb(p.roles.status.attention));
    let edge = t.stroke.hairline;
    div()
        .absolute()
        .left(px(-edge))
        .top(px(-edge))
        .bottom(px(-edge))
        .w(px(bar))
        .debug_selector(|| "attention-bar".into())
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    if let Some(path) = clipped_bar(bounds, bar, radius) {
                        window.paint_path(path, color);
                    }
                },
            )
            .size_full(),
        )
}

/// The leading `bar`-wide strip of a rounded rectangle whose left edge is
/// `bounds`' left edge and whose height is `bounds`' height: straight on
/// the right, following the corner radius on the left.
fn clipped_bar(
    bounds: gpui::Bounds<gpui::Pixels>,
    bar: f32,
    radius: f32,
) -> Option<gpui::Path<gpui::Pixels>> {
    use gpui::{PathBuilder, point};
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    if w <= 0. || h <= 0. {
        return None;
    }
    // Logical sizes to window pixels: the strip is `bar` logical px wide.
    let scale = w / bar;
    let r = (radius * scale).min(h / 2.);
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let at = |x: f32, y: f32| point(px_raw(x0 + x), px_raw(y0 + y));
    let mut b = PathBuilder::fill();
    if r <= w {
        b.move_to(at(0., 0.));
        b.line_to(at(w, 0.));
        b.line_to(at(w, h));
        b.line_to(at(0., h));
    } else {
        // Where the corner's circle crosses the strip's right edge.
        let dip = r - (r * r - (r - w) * (r - w)).sqrt();
        let radii = point(px_raw(r), px_raw(r));
        b.move_to(at(0., r));
        b.arc_to(radii, px_raw(0.), false, true, at(w, dip));
        b.line_to(at(w, h - dip));
        b.arc_to(radii, px_raw(0.), false, true, at(0., h - r));
    }
    b.close();
    b.build().ok()
}

/// One run of a decision headline: plain words, or a code span (a command,
/// a path) drawn in mono on the inline-code fill.
pub(crate) enum Headline {
    Text(SharedString),
    Code(SharedString),
}

/// A question-style headline ("Run `cargo test` in native?") at `text.lead`
/// 600: words never shrink, a code span truncates (full value on hover).
pub(crate) fn headline(parts: Vec<Headline>, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .text_size(px(t.text.lead))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.text))
        .children(parts.into_iter().enumerate().map(|(ix, part)| {
            match part {
                Headline::Text(text) => div().flex_shrink_0().child(text).into_any_element(),
                Headline::Code(code) => truncated(("headline-code", ix), code, p)
                    .debug_selector(|| "headline-code".into())
                    .flex_shrink()
                    .px(px(t.space.xs))
                    .rounded(px(t.radius.xs))
                    .bg(rgb(p.code_inline))
                    .font_family(mono_font())
                    .font_weight(FontWeight(t.weight.medium))
                    .text_size(px(t.text.subheading))
                    .into_any_element(),
            }
        }))
}

/// A "waiting on you" container's head: the question headline, the
/// always-visible where/why line under it, then a pager ("2/3") and any
/// trailing controls (Details) on the right.
pub(crate) fn waiting_head(
    headline: impl IntoElement,
    context: Option<SharedString>,
    position: Option<SharedString>,
    trailing: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(t.space.md))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(t.space.xxs))
                .child(headline)
                .when_some(context, |d, context| {
                    d.child(
                        div()
                            .debug_selector(|| "waiting-context".into())
                            .min_w_0()
                            .text_size(px(t.text.meta))
                            .line_height(relative(t.text.line_height))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(context),
                    )
                }),
        )
        .when_some(position, |d, position| {
            d.child(tabular(
                div()
                    .debug_selector(|| "waiting-position".into())
                    .flex_shrink_0()
                    .h(px(t.layout.control_height_sm))
                    .flex()
                    .items_center()
                    .text_size(px(t.text.meta))
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(position),
            ))
        })
        .children(trailing)
}

/// A "waiting on you" container's action row: the decision's buttons, left
/// to right in role order (primary, secondary, destructive, tertiary).
pub(crate) fn waiting_actions(p: &Palette) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(p.tokens.space.sm))
}
