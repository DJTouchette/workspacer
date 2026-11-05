//! R5.4 TextField / Composer: the one text-entry surface. The conversation
//! (`composer_v2.rs`), the welcome and New session (`launch_task_box_v2`)
//! all draw it with [`composer_field`] and lay their controls out with
//! [`composer_toolbar`]; nothing else draws a composer.
//!
//! The field is always there and always bordered. While a decision waits
//! it does not collapse or fade: it stays a full-contrast field whose
//! placeholder says the request stays pending, and Send drops to Secondary
//! (the decision card owns the Primary, [`super::composer_send_state`]).
use super::super::primitives::{self as prim, ButtonSize, ButtonVariant, ControlState};
use super::*;

/// The composer's states (`docs/design-system.md`, TextField / Composer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerState {
    /// Resting: the `border_strong` hairline.
    Rest,
    /// Focused: the edge takes the `focus` colour.
    Focus,
    /// A decision waits on the user: the same field at full contrast (never
    /// collapsed, never faded); the caller sets the "Reply instead" text
    /// and Send drops to Secondary.
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
