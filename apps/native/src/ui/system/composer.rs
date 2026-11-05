//! R5.4 TextField / Composer: the one text-entry surface. The conversation
//! (`composer_v2.rs`), the welcome and New session (`launch_task_box_v2`)
//! all draw it with [`composer_field`] and lay their controls out with
//! [`composer_toolbar`]; nothing else draws a composer.
//!
//! The field is always there and always bordered. While a decision waits
//! it never fades: unfocused and empty it folds to one bordered line,
//! "Reply instead…" ([`composer_reply_line`], R6.3), and on focus it is the
//! full field whose placeholder says the request stays open, with Send
//! at Secondary (the decision card owns the Primary,
//! [`super::composer_send_state`]).
use super::super::primitives::{self as prim, ButtonSize, ButtonVariant, ControlState};
use super::*;

/// r7-pages (R7.6): the composer's one width, on the welcome, New session
/// and in the conversation: the form measure (880), which is also the
/// Comfortable reading width. A wider reading column (Wide, Full) keeps the
/// composer at this width, centred under it.
pub(crate) fn composer_width(p: &Palette) -> f32 {
    p.tokens.pages().form
}

/// The composer's states (`docs/design-system.md`, TextField / Composer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerState {
    /// Resting: the `border_strong` hairline.
    Rest,
    /// Focused: the edge takes the `focus` colour.
    Focus,
    /// A decision waits on the user: the field at full contrast (never
    /// faded; one line until it has focus); the caller sets the "Reply
    /// instead" text and Send drops to Secondary.
    Yielded,
    /// Nothing can be sent (offline, busy): the body at `wash.yielded`.
    Disabled,
}

impl ComposerState {
    pub(crate) const ALL: [ComposerState; 4] = [
        ComposerState::Rest,
        ComposerState::Focus,
        ComposerState::Yielded,
        ComposerState::Disabled,
    ];

    /// The field's edge colour.
    pub(crate) fn edge(self, p: &Palette) -> u32 {
        match self {
            ComposerState::Focus => p.roles.focus,
            _ => p.roles.border_strong,
        }
    }
}

/// The composer's field: `surface` fill, a hairline edge for `state`, the
/// composer radius and the floating shadow. Put the input (or, in a
/// specimen, its text) and then a [`composer_toolbar`] inside with
/// [`composer_body`].
pub(crate) fn composer_field(state: ComposerState, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .relative()
        .w_full()
        .flex()
        .flex_col()
        .bg(rgb(p.surface))
        .border(px(t.stroke.hairline))
        .border_color(rgb(state.edge(p)))
        .rounded(px(t.radius.composer))
        .shadow(prim::elevation(p, t.elevation.floating))
}

/// The padded column inside the field: the text, then the toolbar.
/// `compact` tightens the padding (short windows); a disabled composer's
/// body takes `wash.yielded`.
pub(crate) fn composer_body(state: ComposerState, compact: bool, p: &Palette) -> Div {
    let t = p.tokens;
    let pad = if compact { t.space.md } else { t.space.lg };
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.sm))
        .px(px(pad))
        .pt(px(pad))
        .pb(px(t.space.md))
        .when(state == ComposerState::Disabled, |d| {
            d.opacity(t.wash.yielded)
        })
}

/// The text slot: the input sits a hair in from the toolbar's chips, and
/// is at least `min_lines` rows tall (the welcome and New session ask for
/// a task: three lines; the conversation grows from one).
pub(crate) fn composer_text(min_lines: f32, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .px(px(t.space.xs))
        .text_size(px(t.text.ui))
        .min_h(px(t.layout.row_height * min_lines.max(1.)))
        .flex()
        .flex_col()
}

/// The toolbar under the text: `leading` (attach, then context chips) on
/// the left, shrinking and clipping first; `trailing` (status, then the
/// Send slot) on the right, never shrinking.
pub(crate) fn composer_toolbar(
    leading: impl IntoIterator<Item = AnyElement>,
    trailing: impl IntoIterator<Item = AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(t.space.md))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(t.space.xxs))
                .children(leading),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .children(trailing),
        )
}

/// R6.3: while a decision waits and the composer is not in use, it is one
/// bordered line ("Reply instead…") that expands into the full composer on
/// focus. `hint` (where the digit keys go, [`decision_keys_hint`]) sits at
/// its trailing end. The caller makes it focus the input on click.
pub(crate) fn composer_reply_line(
    label: impl Into<SharedString>,
    hint: Option<AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    composer_field(ComposerState::Yielded, p).child(
        div()
            .h(px(t.layout.control_height))
            .px(px(t.space.lg + t.space.xs))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(t.space.md))
            .text_size(px(t.text.ui))
            .text_color(rgb(p.roles.text_tertiary))
            .child(div().min_w_0().truncate().child(label.into()))
            .children(hint),
    )
}

