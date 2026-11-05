//! R5.4 TextField / Composer: the one text-entry surface. The conversation
//! (`composer_v2.rs`), the welcome and New session (`launch_task_box_v2`)
//! all draw it with [`composer_field`] and lay their controls out with
//! [`composer_toolbar`]; nothing else draws a composer.
//!
//! The field is always bordered. r8-conversation (R8.4): while a decision
//! holds the conversation's slot the composer is hidden and its input is
//! the card's reply ([`card_reply`]); the card folds to [`decision_line`]
//! while reading back, and other sessions' news is [`updates_chip`].
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

/// r7-qa (systems P7): one keyboard hint, "↑ ↓ Move": faint keycaps, then
/// the plain words. Every hint footer (Fleet, Projects) is a row of these
/// ([`key_hints`]), never shortcuts spelled out in prose.
pub(crate) fn key_hint(keys: &[&'static str], label: &'static str, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .children(keys.iter().map(|k| prim::keycap_hint(*k, false, p)))
        .child(div().pl(px(t.space.xxs)).child(label))
}

/// A hint footer: [`key_hint`]s in one wrapping row, meta size, tertiary.
pub(crate) fn key_hints(hints: impl IntoIterator<Item = Div>, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(t.space.xl))
        .text_size(px(t.text.meta))
        .text_color(rgb(p.roles.text_tertiary))
        .children(hints)
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

// ----- r8-conversation (R8.4): one decision-and-reply area ----------------

/// R8.4: the reply inside a decision card. While a decision waits the
/// composer is hidden; "Reply instead" on the card opens this: the one
/// input (the composer's own, so a draft survives), a quiet line saying the
/// request stays open, and `actions` (Cancel, then the send that names
/// where the text goes, "Send to Claude Code"). It goes last in the card.
pub(crate) fn card_reply(
    input: impl IntoElement,
    note: impl Into<SharedString>,
    actions: impl IntoIterator<Item = AnyElement>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    super::edge(
        div()
            .w_full()
            .min_w_0()
            .pt(px(t.space.md))
            .flex()
            .flex_col()
            .gap(px(t.space.sm)),
        super::Side::Top,
        super::EdgeTone::Divider,
        p,
    )
    .child(
        div()
            .w_full()
            .min_w_0()
            .text_size(px(t.text.ui))
            .child(input),
    )
    .child(
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(t.space.md))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(note.into()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .children(actions),
            ),
    )
}

/// R8.4: a waiting decision folded to one line while the reader scrolls
/// up through the transcript ("● Needs you · Allow cargo test? · Open").
/// The card's attention frame, one control tall; `action` (Open: back to
/// the latest, the card unfolded) sits at its end, so no separate "Jump to
/// latest" floats over the text.
pub(crate) fn decision_line(
    summary: impl Into<SharedString>,
    action: impl IntoElement,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    let kind = prim::StatusKind::NeedsYou;
    prim::attention_frame(t.radius.panel, p)
        .w_full()
        .flex_shrink_0()
        .h(px(t.layout.control_height + 2. * t.space.xs))
        .pl(px(t.space.md + t.signals().attention_bar))
        .pr(px(t.space.xs))
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .bg(rgb(p.surface))
        .shadow(prim::elevation(p, t.elevation.floating))
        .text_size(px(t.text.body))
        .children(prim::status_glyph(kind, p))
        .child(
            div()
                .flex_shrink_0()
                .font_weight(FontWeight(t.weight.medium))
                .text_color(rgb(kind.text_color(p)))
                .child(kind.label()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(p.roles.text_tertiary))
                .child("·"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgb(p.text))
                .child(summary.into()),
        )
        .child(div().flex_shrink_0().child(action))
}

/// R8.4: other sessions' news as one chip in the conversation header
/// ("2 updates"), led by the worst state's glyph (a failure's red, else
/// Needs you's amber) or a quiet inbox. It opens [`updates_popover`].
pub(crate) fn updates_chip(
    id: impl Into<gpui::ElementId>,
    worst: Option<prim::StatusKind>,
    label: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let leading = match worst {
        Some(kind) => prim::status_glyph(kind, p).map(IntoElement::into_any_element),
        None => Some(
            Icon::new(IconName::Inbox)
                .size(px(t.icon.xs))
                .text_color(rgb(p.roles.text_tertiary))
                .into_any_element(),
        ),
    };
    prim::toolbar_chip(id, leading, label, true, true, p).flex_shrink_0()
}

/// One update in the header's popover: its state glyph, what happened in
/// words, and its own jump link (`link`, "Open").
pub(crate) fn update_row(
    id: impl Into<gpui::ElementId>,
    kind: Option<prim::StatusKind>,
    text: impl Into<SharedString>,
    link: Option<AnyElement>,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    prim::list_row(id, false, true, p)
        .child(
            div()
                .size(px(t.icon.sm))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .map(|d| match kind {
                    Some(kind) => d.children(prim::status_glyph(kind, p)),
                    None => d.child(
                        Icon::new(IconName::Inbox)
                            .size(px(t.icon.xs))
                            .text_color(rgb(p.roles.text_tertiary)),
                    ),
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgb(p.roles.text_secondary))
                .child(text.into()),
        )
        .children(link.map(|l| div().flex_shrink_0().child(l)))
}

/// The updates popover: one [`update_row`] per event, failures first.
pub(crate) fn updates_popover(rows: impl IntoIterator<Item = AnyElement>, p: &Palette) -> Div {
    let t = p.tokens;
    prim::menu(p)
        .w(px(t.system().overlay_width * 0.75))
        .children(rows)
}
