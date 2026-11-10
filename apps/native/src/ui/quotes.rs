//! Quoting the chat: text selected in a message becomes a quote the reply
//! carries. Selecting offers a floating Quote action; taking it puts the
//! passage above the composer's draft as a card, and the sent message shows
//! it the same way.
use super::*;
use gpui::{AnyElement, MouseButton, Pixels, Point};

const QUOTE_ICON: &str = "lucide/text-quote.svg";
/// Lines a quote shows above the draft, and in a sent message.
const DRAFT_LINES: usize = 3;
const SENT_LINES: usize = 8;

/// The selection a Quote action would take.
pub(super) struct QuotePick {
    /// The conversation it was selected in ([`Workspace::chat_owner`]).
    pub owner: String,
    pub text: String,
    /// Where the drag ended, while the floating action is still beside it.
    /// Scrolling moves the text away, so it clears this and leaves the key.
    pub at: Option<Point<Pixels>>,
}

/// A quoted passage: an accent bar beside muted, italic copy.
fn quote_card(text: &str, lines: usize, p: Palette) -> Div {
    div()
        .min_w_0()
        .flex()
        .gap_2()
        .child(
            div()
                .w(px(2.))
                .flex_shrink_0()
                .rounded_full()
                .bg(gpui::Hsla::from(rgb(p.accent)).opacity(0.55)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .line_clamp(lines)
                .italic()
                .text_size(px(scale::META))
                .text_color(rgb(p.muted))
                .child(text.to_owned()),
        )
}

/// The quotes a sent message opened with, as it shows them.
pub(super) fn sent_quotes(quotes: &[String], p: Palette) -> Div {
    div()
        .debug_selector(|| "message-quotes".into())
        .flex()
        .flex_col()
        .gap_2()
        .children(quotes.iter().map(|quote| quote_card(quote, SENT_LINES, p)))
}

impl Workspace {
    /// Whether the chat on screen has a composer for a quote to land in.
    fn can_quote(&self) -> bool {
        self.screen == Screen::Conversation
            && !self.new_session
            && self.view.selected.is_some()
            && self.view.child.is_none()
    }

    /// The selection reporter for a chat text view: a finished drag offers
    /// the Quote action, and a press that clears the selection withdraws it.
    pub(super) fn quote_selection_events(
        &self,
        cx: &mut Context<Self>,
    ) -> impl Fn(Option<&str>, Point<Pixels>, &mut Window, &mut App) + Send + Sync + 'static {
        let workspace = cx.entity().downgrade();
        move |selection, at, _, cx| {
            let _ = workspace.update(cx, |this, cx| {
                let pick = selection
                    .and_then(wks_native::quotes::clean)
                    .filter(|_| this.can_quote())
                    .map(|text| QuotePick {
                        owner: this.chat_owner(),
                        text,
                        at: Some(at),
                    });
                if pick.is_some() || this.extras.quote_pick.is_some() {
                    this.extras.quote_pick = pick;
                    cx.notify();
                }
            });
        }
    }

    /// The pick, when it still belongs to the conversation on screen.
    fn quote_pick(&self) -> Option<&QuotePick> {
        self.extras
            .quote_pick
            .as_ref()
            .filter(|pick| self.can_quote() && pick.owner == self.chat_owner())
    }

    /// Put `text` above the selected session's draft and hand the keyboard
    /// to the composer, ready for the reply.
    pub(super) fn add_quote(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.quote_pick = None;
        let Some(id) = self.view.selected.clone().filter(|_| self.can_quote()) else {
            return;
        };
        let quotes = self.extras.quotes.entry(id).or_default();
        if !quotes.contains(&text) {
            quotes.push(text);
        }
        self.composer.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// `r` on a chat selection: quote it.
    pub(super) fn quote_selected(
        &mut self,
        _: &QuoteSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.quote_pick().map(|pick| pick.text.clone()) {
            Some(text) => self.add_quote(text, window, cx),
            None => cx.propagate(),
        }
    }

    /// The Quote action floating where the selection drag ended. It has no
    /// backdrop: the chat stays live under it, and it acts on the press,
    /// since that same press is what clears the selection it quotes.
    pub(super) fn render_quote_pick(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pick = self.quote_pick()?;
        let at = pick.at?;
        let p = self.palette();
        let text = pick.text.clone();
        let button = div()
            .id("quote-selection")
            .debug_selector(|| "quote-selection".into())
            .h(px(control::HEIGHT_SM))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_full()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .shadow(chrome::floating_shadow(p))
            .text_size(px(scale::META))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(p.text))
            .cursor_pointer()
            .occlude()
            .hover(|d| d.bg(rgb(p.selected)))
            .child(
                Icon::empty()
                    .path(QUOTE_ICON)
                    .size(px(icon_size::MD))
                    .text_color(rgb(p.accent)),
            )
            .child("Quote")
            .child(keycap("R", p))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    this.add_quote(text.clone(), window, cx);
                }),
            );
        let workspace = cx.entity().downgrade();
        let layer = div()
            .child(
                gpui::anchored()
                    .anchor(gpui::Corner::BottomLeft)
                    .position(gpui::point(at.x + px(8.), at.y - px(12.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(button),
            )
            .child(
                // The text scrolls out from under the action, so a wheel
                // turn puts it away (the selection and its key remain).
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        window.on_mouse_event(
                            move |_: &gpui::ScrollWheelEvent, phase, _, cx| {
                                if phase.capture() {
                                    let _ = workspace.update(cx, |this, cx| {
                                        if let Some(pick) = &mut this.extras.quote_pick
                                            && pick.at.take().is_some()
                                        {
                                            cx.notify();
                                        }
                                    });
                                }
                            },
                        );
                    },
                )
                .absolute()
                .size(px(0.)),
            );
        Some(gpui::deferred(layer).with_priority(1).into_any_element())
    }

    /// The selected session's draft quotes, for the composer.
    pub(super) fn draft_quotes(&self) -> &[String] {
        self.view
            .selected
            .as_ref()
            .and_then(|id| self.extras.quotes.get(id))
            .map_or(&[], Vec::as_slice)
    }

    /// The draft's quotes above its text: each a card that can be removed.
    pub(super) fn render_draft_quotes(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let busy = self.view.busy;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.draft_quotes().iter().enumerate().map(|(ix, quote)| {
                div()
                    .id(("draft-quote", ix))
                    .debug_selector(move || format!("draft-quote-{ix}"))
                    .pl_2()
                    .pr_1()
                    .py_1()
                    .rounded(px(p.control_radius))
                    .bg(chrome::tonal(p, alpha::FAINT))
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(quote_card(quote, DRAFT_LINES, p).flex_1().py_1())
                    .child(
                        self.icon_button(format!("remove-quote-{ix}"), "Remove quote", IconName::Close, !busy)
                            .debug_selector(move || format!("remove-quote-{ix}"))
                            .size(px(24.))
                            .when(!busy, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(id) = &this.view.selected
                                        && let Some(quotes) = this.extras.quotes.get_mut(id)
                                        && ix < quotes.len()
                                    {
                                        quotes.remove(ix);
                                    }
                                    cx.notify();
                                }))
                            }),
                    )
            }))
    }
}