/// r7-pages (R7.7): the composer folded under a question card. The card
/// holds the one input, so the composer is not a second field: it is a
/// quiet text line ("Reply — the request stays open") that opens the full
/// composer on click, with the digit hint at its end.
pub(crate) fn composer_reply_link(
    label: impl Into<SharedString>,
    hint: Option<AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    div()
        .w_full()
        .h(px(t.layout.control_height))
        .px(px(t.space.lg + t.space.xs))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(t.space.md))
        .rounded(px(t.radius.control))
        .text_size(px(t.text.meta))
        .text_color(rgb(p.roles.text_tertiary))
        .hover(|s| {
            s.bg(rgb(p.roles.hover))
                .text_color(rgb(p.roles.text_secondary))
        })
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .child(
                    Icon::new(IconName::Undo2)
                        .size(px(t.icon.sm))
                        .flex_shrink_0(),
                )
                .child(div().min_w_0().truncate().child(label.into())),
        )
        .children(hint)
}

/// R6.3: where the digit keys go. R7.1: the hint lists exactly the keys
/// the card shows ("1 2 4 answer the request"), each as a keycap; nothing
/// while the composer has focus (digits type there).
pub(crate) fn decision_keys_hint(caps: &[String], p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(t.space.xxs))
        .text_size(px(t.text.meta))
        .text_color(rgb(p.roles.text_tertiary))
        .children(caps.iter().map(|k| prim::keycap_hint(k.clone(), false, p)))
        .child(div().ml(px(t.space.xs)).child(if caps.len() > 1 {
            "answer the request"
        } else {
            "answers the request"
        }))
}

/// R6.4: the access chip beside the model chip ("Sonnet · Ask first"). Full
/// access reads in the caution tone so it is never mistaken for the
/// default. It opens the access choices, which apply at once.
pub(crate) fn access_chip(
    id: impl Into<gpui::ElementId>,
    word: &'static str,
    full: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    prim::toolbar_chip(
        id,
        full.then(|| {
            Icon::new(IconName::TriangleAlert)
                .size(px(t.icon.xs))
                .into_any_element()
        }),
        word,
        true,
        enabled,
        p,
    )
    // R6.2: Full access in the warning hue (`roles.caution`), distinct
    // from Needs-you amber and fault red.
    .when(full, |d| d.text_color(rgb(access_tone(true, p))))
}

/// The access word's colour: caution for Full access, else secondary.
pub(crate) fn access_tone(full: bool, p: &Palette) -> u32 {
    if full {
        p.roles.caution
    } else {
        p.roles.text_secondary
    }
}

/// R6.4: one access choice, in the chip's menu and on New session: a check
/// on the mode in force, the word, and what it lets the agent do. A choice
/// applies on click (no Apply).
pub(crate) fn access_option(
    id: impl Into<gpui::ElementId>,
    word: &'static str,
    consequence: &'static str,
    active: bool,
    full: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    prim::list_row(id, active, enabled, p)
        .py(px(t.space.xs))
        .items_start()
        .child(
            div()
                .size(px(t.icon.sm))
                .mt(px(t.space.xxs))
                .flex_shrink_0()
                .when(active, |d| {
                    d.child(
                        Icon::new(IconName::Check)
                            .size(px(t.icon.sm))
                            .text_color(rgb(p.accent)),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(if full {
                            access_tone(true, p)
                        } else if active {
                            p.text
                        } else {
                            p.roles.text_secondary
                        }))
                        .child(word),
                )
                .child(
                    div()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(consequence),
                ),
        )
}

/// A static composer for the gallery: `text` (a placeholder when
/// `placeholder`), a folder and a model chip, and Send in the variant and
/// enabled state [`super::composer_send_state`] gives for `deciding` /
/// `can_send`.
pub(crate) fn composer_specimen(
    id: &'static str,
    state: ComposerState,
    text: &'static str,
    placeholder: bool,
    deciding: bool,
    can_send: bool,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    let (variant, enabled) = super::composer_send_state(can_send, deciding);
    let send = prim::button_specimen(
        "Send",
        variant,
        ButtonSize::Small,
        if enabled {
            ControlState::Rest
        } else {
            ControlState::Disabled
        },
        p,
    );
    let chip = |suffix: &'static str, icon: IconName, label: &'static str, opens: bool| {
        prim::toolbar_chip(
            SharedString::from(format!("{id}-{suffix}")),
            Some(
                Icon::new(icon)
                    .size(px(t.icon.xs))
                    .text_color(rgb(p.roles.text_tertiary))
                    .into_any_element(),
            ),
            label,
            opens,
            state != ComposerState::Disabled,
            p,
        )
        .into_any_element()
    };
    composer_field(state, p).child(
        composer_body(state, false, p)
            .child(
                composer_text(1., p).child(
                    div()
                        .text_color(rgb(if placeholder {
                            p.roles.text_tertiary
                        } else {
                            p.text
                        }))
                        .child(text),
                ),
            )
            .child(composer_toolbar(
                [
                    prim::icon_button(
                        SharedString::from(format!("{id}-attach")),
                        IconName::Plus,
                        "Attach a file",
                        ButtonVariant::Tertiary,
                        state != ComposerState::Disabled,
                        p,
                    )
                    .into_any_element(),
                    chip("folder", IconName::Folder, "native", false),
                    chip("model", IconName::Bot, "Sonnet", true),
                ],
                [send.into_any_element()],
                p,
            )),
    )
}
