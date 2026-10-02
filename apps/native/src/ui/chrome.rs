//! Shared native chrome: quiet actions, readable sections, and preference rows.
use super::*;
use gpui_component::tooltip::Tooltip;

/// GPUI 0.2.2 replaces the entire text refinement in interaction styles.
/// Seed it from the base style so a color change preserves size, weight and font.
pub(super) trait ControlTextStyle: Sized {
    fn hover_text_style(
        self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self;
    fn active_text_style(
        self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self;
}

impl ControlTextStyle for Stateful<Div> {
    fn hover_text_style(
        mut self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self {
        let text = self.style().text.clone();
        self.hover(|mut style| {
            style.text = text;
            f(style)
        })
    }

    fn active_text_style(
        mut self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self {
        let text = self.style().text.clone();
        self.active(|mut style| {
            style.text = text;
            f(style)
        })
    }
}

/// GPUI owns stable focus handles and Enter/Space activation for focusable divs.
/// Reserve the ring at rest so focus never changes control geometry.
pub(super) fn interactive_control(
    control: Stateful<Div>,
    p: Palette,
    enabled: bool,
) -> Stateful<Div> {
    control
        .border_2()
        .border_color(gpui::rgba(0))
        .when(enabled, |d| {
            d.focusable()
                .tab_stop(true)
                .key_context("NativeControl")
                .cursor_pointer()
                .focus(|s| s.border_color(rgb(p.accent)))
        })
}

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
    /// Compact floating title pill: status, project / title, model chip and
    /// icon actions, sized to its content rather than the whole chat width.
    pub(super) fn render_title_bar(
        &self,
        narrow: bool,
        enabled: bool,
        title: &str,
        session: Option<&Session>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let divider = || div().w(px(1.)).h(px(18.)).flex_shrink_0().bg(rgb(p.border));
        div()
            .id("title-bar")
            .debug_selector(|| "title-bar".into())
            .occlude()
            .max_w_full()
            .min_w_0()
            .h(px(40.))
            .pl_4()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded_full()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .shadow(floating_shadow(p))
            .when_some(session, |d, session| {
                let (label, color) = if self.view.connected {
                    session_status(session, p)
                } else {
                    ("Offline", p.muted)
                };
                d.child(
                    div()
                        .id("title-status")
                        .flex_shrink_0()
                        .tooltip({
                            let label = SharedString::from(label.to_owned());
                            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                        })
                        .child(if self.view.connected && session.working() {
                            brand_spinner(
                                12.,
                                p,
                                SharedString::from(format!("title-working-{}", session.id)),
                            )
                        } else {
                            status_dot(color)
                        }),
                )
                .when(!narrow, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(140.))
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(project_label(&session.cwd).to_owned()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.disabled))
                            .child("/"),
                    )
                })
            })
            .child(
                div()
                    .min_w_0()
                    .max_w(px(if narrow { 220. } else { 360. }))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_owned()),
            )
            .when_some(session.filter(|_| !narrow), |d, session| {
                let provider = match session.provider.as_str() {
                    "claude" => "Claude",
                    "codex" => "Codex",
                    "" => "Agent",
                    other => other,
                };
                let model = if session.model.is_empty() {
                    provider.to_owned()
                } else {
                    format!("{provider} · {}", session.model)
                };
                d.child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(180.))
                        .truncate()
                        .px_2()
                        .py(px(2.))
                        .rounded_full()
                        .bg(rgb(p.selected))
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(model),
                )
            })
            .child(divider())
            .child(self.chat_actions(enabled, cx))
    }

    pub(super) fn chat_actions(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let selected = self.view.selected.is_some();
        let model_enabled = enabled && self.supported_session();
        let can_refresh =
            (self.view.connected && !self.view.loading && !self.view.sessions_loading)
                || (self.view.power_paused && self.view.can_resume_power_pause);
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .flex_shrink_0()
            .child(
                self.icon_button("open-changes", "Changes", IconName::Replace, selected)
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
                self.icon_button(
                    "refresh",
                    if self.view.power_paused {
                        "Reconnect and wake"
                    } else {
                        "Refresh conversation"
                    },
                    IconName::Redo,
                    can_refresh,
                )
                .when(can_refresh, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                }),
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
        interactive_control(div().id(id.into()), p, enabled)
            .px_2()
            .py_2()
            .rounded(px(p.control_radius))
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .text_size(px(12.))
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover_text_style(|s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
                    .active_text_style(|s| s.bg(rgb(p.border)).text_color(rgb(p.text)))
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
        self.icon_button_style(id, label, icon, enabled, false)
    }

    pub(super) fn primary_icon_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        self.icon_button_style(id, label, icon, enabled, true)
    }

    fn icon_button_style(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
        primary: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        interactive_control(div().id(id.into()), p, enabled)
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(primary, |d| {
                d.bg(rgb(if enabled { p.primary } else { p.selected }))
                    .text_color(rgb(if enabled { p.on_primary } else { p.disabled }))
            })
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover_text_style(|s| {
                        s.bg(rgb(if primary { p.primary_hover } else { p.selected }))
                            .text_color(rgb(if primary { p.on_primary } else { p.text }))
                    })
                    .active_text_style(|s| {
                        s.bg(rgb(if primary { p.primary_pressed } else { p.border }))
                            .text_color(rgb(if primary { p.on_primary } else { p.text }))
                    })
            })
            .child(Icon::new(icon).size(px(13.)))
    }
}
