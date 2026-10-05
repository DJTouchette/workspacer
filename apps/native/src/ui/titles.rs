//! Settings for automatic session names (`agents.autoTitle`). The owning hub
//! writes titles (see `services/hub-rs/src/services/sessions/titles.rs`), so
//! these are the hub's shared settings, read and verified like child access.
//! Controls mirror desktop Settings → Session: on/off, the harness that writes
//! titles ("Agent's own" by default), and that harness's model from its live
//! catalog, where Provider default is the cheap per-CLI default.
use super::*;
use gpui_component::{Disableable, select::SelectEvent, switch::Switch};
use launch::PickerItem;
use serde_json::Value;
use wks_native::features::{Request, TitleChange};

/// Native catalogs (and the provider choices) cover these harnesses.
const HARNESSES: [(&str, &str); 2] = [("claude", "Claude"), ("codex", "Codex")];

pub(super) fn harness_label(id: &str) -> String {
    HARNESSES
        .iter()
        .find(|(value, _)| *value == id)
        .map(|(_, label)| (*label).to_owned())
        .unwrap_or_else(|| id.to_owned())
}

/// Dropdown rows for one harness: Provider default, the catalog, and a
/// configured ID the catalog does not list (shown, never substituted).
pub(super) fn title_model_items(
    harness: &str,
    models: &[ModelChoice],
    configured: &str,
) -> Vec<PickerItem> {
    // Claude's title default is Haiku on the hub and desktop, not the CLI's
    // chat default the catalog marks; other harnesses use their CLI's own.
    let default = if harness == "claude" {
        "Default · Haiku".to_owned()
    } else {
        models
            .iter()
            .find(|m| m.is_default)
            .map(|m| format!("Provider default · {}", m.label))
            .unwrap_or_else(|| "Provider default".into())
    };
    let mut items = vec![PickerItem::new("", default, "title-model-option")];
    items.extend(
        models
            .iter()
            .map(|m| PickerItem::new(m.id.clone(), m.picker_label(), "title-model-option")),
    );
    if !configured.is_empty() && !models.iter().any(|m| m.id == configured) {
        items.push(PickerItem::new(
            configured,
            format!("{configured} · configured"),
            "title-model-option",
        ));
    }
    items
}

impl Workspace {
    fn title_value(&self) -> Option<&Value> {
        self.extras.titles.as_ref()
    }

    /// The harness whose title model the picker edits: the pinned one, or the
    /// row chosen under "Model for" while titles follow each agent.
    pub(super) fn title_harness(&self) -> String {
        match self
            .title_value()
            .and_then(|v| v["provider"].as_str())
            .filter(|p| !p.is_empty())
        {
            Some(pinned) => pinned.to_owned(),
            None => self.extras.title_harness.to_owned(),
        }
    }

    fn title_model(&self) -> String {
        let harness = self.title_harness();
        self.title_value()
            .map(|v| wks_native::features::effective_title_model(v, &harness))
            .unwrap_or_default()
    }

    fn title_catalog_key(&self) -> CatalogKey {
        CatalogKey {
            provider: self.title_harness(),
            cwd: String::new(),
        }
    }

    /// Read the hub's settings and the edited harness's catalog for Settings.
    pub(super) fn load_titles(&mut self, cx: &mut Context<Self>) {
        if self.demo || !self.view.connected {
            return;
        }
        if !self.view.requests.get("titles").is_some_and(|s| s.loading) {
            self.request(Request::Titles { set: None }, cx);
        }
        self.load_title_models(false, cx);
    }

    fn load_title_models(&mut self, refresh: bool, cx: &mut Context<Self>) {
        let key = self.title_catalog_key();
        if self.demo || !self.view.connected || !HARNESSES.iter().any(|(id, _)| *id == key.provider)
        {
            return;
        }
        self.command(Command::LoadModels { key, refresh }, cx);
    }

    /// One change per user action: a repeat of the change already in flight
    /// (a menu confirming twice) is not sent again.
    fn change_titles(&mut self, change: TitleChange, cx: &mut Context<Self>) {
        if let Some(state) = self.view.requests.get("titles")
            && state.loading
            && matches!(&state.request, Request::Titles { set: Some(sent) } if *sent == change)
        {
            return;
        }
        if self.extras.title_sent.as_ref() == Some(&change)
            && self.extras.titles_receipt < self.extras.title_sent_after
        {
            return;
        }
        self.extras.title_sent = Some(change.clone());
        self.extras.title_sent_after = self.extras.titles_receipt + 1;
        self.request(Request::Titles { set: Some(change) }, cx);
    }

