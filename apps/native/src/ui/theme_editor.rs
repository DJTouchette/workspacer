//! Custom themes: fork the current look, change any palette color or the
//! corner set, and keep it on this device. Settings → Custom themes renders
//! the editor; the palette it produces drives every token-based surface.
use super::chrome::{Tone, notice_line};
use super::*;
use gpui::AnyElement;
use gpui_component::input::InputEvent;
use wks_native::appearance::{
    Corners, CustomThemes, TOKENS, contrast_checks, custom_themes_path, parse_hex,
};

/// The editor's text fields for the active custom theme.
pub(super) struct ThemeEditor {
    theme_id: String,
    name: Entity<InputState>,
    fields: Vec<(&'static str, Entity<InputState>)>,
    _subscriptions: Vec<gpui::Subscription>,
}

#[cfg(all(test, feature = "ui-tests"))]
impl ThemeEditor {
    pub(super) fn field(&self, key: &str) -> Option<&Entity<InputState>> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, f)| f)
    }
}

/// A palette preview: chrome strip, conversation, a card and the accent. The
/// same miniature for built-in and custom themes.
pub(super) fn theme_tile(
    id: SharedString,
    label: SharedString,
    colors: Palette,
    active: bool,
    p: Palette,
) -> Stateful<Div> {
    let selector = format!("theme-{label}");
    chrome::interactive_control(div().id(id), p, true)
        .debug_selector(move || selector.clone())
        .w(px(148.))
        .flex_shrink_0()
        .rounded(px(p.panel_radius))
        .overflow_hidden()
        .cursor_pointer()
        .border_color(if active { rgb(p.accent) } else { gpui::rgba(0) })
        .bg(rgb(p.base))
        .hover(|style| style.bg(rgb(p.selected)))
        .child(
            div()
                .m_2()
                .h(px(64.))
                .rounded(px(p.control_radius))
                .bg(rgb(colors.chat))
                .flex()
                .overflow_hidden()
                .child(
                    div()
                        .w(px(24.))
                        .h_full()
                        .bg(rgb(colors.base))
                        .p_2()
                        .child(status_dot(colors.accent)),
                )
                .child(
                    div()
                        .flex_1()
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .h(px(6.))
                                .w(px(36.))
                                .rounded_full()
                                .bg(rgb(colors.text)),
                        )
                        .child(
                            div()
                                .h(px(4.))
                                .w_full()
                                .rounded_full()
                                .bg(rgb(colors.border)),
                        )
                        .child(
                            div()
                                .h(px(14.))
                                .w_full()
                                .rounded(px(radius::TAG))
                                .bg(rgb(colors.surface))
                                .flex()
                                .justify_end()
                                .p(px(3.))
                                .child(
                                    div()
                                        .w(px(16.))
                                        .h_full()
                                        .rounded(px(radius::MARK))
                                        .bg(rgb(colors.primary)),
                                ),
                        ),
                ),
        )
        .child(
            div()
                .px_3()
                .pb_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(scale::META))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if active { p.accent } else { p.text }))
                        .child(label),
                )
                .when(active, |d| {
                    d.child(
                        Icon::new(IconName::Check)
                            .size(px(icon_size::LG))
                            .text_color(rgb(p.accent)),
                    )
                }),
        )
}

