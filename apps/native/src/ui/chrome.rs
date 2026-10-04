//! Shared native chrome: quiet actions, readable sections, and preference rows.
use super::*;
use gpui::AnyElement;
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
/// Anthropic brand clay, as the desktop's `CLAUDE_CLAY`.
pub(super) const CLAUDE_CLAY: u32 = 0xD97757;

/// Provider mark and brand color, following desktop `agentLogos.tsx`: Claude
/// keeps its clay; the OpenAI mark (Codex) takes the text color.
pub(super) fn model_badge(session: &Session, p: Palette, size: f32) -> Div {
    brand_badge(&session.provider, session.display_model(), p, size)
}

/// A provider's brand mark on a rounded tile, for choosing or describing an
/// agent (New Agent, Agent setup). Same marks and colors as [`brand_badge`].
pub(super) fn provider_mark(provider: &str, size: f32, p: Palette) -> Div {
    let (mark, color) = match provider {
        "claude" => (Some("brand/claude.svg"), CLAUDE_CLAY),
        "codex" => (Some("brand/openai.svg"), p.text),
        _ => (None, p.accent),
    };
    let glyph = size * 0.5;
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px((size * 0.3).min(p.control_radius)))
        .bg(rgb(p.selected))
        .flex()
        .items_center()
        .justify_center()
        .child(match mark {
            Some(path) => gpui::svg()
                .path(path)
                .size(px(glyph))
                .text_color(rgb(color))
                .into_any_element(),
            None => Icon::new(IconName::Bot)
                .size(px(glyph))
                .text_color(rgb(color))
                .into_any_element(),
        })
}

/// The badge for any provider and display name (sessions and child agents).
pub(super) fn brand_badge(provider: &str, name: String, p: Palette, size: f32) -> Div {
    let (mark, color) = match provider {
        "claude" => (Some("brand/claude.svg"), CLAUDE_CLAY),
        "codex" => (Some("brand/openai.svg"), p.text),
        _ => (None, p.accent),
    };
    let name = if name.is_empty() {
        match provider {
            "claude" => "Claude".to_owned(),
            "codex" => "Codex".to_owned(),
            "" => "Agent".to_owned(),
            other => other.to_owned(),
        }
    } else {
        name
    };
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w_0()
        .child(match mark {
            Some(path) => gpui::svg()
                .path(path)
                .size(px(size))
                .flex_shrink_0()
                .text_color(rgb(color))
                .into_any_element(),
            None => Icon::new(IconName::Bot)
                .size(px(size))
                .flex_shrink_0()
                .text_color(rgb(color))
                .into_any_element(),
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(color))
                .child(name),
        )
}

/// Native type scale for chrome outside the conversation. Chat prose follows
/// the reader's text-size setting instead; these keep pages consistent.
pub(super) mod scale {
    /// Page titles (Settings, Projects, Changes…); `TITLE_SHORT` in short windows.
    pub const TITLE: f32 = 22.;
    pub const TITLE_SHORT: f32 = 18.;
    /// Card and section headings.
    pub const HEADING: f32 = 15.;
    /// Body copy and control labels on pages.
    pub const BODY: f32 = 13.;
    /// Descriptions, metadata and notices.
    pub const META: f32 = 12.;
    /// Captions, chips and hints.
    pub const CAPTION: f32 = 11.;
    /// Uppercase overlines.
    pub const OVERLINE: f32 = 10.;
}

/// The shared raised surface for page content: settings groups, launch
/// sections and secondary screens all use this one card treatment.
pub(super) fn card(p: Palette) -> Div {
    div()
        .rounded(px(p.panel_radius))
        .bg(rgb(p.surface))
        .border_1()
        .border_color(rgb(p.border))
}

/// What a status line means; color and icon follow from it, so loading is
/// never shown as a warning and a success never reads as an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Info,
    Loading,
    Success,
    Warning,
    Error,
}

