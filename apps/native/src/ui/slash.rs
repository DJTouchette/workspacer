//! The composer's `/` menu: the selected session's commands and skills
//! (`slashCommands` on its snapshot), filtered by the token being typed.
//! Matching lives in `wks_native::model::slash_matches`; this is the list,
//! its highlighted row and the keys it takes while it is open.
//!
//! The menu owns Up, Down, Enter, Tab and Esc only while it shows rows (its
//! `SlashMenu` key context exists only then), so the busy composer's Tab
//! queue and Enter send are untouched the moment it closes. An IME
//! composition keeps those keys, and Enter on a draft that already is the
//! highlighted command sends it rather than inserting it again.
use super::*;
use wks_native::model::{SlashCommand, slash_matches};

impl Workspace {
    /// The rows the menu shows for the current draft; empty = shut.
    pub(super) fn slash_menu(&self, cx: &App) -> Vec<SlashCommand> {
        if self.screen != Screen::Conversation || self.new_session {
            return Vec::new();
        }
        let Some(session) = self.selected_session() else {
            return Vec::new();
        };
        let draft = self.composer.read(cx).value();
        if self.extras.slash_dismissed.as_ref() == Some(&(session.id.clone(), draft.to_string())) {
            return Vec::new();
        }
        slash_matches(&session.slash_commands, draft.as_ref())
            .into_iter()
            .cloned()
            .collect()
    }

    /// The highlighted row: it belongs to the draft it was moved on, so
    /// typing another character starts again from the first match.
    pub(super) fn slash_highlight(&self, rows: usize, cx: &App) -> usize {
        match &self.extras.slash_cursor {
            Some(owner) if Some(owner) == self.slash_owner(cx).as_ref() => {
                owner.2.min(rows.saturating_sub(1))
            }
            _ => 0,
        }
    }

    /// The (session, draft) a highlight or a dismissal belongs to, with the
    /// highlight's row (0 when only the owner is wanted).
    fn slash_owner(&self, cx: &App) -> Option<(String, String, usize)> {
        let session = self.view.selected.clone()?;
        let draft = self.composer.read(cx).value().to_string();
        let ix = self.extras.slash_cursor.as_ref().map_or(0, |c| c.2);
        Some((session, draft, ix))
    }

    /// An IME composition keeps its keys, as it keeps Enter and Tab.
    fn slash_composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.composer
            .update(cx, |input, cx| {
                gpui::EntityInputHandler::marked_text_range(input, window, cx)
            })
            .is_some_and(|range| !range.is_empty())
    }

    fn slash_move(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.slash_menu(cx).len();
        if rows == 0 || self.slash_composing(window, cx) {
            cx.propagate();
            return;
        }
        let current = self.slash_highlight(rows, cx) as isize;
        let next = (current + step).rem_euclid(rows as isize) as usize;
        self.extras.slash_cursor = self
            .slash_owner(cx)
            .map(|(session, draft, _)| (session, draft, next));
        cx.notify();
    }

    pub(super) fn slash_up(&mut self, _: &SlashUp, window: &mut Window, cx: &mut Context<Self>) {
        self.slash_move(-1, window, cx);
    }

    pub(super) fn slash_down(
        &mut self,
        _: &SlashDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.slash_move(1, window, cx);
    }

    /// The highlighted row, unless the menu is shut or an IME composes.
    fn slash_pick(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<SlashCommand> {
        let rows = self.slash_menu(cx);
        if rows.is_empty() || self.slash_composing(window, cx) {
            return None;
        }
        let ix = self.slash_highlight(rows.len(), cx);
        rows.into_iter().nth(ix)
    }

    /// Tab on the open menu: put the highlighted command in the composer,
    /// followed by a space for its arguments.
    pub(super) fn slash_accept(
        &mut self,
        _: &SlashAccept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.slash_pick(window, cx) {
            Some(command) => self.slash_insert(&command, window, cx),
            None => cx.propagate(),
        }
    }

    /// Enter on the open menu inserts like Tab — except when the draft
    /// already IS the highlighted command (`/compact`), when Enter keeps its
    /// ordinary job (send, with Enter sending) instead of adding a space.
    pub(super) fn slash_enter(
        &mut self,
        _: &SlashEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = self.composer.read(cx).value().to_string();
        match self.slash_pick(window, cx) {
            Some(command) if command.insert != draft => self.slash_insert(&command, window, cx),
            _ => cx.propagate(),
        }
    }

    pub(super) fn slash_insert(
        &mut self,
        command: &SlashCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = format!("{} ", command.insert);
        // Cleared then inserted, so the cursor ends after the command,
        // ready for its arguments (`set_value` leaves it at the start).
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.insert(text, window, cx);
        });
        self.extras.slash_cursor = None;
        self.composer.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    /// Esc: shut the menu for this draft; it reopens once the draft changes.
    pub(super) fn slash_close(&mut self, _: &SlashClose, _: &mut Window, cx: &mut Context<Self>) {
        if self.slash_menu(cx).is_empty() {
            cx.propagate();
            return;
        }
        self.extras.slash_dismissed = self
            .slash_owner(cx)
            .map(|(session, draft, _)| (session, draft));
        cx.notify();
    }

    /// The open menu, drawn inside the composer above the input.
    pub(super) fn render_slash_menu(&self, rows: Vec<SlashCommand>, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let highlight = self.slash_highlight(rows.len(), cx);
        div()
            .debug_selector(|| "slash-menu".into())
            .flex()
            .flex_col()
            .gap(px(2.))
            .pb_1()
            .border_b_1()
            .border_color(gpui::Hsla::from(rgb(p.border)).opacity(0.55))
            .children(rows.into_iter().enumerate().map(|(ix, command)| {
                let selected = ix == highlight;
                let description = command.description.clone();
                let label = command.insert.clone();
                div()
                    .id(("slash-item", ix))
                    .debug_selector(move || format!("slash-item-{ix}"))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(rgb(p.selected)))
                    .hover(|d| d.bg(rgb(p.selected)))
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(if selected { p.accent } else { p.text }))
                            .child(label),
                    )
                    .when(!description.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child(description),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.slash_insert(&command, window, cx)
                    }))
            }))
    }
}
