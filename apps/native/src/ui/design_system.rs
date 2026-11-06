//! The design system, live: every palette color, the type scale, controls,
//! status tones and surfaces, drawn with the shared helpers in the active
//! theme. Settings → Appearance (or `g y`) opens it; it is both the reference
//! for building screens and a preview while editing a custom theme.
use super::chrome::{Tone, card, notice_line};
use super::*;
use gpui::AnyElement;
use gpui_component::switch::Switch;
use wks_native::appearance::TOKENS;

/// Width of the gallery's content column.
const GALLERY_WIDTH: f32 = 960.;

/// A labelled specimen: the thing itself above its token name.
fn specimen(name: &'static str, detail: String, item: impl IntoElement, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .min_h(px(control::HEIGHT))
                .flex()
                .items_center()
                .child(item),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(scale::META))
                        .font_weight(FontWeight::MEDIUM)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(detail),
                ),
        )
}

/// A gallery section: a card with a heading, a one-line rule, and content.
fn section(title: &'static str, rule: &'static str, body: impl IntoElement, p: Palette) -> Div {
    card(p)
        .p_4()
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
                        .text_size(px(scale::HEADING))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(scale::META))
                        .line_height(gpui::relative(1.45))
                        .text_color(rgb(p.muted))
                        .child(rule),
                ),
        )
        .child(body)
}

impl Workspace {
    pub(super) fn render_design_system(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let short = window.viewport_size().height < px(620.);
        let theme_name = match self.custom_themes.active() {
            Some(theme) => theme.name.clone(),
            None => self.appearance.label().to_owned(),
        };
        let back = self
            .quiet_button("design-back", "Back to settings", IconName::ArrowLeft, true)
            .debug_selector(|| "design-back".into())
            .on_click(
                cx.listener(|this, _, window, cx| this.show_screen(Screen::Settings, window, cx)),
            );
        let edit = self
            .button("design-edit-theme", "Customize theme…", true)
            .on_click(cx.listener(|this, _, window, cx| {
                this.enter_settings_section(settings::SettingsSection::Themes, cx);
                this.show_screen(Screen::Settings, window, cx)
            }))
            .into_any_element();
        let header = self.page_header(
            Some(back),
            Some("APPEARANCE"),
            "Design system",
            Some(
                format!(
                    "The tokens and components every screen is built from, drawn in {theme_name}. \
                     Use these instead of local styling; see apps/native/DESIGN_SYSTEM.md."
                )
                .into(),
            ),
            Some(edit),
            short,
        );
        self.page_view(
            "design-system",
            GALLERY_WIDTH,
            short,
            div()
                .debug_selector(|| "design-system".into())
                .flex()
                .flex_col()
                .gap_5()
                .child(header)
                .child(self.design_colors(p))
                .child(self.design_type(p))
                .child(self.design_controls(cx))
                .child(self.design_status(p))
                .child(self.design_surfaces(p)),
        )
    }