/// One wrapped status line with its tone's icon; for notices on pages.
pub(super) fn notice_line(
    text: impl Into<SharedString>,
    tone: Tone,
    p: Palette,
    id: impl Into<gpui::ElementId>,
) -> Div {
    let (color, text_color) = match tone {
        Tone::Info | Tone::Loading => (p.muted, p.muted),
        Tone::Success => (p.success, p.text),
        Tone::Warning => (p.warning, p.warning),
        Tone::Error => (p.error, p.error),
    };
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(scale::META))
        .line_height(gpui::relative(1.45))
        .text_color(rgb(text_color))
        .child(
            div()
                .flex_shrink_0()
                .h(px(scale::META * 1.45))
                .flex()
                .items_center()
                .child(match tone {
                    Tone::Loading => brand_spinner(11., p, id).into_any_element(),
                    Tone::Success => Icon::new(IconName::CircleCheck)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                    Tone::Info => Icon::new(IconName::Info)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                    Tone::Warning | Tone::Error => Icon::new(IconName::TriangleAlert)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                }),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// Free text from the hub or a local action, classified by how it reads.
/// Used where one notice slot carries both confirmations and failures.
pub(super) fn notice_tone(text: &str) -> Tone {
    let lower = text.to_lowercase();
    if lower.ends_with('…') && !lower.contains("fail") && !lower.contains("could not") {
        Tone::Loading
    } else if lower.contains("saved")
        || lower.starts_with("pinned")
        || lower.starts_with("unpinned")
        || lower.ends_with(" applied")
    {
        Tone::Success
    } else if lower.contains("fail")
        || lower.contains("error")
        || lower.contains("could not")
        || lower.contains("couldn't")
        || lower.contains("cannot")
        || lower.contains("unavailable")
    {
        Tone::Error
    } else {
        Tone::Warning
    }
}

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
                        // Static: the composer status line owns the animated
                        // loader and elapsed time, so the pill only signals state.
                        .child(status_dot(color)),
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
                d.child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(180.))
                        .pl(px(6.))
                        .pr_2()
                        .py(px(3.))
                        .rounded_full()
                        .bg(rgb(p.selected))
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(model_badge(session, p, 12.)),
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

    /// A text button for actions that stop or remove something: quiet at
    /// rest, error-toned, never mistaken for the page's primary action.
    pub(super) fn danger_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let tint = |alpha: f32| gpui::Hsla::from(rgb(p.error)).opacity(alpha);
        interactive_control(div().id(id.into()), p, enabled)
            .px_3()
            .py_2()
            .font_weight(FontWeight::MEDIUM)
            .rounded(px(p.control_radius))
            .text_size(px(scale::META))
            .text_color(rgb(if enabled { p.error } else { p.disabled }))
            .when(enabled, |d| {
                d.hover_text_style(move |s| s.bg(tint(0.12)).text_color(rgb(p.error)))
                    .active_text_style(move |s| s.bg(tint(0.2)).text_color(rgb(p.error)))
            })
            .child(label.into())
    }

    /// Scrolling page body for every non-chat screen. Where the app draws its
    /// own caption the top strip is window chrome: it drags the window and
    /// keeps page actions clear of the minimize / maximize / close buttons.
    pub(super) fn page_view(
        &self,
        id: &'static str,
        max_width: f32,
        short: bool,
        content: impl IntoElement,
    ) -> Div {
        let p = self.appearance.palette();
        let caption = custom_caption();
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(p.chat))
            .child(
                div()
                    .id(id)
                    .size_full()
                    .overflow_y_scroll()
                    .px(px(if short { 16. } else { 24. }))
                    .pt(px(if caption {
                        PAGE_CAPTION_INSET
                    } else if short {
                        12.
                    } else {
                        24.
                    }))
                    .pb(px(if short { 16. } else { 32. }))
                    .child(div().w_full().max_w(px(max_width)).mx_auto().child(content)),
            )
            .children(page_drag_strip())
    }

    /// The one page header: optional back action, overline, title and a
    /// wrapped description, with trailing actions on the right.
    pub(super) fn page_header(
        &self,
        back: Option<Stateful<Div>>,
        overline_text: Option<&'static str>,
        title: impl Into<SharedString>,
        description: Option<SharedString>,
        trailing: Option<AnyElement>,
        short: bool,
    ) -> Div {
        let p = self.appearance.palette();
        div()
            .debug_selector(|| "page-header".into())
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .children(back.map(|back| div().flex().child(back.ml(px(-10.)))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(overline_text.map(|text| overline(text, p)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .debug_selector(|| "page-title".into())
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(if short {
                                        scale::TITLE_SHORT
                                    } else {
                                        scale::TITLE
                                    }))
                                    .font_weight(FontWeight::BOLD)
                                    .child(title.into()),
                            )
                            .children(trailing.map(|t| {
                                div()
                                    .debug_selector(|| "page-actions".into())
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(t)
                            })),
                    )
                    .children(description.map(|text| {
                        div()
                            .text_size(px(scale::META))
                            .line_height(gpui::relative(1.45))
                            .text_color(rgb(p.muted))
                            .child(text)
                    })),
            )
    }
}