impl Workspace {
    /// Read this device's custom themes and apply the active one.
    pub fn load_custom_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = custom_themes_path() else {
            return;
        };
        match CustomThemes::load(&path) {
            Ok(themes) => self.custom_themes = themes,
            Err(error) => {
                // Keep the unreadable file: saving over it would lose themes.
                self.theme_error = format!("Could not read custom themes: {error}");
                return;
            }
        }
        self.custom_themes_path = Some(path);
        if self.custom_themes.active().is_some() {
            self.apply_custom_theme(window, cx);
        } else {
            self.custom_themes.active = None;
        }
    }

    pub(super) fn save_custom_themes(&mut self) {
        let Some(path) = &self.custom_themes_path else {
            return;
        };
        if let Err(error) = self.custom_themes.save(path) {
            self.theme_error = format!("Theme applied, but could not save: {error}");
        }
    }

    /// Recompute and apply the active custom theme's palette.
    fn apply_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(theme) = self.custom_themes.active() else {
            return;
        };
        let palette = theme.palette();
        // Syntax colors and the component library's mode follow a built-in
        // that reads the same way: the base, unless the edits flipped it.
        let dark = theme.is_dark();
        let reads_like = if theme.base.is_dark() == dark {
            theme.base
        } else if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        self.appearance = reads_like;
        self.palette_override = Some(palette);
        configure_palette(reads_like, palette, Some(window), cx);
        self.apply_typography(cx);
        self.sync_theme_editor(window, cx);
        cx.notify();
    }

    pub(super) fn choose_custom_theme(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.custom_themes.active = Some(id);
        self.theme_error.clear();
        self.apply_custom_theme(window, cx);
        self.save_custom_themes();
    }

    /// Fork the current look into a new custom theme and open it for editing.
    pub(super) fn create_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .custom_themes
            .active()
            .map(|t| t.base)
            .unwrap_or(self.appearance);
        self.custom_themes.create(base);
        self.theme_error.clear();
        self.apply_custom_theme(window, cx);
        self.save_custom_themes();
    }

    fn delete_custom_theme(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = self.custom_themes.active.as_deref() == Some(id.as_str());
        let Some(base) = self.custom_themes.remove(&id) else {
            return;
        };
        self.theme_editor = None;
        self.save_custom_themes();
        if was_active {
            self.choose_theme(base, window, cx);
        }
        cx.notify();
    }

    fn edit_custom_theme(
        &mut self,
        edit: impl FnOnce(&mut wks_native::appearance::CustomTheme),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(theme) = self.custom_themes.active_mut() else {
            return;
        };
        edit(theme);
        self.apply_custom_theme(window, cx);
        self.save_custom_themes();
    }

    /// Build the editor's fields for the active theme, or refresh their
    /// text after the theme changed some other way (reset, switch).
    fn sync_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(theme) = self.custom_themes.active().cloned() else {
            self.theme_editor = None;
            return;
        };
        let palette = theme.palette();
        if self.theme_editor.is_none() {
            let mut subscriptions = Vec::new();
            let name = cx.new(|cx| InputState::new(window, cx).placeholder("Theme name"));
            subscriptions.push(cx.subscribe_in(
                &name,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let text = input.read(cx).value().trim().to_owned();
                    let unchanged = this.custom_themes.active().is_none_or(|t| t.name == text);
                    if !text.is_empty() && !unchanged {
                        this.edit_custom_theme(|t| t.name = text, window, cx);
                    }
                },
            ));
            let mut fields = Vec::new();
            for token in TOKENS {
                let key = token.key;
                let field = cx.new(|cx| InputState::new(window, cx).placeholder("#rrggbb"));
                subscriptions.push(cx.subscribe_in(
                    &field,
                    window,
                    move |this, input, event: &InputEvent, window, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        let Some(color) = parse_hex(&input.read(cx).value(), key == "selection")
                        else {
                            return;
                        };
                        // Our own resync lands here too; equal means no edit.
                        if this.palette().get(key) != Some(color) {
                            this.edit_custom_theme(|t| t.set_color(key, Some(color)), window, cx);
                        }
                    },
                ));
                fields.push((key, field));
            }
            self.theme_editor = Some(ThemeEditor {
                theme_id: String::new(),
                name,
                fields,
                _subscriptions: subscriptions,
            });
        }
        let editor = self.theme_editor.as_mut().unwrap();
        let switched = editor.theme_id != theme.id;
        editor.theme_id = theme.id.clone();
        if switched {
            editor.name.update(cx, |input, cx| {
                input.set_value(theme.name.clone(), window, cx)
            });
        }
        for (key, field) in &editor.fields {
            let Some(hex) = palette.hex(key) else {
                continue;
            };
            // Leave a field being typed in alone unless it already parses
            // to something else (a reset or a theme switch).
            let current = field.read(cx).value().to_owned();
            let current_color = parse_hex(&current, *key == "selection");
            if switched || current_color != palette.get(key) {
                field.update(cx, |input, cx| input.set_value(hex, window, cx));
            }
        }
    }

    /// Custom theme tiles followed by the "New theme" tile; used in the
    /// Appearance picker and atop the Custom themes section.
    pub(super) fn custom_theme_tiles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.palette();
        let mut tiles: Vec<AnyElement> = self
            .custom_themes
            .themes
            .iter()
            .map(|theme| {
                let id = theme.id.clone();
                let active = self.custom_themes.active.as_deref() == Some(id.as_str());
                theme_tile(
                    SharedString::from(format!("custom-theme-{id}")),
                    theme.name.clone().into(),
                    theme.palette(),
                    active,
                    p,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_custom_theme(id.clone(), window, cx)
                }))
                .into_any_element()
            })
            .collect();
        tiles.push(
            chrome::interactive_control(div().id("new-custom-theme"), p, true)
                .debug_selector(|| "new-custom-theme".into())
                .w(px(148.))
                .min_h(px(108.))
                .flex_shrink_0()
                .rounded(px(p.panel_radius))
                .border_color(gpui::rgba(0))
                .bg(chrome::tonal(p, alpha::FAINT))
                .hover(|s| s.bg(rgb(p.selected)))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(rgb(p.muted))
                .text_size(px(scale::META))
                .font_weight(FontWeight::MEDIUM)
                .child(Icon::new(IconName::Plus).size(px(icon_size::XL)))
                .child("New theme")
                .on_click(cx.listener(|this, _, window, cx| this.create_custom_theme(window, cx)))
                .into_any_element(),
        );
        tiles
    }

    /// Name field for the active custom theme.
    pub(super) fn render_theme_name(&self) -> Option<Div> {
        let editor = self.theme_editor.as_ref()?;
        Some(div().w(px(240.)).child(Input::new(&editor.name)))
    }

    pub(super) fn render_theme_corners(&self, cx: &mut Context<Self>) -> Option<Div> {
        let theme = self.custom_themes.active()?;
        Some(
            self.segmented(
                "theme-corners",
                Corners::ALL
                    .into_iter()
                    .map(|c| (c, c.label().to_owned()))
                    .collect(),
                theme.corners,
                |this, corners, window, cx| {
                    this.edit_custom_theme(|t| t.corners = corners, window, cx)
                },
                cx,
            ),
        )
    }

    /// Every palette color, grouped, as swatch + name + hex field + reset.
    pub(super) fn render_theme_colors(&self, cx: &mut Context<Self>) -> Option<Div> {
        let theme = self.custom_themes.active()?;
        let editor = self.theme_editor.as_ref()?;
        let p = self.palette();
        let mut groups: Vec<(&'static str, Vec<AnyElement>)> = Vec::new();
        for (token, (_, field)) in TOKENS.iter().zip(&editor.fields) {
            let key = token.key;
            let changed = theme.colors.contains_key(key);
            let color = p.get(key).unwrap_or(0);
            let swatch = if key == "selection" {
                gpui::rgba(color).into()
            } else {
                gpui::Hsla::from(rgb(color))
            };
            let row = div()
                .debug_selector(move || format!("theme-token-{key}"))
                .flex()
                .items_center()
                .gap_3()
                .py_1()
                .child(
                    div()
                        .size(px(control::HEIGHT_SM))
                        .flex_shrink_0()
                        .rounded(px(p.control_radius.min(8.)))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.chat))
                        .child(
                            div()
                                .size_full()
                                .rounded(px(p.control_radius.min(8.)))
                                .bg(swatch),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(scale::BODY))
                                .font_weight(FontWeight::MEDIUM)
                                .child(token.label)
                                .when(changed, |d| d.child(status_dot(p.accent))),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child(token.help),
                        ),
                )
                .child(div().w(px(128.)).flex_shrink_0().child(Input::new(field)))
                .child(
                    self.icon_button(
                        SharedString::from(format!("reset-token-{key}")),
                        "Use the base color",
                        IconName::Undo,
                        changed,
                    )
                    .debug_selector(move || format!("reset-token-{key}"))
                    .when(changed, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_custom_theme(|t| t.set_color(key, None), window, cx)
                        }))
                    }),
                );
            match groups.last_mut() {
                Some((group, rows)) if *group == token.group => rows.push(row.into_any_element()),
                _ => groups.push((token.group, vec![row.into_any_element()])),
            }
        }
        Some(
            div()
                .debug_selector(|| "theme-colors".into())
                .flex()
                .flex_col()
                .gap_4()
                .children(groups.into_iter().map(|(group, rows)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(overline(group.to_uppercase(), p))
                        .children(rows)
                })),
        )
    }

    /// The readability checks the built-ins are held to, live.
    pub(super) fn render_theme_contrast(&self) -> Option<Div> {
        self.custom_themes.active()?;
        let p = self.palette();
        let checks = contrast_checks(&p);
        let failing: Vec<_> = checks.iter().filter(|c| !c.passes()).collect();
        Some(
            div()
                .debug_selector(|| "theme-contrast".into())
                .flex()
                .flex_col()
                .gap_1()
                .when(failing.is_empty(), |d| {
                    d.child(notice_line(
                        format!(
                            "All {} checks pass: text, status colors and buttons stay readable.",
                            checks.len()
                        ),
                        Tone::Success,
                        p,
                    ))
                })
                .children(failing.into_iter().map(|check| {
                    notice_line(
                        format!(
                            "{}: {:.1}:1, aim for at least {}:1",
                            check.label, check.ratio, check.required
                        ),
                        Tone::Warning,
                        p,
                    )
                })),
        )
    }

    pub(super) fn render_theme_actions(&self, cx: &mut Context<Self>) -> Option<Div> {
        let id = self.custom_themes.active.clone()?;
        Some(
            div()
                .flex()
                .gap_2()
                .child(
                    self.button("preview-custom-theme", "Preview components", true)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_screen(Screen::DesignSystem, window, cx)
                        })),
                )
                .child(
                    self.button("duplicate-custom-theme", "Duplicate", true)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_custom_theme(window, cx)),
                        ),
                )
                .child(
                    self.danger_button("delete-custom-theme", "Delete theme", true)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.delete_custom_theme(id.clone(), window, cx)
                        })),
                ),
        )
    }
}