    fn design_colors(&self, p: Palette) -> Div {
        let mut groups: Vec<(&'static str, Vec<AnyElement>)> = Vec::new();
        for token in TOKENS {
            let color = p.get(token.key).unwrap_or(0);
            let fill = if token.key == "selection" {
                gpui::rgba(color).into()
            } else {
                gpui::Hsla::from(rgb(color))
            };
            let chip = div()
                .w(px(132.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .h(px(44.))
                        .rounded(px(p.control_radius))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.chat))
                        .overflow_hidden()
                        .child(div().size_full().bg(fill)),
                )
                .child(
                    div()
                        .text_size(px(scale::META))
                        .font_weight(FontWeight::MEDIUM)
                        .child(token.label),
                )
                .child(
                    div()
                        .text_size(px(scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .font_family(self.settings.code_font.clone())
                        .child(format!(
                            "{} {}",
                            token.key,
                            p.hex(token.key).unwrap_or_default()
                        )),
                )
                .into_any_element();
            match groups.last_mut() {
                Some((group, chips)) if *group == token.group => chips.push(chip),
                _ => groups.push((token.group, vec![chip])),
            }
        }
        section(
            "Color",
            "Name a palette role (p.text, p.surface…), never a hex value: custom themes change these, and a literal would not follow.",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .children(groups.into_iter().map(|(group, chips)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(overline(group.to_uppercase(), p))
                        .child(div().flex().flex_wrap().gap_3().children(chips))
                })),
            p,
        )
    }

    fn design_type(&self, p: Palette) -> Div {
        let rows = [
            ("TITLE", scale::TITLE, FontWeight::BOLD, "Page titles"),
            (
                "SUBTITLE",
                scale::SUBTITLE,
                FontWeight::SEMIBOLD,
                "A section's title in a page",
            ),
            (
                "HEADING",
                scale::HEADING,
                FontWeight::SEMIBOLD,
                "Card and section headings",
            ),
            (
                "LABEL",
                scale::LABEL,
                FontWeight::MEDIUM,
                "Row titles: sessions, preferences",
            ),
            (
                "BODY",
                scale::BODY,
                FontWeight::NORMAL,
                "Body copy and control labels",
            ),
            (
                "META",
                scale::META,
                FontWeight::NORMAL,
                "Descriptions, notices, button labels",
            ),
            (
                "CAPTION",
                scale::CAPTION,
                FontWeight::NORMAL,
                "Captions, chips and hints",
            ),
            (
                "OVERLINE",
                scale::OVERLINE,
                FontWeight::SEMIBOLD,
                "UPPERCASE OVERLINES",
            ),
        ];
        section(
            "Type",
            "Eight sizes from scale::. Weight carries emphasis; color carries hierarchy (text → prose → muted → faint).",
            div()
                .flex()
                .flex_col()
                .children(
                    rows.into_iter()
                        .enumerate()
                        .map(|(ix, (name, size, weight, sample))| {
                            div()
                                .flex()
                                .items_center()
                                .gap_4()
                                .py_2()
                                .when(ix > 0, |d| d.border_t_1().border_color(rgb(p.border)))
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_shrink_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_size(px(scale::CAPTION))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(p.muted))
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(scale::CAPTION))
                                                .text_color(rgb(p.disabled))
                                                .child(format!("{size} px")),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(size))
                                        .font_weight(weight)
                                        .child(sample),
                                )
                        }),
                )
                .child(
                    div()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .text_size(px(scale::BODY))
                        .child(div().text_color(rgb(p.text)).child("text"))
                        .child(div().text_color(rgb(p.prose)).child("prose"))
                        .child(div().text_color(rgb(p.muted)).child("muted"))
                        .child(div().text_color(rgb(p.disabled)).child("faint"))
                        .child(div().text_color(rgb(p.accent)).child("accent link"))
                        .child(
                            div()
                                .px_1()
                                .rounded(px(radius::TAG))
                                .bg(rgb(p.code_inline))
                                .font_family(self.settings.code_font.clone())
                                .child("inline code"),
                        ),
                ),
            p,
        )
    }

    fn design_controls(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let h = control::HEIGHT;
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .child(specimen(
                "primary_button",
                format!("{h} px · one per view"),
                self.primary_button("ds-primary", "Apply", true),
                p,
            ))
            .child(specimen(
                "button",
                format!("{h} px · tonal secondary"),
                self.button("ds-secondary", "Check connection", true),
                p,
            ))
            .child(specimen(
                "danger_button",
                format!("{h} px · stops or removes"),
                self.danger_button("ds-danger", "End session…", true),
                p,
            ))
            .child(specimen(
                "quiet_button",
                "Header and inline actions".into(),
                self.quiet_button("ds-quiet", "Refresh", IconName::Redo, true),
                p,
            ))
            .child(specimen(
                "pill_button",
                format!("{h} px · composer actions"),
                self.pill_button("ds-pill", "Send now", IconName::ArrowUp, true, true),
                p,
            ))
            .child(specimen(
                "icon_button",
                format!("{} px · always has a tooltip", control::HEIGHT_SM),
                div()
                    .flex()
                    .gap_1()
                    .child(self.icon_button("ds-icon", "Settings", IconName::Settings, true))
                    .child(self.primary_icon_button(
                        "ds-icon-primary",
                        "Send",
                        IconName::ArrowUp,
                        true,
                    )),
                p,
            ))
            .child(specimen(
                "disabled",
                "Same shape, faint".into(),
                div()
                    .flex()
                    .gap_2()
                    .child(self.primary_button("ds-primary-off", "Apply", false))
                    .child(self.button("ds-secondary-off", "Save", false)),
                p,
            ));
        let choices = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .child(specimen(
                "segmented",
                format!("{} px options · small enumerations", control::HEIGHT_SM),
                self.segmented(
                    "ds-segmented",
                    vec![(0u8, "Auto".into()), (1, "Wide".into()), (2, "Tall".into())],
                    0,
                    |_, _, _, _| {},
                    cx,
                ),
                p,
            ))
            .child(specimen(
                "Switch",
                "On/off preferences".into(),
                div()
                    .flex()
                    .gap_3()
                    .child(Switch::new("ds-switch-on").checked(true))
                    .child(Switch::new("ds-switch-off").checked(false)),
                p,
            ))
            .child(specimen(
                "keycap",
                "Shortcut hints".into(),
                div()
                    .flex()
                    .gap_1()
                    .child(keycap("Ctrl", p))
                    .child(keycap("Enter", p)),
                p,
            ));
        section(
            "Controls",
            "Pick by role, not by look: one primary per view, tonal secondaries beside it, danger only for stop/remove. Text buttons share one height so rows line up.",
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(buttons)
                .child(choices),
            p,
        )
    }

    fn design_status(&self, p: Palette) -> Div {
        let badge = |label: &'static str, color: u32| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(22.))
                .px_2()
                .rounded_full()
                .bg(gpui::Hsla::from(rgb(color)).opacity(alpha::TONE_HOVER))
                .text_size(px(scale::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(color))
                .child(status_dot(color))
                .child(label)
        };
        section(
            "Status",
            "A Tone decides color and icon together, so loading never reads as a warning. Status dots and badges use the same four colors.",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(notice_line(
                            "Info: the provider will apply it when ready.",
                            Tone::Info,
                            p,
                        ))
                        .child(notice_line("Loading the conversation…", Tone::Loading, p))
                        .child(notice_line("Saved.", Tone::Success, p))
                        .child(notice_line(
                            "Needs approval before it continues.",
                            Tone::Warning,
                            p,
                        ))
                        .child(notice_line("Could not reach the hub.", Tone::Error, p)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(badge("Ready", p.success))
                        .child(badge("Needs approval", p.warning))
                        .child(badge("Failed", p.error))
                        .child(badge("Working", p.busy))
                        .child(badge("Linked", p.accent)),
                ),
            p,
        )
    }

    fn design_surfaces(&self, p: Palette) -> Div {
        let surface = |name: &'static str, fill: u32, radius_px: f32, detail: String| {
            specimen(
                name,
                detail,
                div()
                    .w(px(132.))
                    .h(px(72.))
                    .rounded(px(radius_px))
                    .bg(rgb(fill))
                    .border_1()
                    .border_color(rgb(p.border)),
                p,
            )
        };
        let icons = [
            ("XS", icon_size::XS),
            ("SM", icon_size::SM),
            ("MD", icon_size::MD),
            ("LG", icon_size::LG),
            ("XL", icon_size::XL),
            ("HERO", icon_size::HERO),
        ];
        let spacing = [1usize, 2, 3, 4, 5, 6, 8];
        section(
            "Surfaces, icons and spacing",
            "Three radii from the palette (a custom theme's Corners sets them together); icon sizes pair with the type scale; spacing is GPUI's 4 px steps (gap_1 … gap_8).",
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_5()
                        .child(surface(
                            "control_radius",
                            p.selected,
                            p.control_radius,
                            format!("{} px · buttons, rows, fields", p.control_radius),
                        ))
                        .child(surface(
                            "panel_radius · card()",
                            p.surface,
                            p.panel_radius,
                            format!("{} px · cards and panels", p.panel_radius),
                        ))
                        .child(surface(
                            "composer_radius",
                            p.chat,
                            p.composer_radius,
                            format!("{} px · the composer", p.composer_radius),
                        ))
                        .child(specimen(
                            "floating_shadow",
                            "Composer, menus, the title capsule".into(),
                            div()
                                .w(px(132.))
                                .h(px(72.))
                                .rounded(px(p.panel_radius))
                                .bg(rgb(p.surface))
                                .shadow(chrome::floating_shadow(p)),
                            p,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_5()
                        .children(icons.into_iter().map(|(name, size)| {
                            specimen(
                                name,
                                format!("icon_size · {size} px"),
                                Icon::new(IconName::Palette)
                                    .size(px(size))
                                    .text_color(rgb(p.text)),
                                p,
                            )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(spacing.into_iter().map(|n| {
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(56.))
                                        .text_size(px(scale::CAPTION))
                                        .text_color(rgb(p.muted))
                                        .child(format!("gap_{n}")),
                                )
                                .child(
                                    div()
                                        .h(px(8.))
                                        .w(px(4. * n as f32))
                                        .rounded(px(radius::HAIRLINE))
                                        .bg(rgb(p.accent)),
                                )
                                .child(
                                    div()
                                        .text_size(px(scale::CAPTION))
                                        .text_color(rgb(p.disabled))
                                        .child(format!("{} px", 4 * n)),
                                )
                        })),
                ),
            p,
        )
    }
}