/// The window-chrome strip across the top of a page, when the app draws its
/// own caption: drags the window, stops short of the caption buttons.
pub(super) fn page_drag_strip() -> Option<Div> {
    custom_caption().then(|| {
        drag_region(div())
            .debug_selector(|| "page-drag-region".into())
            .absolute()
            .top_0()
            .left_0()
            .right(px(CAPTION_WIDTH))
            .h(px(PAGE_CAPTION_INSET - 8.))
    })
}

/// Top inset of pages under an app-drawn caption: the caption's height plus
/// breathing room, all of it drag surface except the caption buttons.
pub(super) const PAGE_CAPTION_INSET: f32 = CAPTION_HEIGHT + 8.;

fn token_label(tokens: u64) -> String {
    match tokens {
        t if t >= 1_000_000 => {
            let m = t as f64 / 1_000_000.;
            if m.fract() < 0.05 {
                format!("{m:.0}M")
            } else {
                format!("{m:.1}M")
            }
        }
        t if t >= 1_000 => format!("{}K", (t as f64 / 1_000.).round() as u64),
        t => t.to_string(),
    }
}

/// Context-window meter, following the desktop status bar's `ctx` gauge: a
/// thin rounded track, green → amber (70%) → red (90%), the percentage, and
/// tokens held of the window in the tooltip. `None` until the runtime reports.
pub(super) fn context_meter(session: &Session, p: Palette) -> Option<Stateful<Div>> {
    let usage = &session.context;
    let (label, pct, tooltip) = match usage.reading() {
        Some(reading) => {
            let pct = reading.pct.clamp(0., 100.);
            let detail = match (reading.tokens, reading.window) {
                (Some(tokens), Some(window)) => format!(
                    "{} of {} tokens in context",
                    token_label(tokens),
                    token_label(window)
                ),
                (Some(tokens), None) => format!("{} tokens in context", token_label(tokens)),
                _ => "Share of the context window in use".to_owned(),
            };
            (
                format!("{}%", pct.round() as u64),
                Some(pct),
                format!("Context {}% · {detail}", pct.round() as u64),
            )
        }
        None if usage.waiting => (
            "—".to_owned(),
            None,
            "The provider reported a context window but not current-request usage yet".to_owned(),
        ),
        None => return None,
    };
    let color = match pct {
        Some(pct) if pct >= 90. => p.error,
        Some(pct) if pct >= 70. => p.warning,
        Some(_) => p.success,
        None => p.muted,
    };
    const TRACK: f32 = 44.;
    Some(
        div()
            .id("context-meter")
            .debug_selector(|| "context-meter".into())
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px_2()
            .h(px(28.))
            .rounded_full()
            .text_size(px(11.))
            .child(div().text_color(rgb(p.muted)).child("ctx"))
            .child(
                div()
                    .w(px(TRACK))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(p.border))
                    .overflow_hidden()
                    .when_some(pct, |d, pct| {
                        d.child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(rgb(color))
                                .w(px(if pct > 0. {
                                    (pct.max(2.) / 100.) as f32 * TRACK
                                } else {
                                    0.
                                })),
                        )
                    }),
            )
            .child(div().text_color(rgb(color)).child(label))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
    )
}

