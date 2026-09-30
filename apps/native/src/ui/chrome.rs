//! Shared native chrome: quiet actions, readable sections, and preference rows.
use super::*;
use gpui_component::tooltip::Tooltip;

/// Shared outer measure and gutters for transcript, header and composer.
pub(super) fn chat_column() -> Div {
    div().w_full().max_w(px(CHAT_WIDTH + 40.)).mx_auto().px_5()
}

pub(super) fn floating_shadow(p: Palette) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: gpui::Hsla::from(rgb(p.shadow)).opacity(p.shadow_opacity),
        offset: gpui::point(px(0.), px(6.)),
        blur_radius: px(20.),
        spread_radius: px(-4.),
    }]
}

pub(super) fn section(title: &'static str, description: &'static str, p: Palette) -> Div {
    div()
        .py_5()
        .border_t_1()
        .border_color(rgb(p.border))
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child(description),
                ),
        )
}

pub(super) fn preference(label: &'static str, description: &'static str, p: Palette) -> Div {
    div().flex().items_center().justify_between().gap_4().child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(description),
            ),
    )
}

pub(super) fn project_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("Workspace")
}

impl Workspace {
    pub(super) fn chat_actions(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let selected = self.view.selected.is_some();
        let model_enabled = enabled && self.supported_session();
        div()
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(
                self.quiet_button("open-changes", "Changes", IconName::Replace, selected)
                    .when(selected, |d| {
                        d.on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Changes, window, cx)
                        }))
                    }),
            )
            .child(
                self.icon_button(
                    "open-history",
                    "Conversation history",
                    IconName::BookOpen,
                    selected,
                )
                .when(selected, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::History, window, cx)
                    }))
                }),
            )
            .child(
                self.icon_button("open-session", "Session details", IconName::Info, selected)
                    .when(selected, |d| {
                        d.on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Session, window, cx)
                        }))
                    }),
            )
            .child(
                self.icon_button(
                    "open-model",
                    "Change model",
                    IconName::Settings2,
                    model_enabled,
                )
                .when(model_enabled, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Model, window, cx)
                    }))
                }),
            )
            .child(
                self.icon_button("refresh", "Refresh conversation", IconName::Redo, true)
                    .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx))),
            )
    }

    pub(super) fn quiet_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        div()
            .id(id.into())
            .px_2()
            .py_2()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .text_size(px(12.))
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
            })
            .child(Icon::new(icon).size(px(12.)))
            .child(label)
    }

    pub(super) fn icon_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        div()
            .id(id.into())
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
            })
            .child(Icon::new(icon).size(px(13.)))
    }
}