    /// Keep the dropdown on the edited harness's catalog and saved choice.
    /// Rebuilt only when either changes, so an open menu is not reset.
    pub(super) fn sync_title_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.title_catalog_key();
        let catalog = &self.view.catalog;
        let models: &[ModelChoice] = if catalog.key == key {
            &catalog.models
        } else {
            &[]
        };
        let selected = self.title_model();
        let items = title_model_items(&key.provider, models, &selected);
        let signature = format!(
            "{}|{}|{}",
            key.provider,
            selected,
            items.iter().map(|i| i.id()).collect::<Vec<_>>().join(",")
        );
        if signature == self.extras.title_picker_key {
            return;
        }
        self.extras.title_picker_key = signature;
        self.extras.title_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(items), window, cx);
            picker.set_selected_value(&selected, window, cx);
        });
    }

    pub(super) fn on_title_pick(
        &mut self,
        _: &Entity<SelectState<SearchableVec<PickerItem>>>,
        event: &SelectEvent<SearchableVec<PickerItem>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(value) = event;
        let model = value.clone().unwrap_or_default();
        if self.extras.titles.is_none() || model == self.title_model() {
            return;
        }
        let provider = self.title_harness();
        self.change_titles(TitleChange::Model { provider, model }, cx);
    }

    fn titles_writable(&self) -> bool {
        self.view.connected
            && !self.demo
            && self.extras.titles.is_some()
            && !self.view.requests.get("titles").is_some_and(|s| s.loading)
    }

    fn titles_status(&self) -> Option<(String, chrome::Tone)> {
        let state = self.view.requests.get("titles");
        let error = state.filter(|s| !s.loading).and_then(|s| s.error.clone());
        Some(match (self.title_value(), error) {
            (_, Some(error)) => (
                format!("Couldn't reach the hub's setting: {error}"),
                chrome::Tone::Error,
            ),
            (None, None) if self.view.connected && !self.demo => {
                ("Reading the hub's setting…".into(), chrome::Tone::Loading)
            }
            (None, None) => (
                "Connect to a hub to change this.".into(),
                chrome::Tone::Info,
            ),
            (Some(_), None) => return None,
        })
    }

    pub(super) fn render_titles_switch(&self, cx: &mut Context<Self>) -> Div {
        let enabled = self.title_value().is_none_or(|v| v["enabled"] != false);
        div()
            .debug_selector(|| "auto-title-switch".into())
            .flex_shrink_0()
            .child(
                Switch::new("settings-auto-title")
                    .checked(self.title_value().is_some() && enabled)
                    .disabled(!self.titles_writable())
                    .tooltip("Name sessions automatically")
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.change_titles(TitleChange::Enabled(*checked), cx);
                    })),
            )
    }

    pub(super) fn render_title_model(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let writable = self.titles_writable();
        let pinned = self
            .title_value()
            .and_then(|v| v["provider"].as_str())
            .unwrap_or("")
            .to_owned();
        let harness = self.title_harness();
        let mut providers: Vec<(&'static str, String)> = vec![("", "Agent's own".into())];
        providers.extend(
            HARNESSES
                .iter()
                .map(|(id, label)| (*id, (*label).to_owned())),
        );
        let pinned_choice = providers
            .iter()
            .map(|(id, _)| *id)
            .find(|id| *id == pinned)
            .unwrap_or("");
        let mut controls =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(self.segmented_enabled(
                    "title-provider",
                    providers,
                    pinned_choice,
                    writable,
                    |this, provider, _, cx| {
                        if !provider.is_empty() {
                            this.extras.title_harness = provider;
                        }
                        this.change_titles(TitleChange::Provider(provider.into()), cx);
                        this.load_title_models(false, cx);
                    },
                    cx,
                ));
        // A provider this client has no catalog for (set on desktop) is kept
        // and named rather than shown as "Agent's own".
        if !pinned.is_empty() && pinned_choice.is_empty() {
            controls = controls.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(format!("Pinned to {} on this hub", harness_label(&pinned))),
            );
        }
        let catalog = &self.view.catalog;
        let current = catalog.key == self.title_catalog_key();
        let catalog_note = if current && catalog.loading {
            Some(("Loading models…".to_owned(), chrome::Tone::Loading))
        } else if current && catalog.error.is_some() {
            Some((
                catalog.error.clone().unwrap_or_default(),
                chrome::Tone::Warning,
            ))
        } else {
            None
        };
        let model_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .when(pinned.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child("Model for"),
                )
                .child(
                    self.segmented_enabled(
                        "title-harness",
                        HARNESSES
                            .iter()
                            .map(|(id, label)| (*id, (*label).to_owned()))
                            .collect(),
                        self.extras.title_harness,
                        writable,
                        |this, harness, window, cx| {
                            this.extras.title_harness = harness;
                            this.sync_title_picker(window, cx);
                            this.load_title_models(false, cx);
                            cx.notify();
                        },
                        cx,
                    ),
                )
            })
            .child(
                div()
                    .debug_selector(|| "title-model-picker".into())
                    .flex_1()
                    .min_w(px(180.))
                    .max_w(px(320.))
                    .child(
                        Select::new(&self.extras.title_picker)
                            .disabled(!writable)
                            .search_placeholder("Find a model…"),
                    ),
            );
        let explain = if pinned.is_empty() {
            format!(
                "Each session is titled by its own provider; this edits the {} title model.",
                harness_label(&harness)
            )
        } else {
            format!(
                "Every session is titled by {}. A model it rejects is not swapped: the session keeps the first line of your request.",
                harness_label(&pinned)
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(controls)
            .child(model_row)
            .child(
                div()
                    .debug_selector(|| "title-model-note".into())
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(explain),
            )
            .children(
                catalog_note
                    .or_else(|| self.titles_status())
                    .map(|(text, tone)| {
                        div()
                            .debug_selector(|| "title-status".into())
                            .child(chrome::notice_line(text, tone, p, "title-status"))
                    }),
            )
    }
}