/// Windows draws no system title bar; the app owns the caption buttons and the
/// drag regions. `WKS_NATIVE_CAPTION=1` previews the same chrome elsewhere.
pub(crate) fn custom_caption() -> bool {
    #[cfg(feature = "ui-tests")]
    if FORCE_CAPTION.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    cfg!(target_os = "windows") || std::env::var_os("WKS_NATIVE_CAPTION").is_some()
}

#[cfg(feature = "ui-tests")]
pub(super) static FORCE_CAPTION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Width reserved at the top-right for the caption buttons.
pub(super) const CAPTION_WIDTH: f32 = 46. * 3.;
/// Height of the caption buttons' strip.
pub(super) const CAPTION_HEIGHT: f32 = 32.;

impl Workspace {
    /// Minimize / maximize / close at the window's top-right, painted over
    /// everything. On Windows the OS handles them through hit-test areas, which
    /// keeps native behavior (snap layouts on maximize, close confirmation via
    /// `on_window_should_close`); elsewhere they act on click.
    pub(super) fn render_caption(&self, window: &Window) -> Option<impl IntoElement> {
        if !custom_caption() {
            return None;
        }
        let p = self.appearance.palette();
        let native = cfg!(target_os = "windows");
        let maximized = window.is_maximized();
        let button = |id: &'static str, icon: IconName, area: gpui::WindowControlArea| {
            let close = matches!(area, gpui::WindowControlArea::Close);
            div()
                .id(id)
                .debug_selector(move || format!("caption-{id}"))
                .w(px(46.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.muted))
                .when(close, |d| d.rounded_tr(px(p.panel_radius)))
                .hover(move |s| {
                    if close {
                        s.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff))
                    } else {
                        s.bg(rgb(p.selected)).text_color(rgb(p.text))
                    }
                })
                .when(native, |d| d.window_control_area(area))
                .when(!native, |d| {
                    d.on_click(move |_, window, _| match area {
                        gpui::WindowControlArea::Min => window.minimize_window(),
                        gpui::WindowControlArea::Max => window.zoom_window(),
                        _ => window.remove_window(),
                    })
                })
                .child(Icon::new(icon).size(px(14.)))
        };
        Some(
            gpui::deferred(
                div()
                    .id("window-caption")
                    .debug_selector(|| "window-caption".into())
                    .absolute()
                    .top_0()
                    .right_0()
                    .h(px(CAPTION_HEIGHT))
                    .flex()
                    .occlude()
                    .child(button(
                        "minimize",
                        IconName::WindowMinimize,
                        gpui::WindowControlArea::Min,
                    ))
                    .child(button(
                        "maximize",
                        if maximized {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        },
                        gpui::WindowControlArea::Max,
                    ))
                    .child(button(
                        "close",
                        IconName::WindowClose,
                        gpui::WindowControlArea::Close,
                    )),
            )
            .with_priority(1),
        )
    }
}

/// Marks an otherwise-empty area as the window's title bar (drag to move,
/// double-click to maximize) when the app draws its own caption.
pub(super) fn drag_region<E: InteractiveElement>(element: E) -> E {
    if custom_caption() {
        // GPUI's focusable shell prevents default on mouse-down. Windows sends
        // WM_NCLBUTTONDOWN through those listeners before DefWindowProc starts
        // the move, so the drag surface must exclude the shell's hitbox too.
        // BlockMouse also excludes underlying transcript selection; later
        // occluding title pills/buttons still exclude this drag hitbox.
        let element = element.occlude();
        let element = if cfg!(target_os = "windows") {
            element.window_control_area(gpui::WindowControlArea::Drag)
        } else {
            element
        };
        // Observe the real Div hitbox in the cross-platform harness. This
        // listener changes neither default handling nor event propagation.
        #[cfg(all(test, feature = "ui-tests"))]
        let element = element.on_mouse_down(gpui::MouseButton::Left, |_, _, _| {
            DRAG_HIT.set(true);
        });
        element
    } else {
        element
    }
}

#[cfg(all(test, feature = "ui-tests"))]
thread_local! {
    pub(super) static DRAG_HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
