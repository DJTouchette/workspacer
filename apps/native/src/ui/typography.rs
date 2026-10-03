//! Bundled defaults and searchable device-local typography preferences.
use super::*;
use gpui_component::{Theme, select::SelectItem};
use std::borrow::Cow;

struct RegisteredFonts;
impl gpui::Global for RegisteredFonts {}

pub(super) fn register_fonts(cx: &mut App) {
    if cx.has_global::<RegisteredFonts>() {
        return;
    }
    // GPUI renders a variable font at its default instance only, so markdown
    // bold and semibold headings need static weights registered alongside it.
    if let Err(error) = cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Variable.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!(
            "../../assets/fonts/JetBrainsMono-Variable.ttf"
        )),
        Cow::Borrowed(include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf")),
    ]) {
        log_font_error(error);
    }
    cx.set_global(RegisteredFonts);
}

fn log_font_error(error: anyhow::Error) {
    eprintln!("Could not register native fonts; using platform fallbacks: {error}");
}

#[derive(Clone)]
pub(super) struct FontChoice {
    family: String,
    label: String,
}

impl SelectItem for FontChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &String {
        &self.family
    }
    fn matches(&self, query: &str) -> bool {
        self.label.to_lowercase().contains(&query.to_lowercase())
    }
}

pub(super) struct FontControls {
    pub interface: Entity<SelectState<SearchableVec<FontChoice>>>,
    pub code: Entity<SelectState<SearchableVec<FontChoice>>>,
    available: Vec<String>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl FontControls {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        register_fonts(cx);
        let mut available = cx.text_system().all_font_names();
        available.extend(["Inter".into(), "JetBrains Mono".into()]);
        available.retain(|name| !name.starts_with('.') && !name.trim().is_empty());
        available.sort();
        available.dedup();
        let interface = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(items(&available, "Inter")),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let code = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(items(&available, "JetBrains Mono")),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let mut subscriptions = Vec::new();
        for (picker, code_font) in [(&interface, false), (&code, true)] {
            subscriptions.push(cx.subscribe_in(
                picker,
                window,
                move |this, _, event: &SelectEvent<SearchableVec<FontChoice>>, window, cx| {
                    let SelectEvent::Confirm(Some(family)) = event else {
                        return;
                    };
                    if code_font {
                        this.settings.code_font = family.clone();
                    } else {
                        this.settings.interface_font = family.clone();
                    }
                    this.apply_typography(cx);
                    this.save_settings(cx);
                    window.refresh();
                },
            ));
        }
        let controls = Self {
            interface,
            code,
            available,
            _subscriptions: subscriptions,
        };
        controls.sync(&Settings::default(), window, cx);
        controls
    }

    pub fn sync(&self, settings: &Settings, window: &mut Window, cx: &mut Context<Workspace>) {
        for (picker, family) in [
            (&self.interface, &settings.interface_font),
            (&self.code, &settings.code_font),
        ] {
            picker.update(cx, |picker, cx| {
                picker.set_items(
                    SearchableVec::new(items(&self.available, family)),
                    window,
                    cx,
                );
                picker.set_selected_value(family, window, cx);
            });
        }
    }

    pub(super) fn resolved(&self, family: &str, fallback: &str) -> SharedString {
        if family.is_empty() {
            if fallback == "JetBrains Mono" {
                mono_font().into()
            } else {
                ".SystemUIFont".into()
            }
        } else if self.available.iter().any(|name| name == family) {
            family.to_owned().into()
        } else {
            fallback.to_owned().into()
        }
    }
}

fn items(available: &[String], selected: &str) -> Vec<FontChoice> {
    let mut choices = vec![FontChoice {
        family: String::new(),
        label: "System default".into(),
    }];
    for family in ["Inter", "JetBrains Mono"] {
        choices.push(FontChoice {
            family: family.into(),
            label: format!("{family} · bundled"),
        });
    }
    choices.extend(
        available
            .iter()
            .filter(|name| !matches!(name.as_str(), "Inter" | "JetBrains Mono"))
            .map(|name| FontChoice {
                family: name.clone(),
                label: name.clone(),
            }),
    );
    if !selected.is_empty() && !choices.iter().any(|choice| choice.family == selected) {
        choices.push(FontChoice {
            family: selected.into(),
            label: format!("{selected} · unavailable"),
        });
    }
    choices
}

impl Workspace {
    pub(super) fn apply_typography(&mut self, cx: &mut Context<Self>) {
        let count = self.view.transcript.rows.len();
        let anchor = if self.follow {
            ListOffset {
                item_ix: count,
                offset_in_item: px(0.),
            }
        } else {
            self.scroll_anchor()
        };
        let theme = Theme::global_mut(cx);
        theme.font_family = self.fonts.resolved(&self.settings.interface_font, "Inter");
        theme.mono_font_family = self
            .fonts
            .resolved(&self.settings.code_font, "JetBrains Mono");
        theme.font_size = px(self.settings.text_size.clamp(12, 20) as f32);
        theme.mono_font_size = px((self.settings.text_size.clamp(12, 20) as f32 - 2.).max(12.));
        self.list.splice(0..count, count);
        self.list.scroll_to(anchor);
        // A popped-out viewer draws in its own window from these settings.
        self.refresh_popout(cx);
        cx.notify();
    }
}
