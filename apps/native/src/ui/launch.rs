use super::*;
use gpui_component::select::SelectItem;

#[derive(Clone)]
pub(super) struct PickerItem {
    id: String,
    label: String,
}
impl SelectItem for PickerItem {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &String {
        &self.id
    }
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query) || self.id.to_lowercase().contains(&query)
    }
}

pub(super) fn model_items(models: &[ModelChoice]) -> Vec<PickerItem> {
    std::iter::once(PickerItem {
        id: String::new(),
        label: "Provider default".into(),
    })
    .chain(models.iter().map(|m| PickerItem {
        id: m.id.clone(),
        label: m.label.clone(),
    }))
    .chain(std::iter::once(PickerItem {
        id: "__custom".into(),
        label: "Custom model…".into(),
    }))
    .collect()
}

impl Workspace {
    fn catalog_key(&self, cx: &App) -> CatalogKey {
        CatalogKey {
            provider: self.provider.into(),
            cwd: if self.provider == "claude" {
                String::new()
            } else {
                self.project.read(cx).value().trim().to_owned()
            },
        }
    }

    pub(super) fn load_models(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.demo || !self.new_session {
            return;
        }
        self.command(
            Command::LoadModels {
                key: self.catalog_key(cx),
                refresh,
            },
            cx,
        );
    }

    pub(super) fn choose_provider(
        &mut self,
        provider: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.provider == provider {
            return;
        }
        self.provider = provider;
        self.model_choice.clear();
        self.context_window = None;
        self.permission = Permission::Ask;
        self.catalog_models.clear();
        self.model
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.model_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(model_items(&[])), window, cx);
            picker.set_selected_value(&String::new(), window, cx);
        });
        cx.notify();
    }

    pub(super) fn sync_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let catalog = &self.view.catalog;
        if catalog.key != self.catalog_key(cx) || self.catalog_models == catalog.models {
            return;
        }
        self.catalog_models = catalog.models.clone();
        // Preserve an explicitly selected ID across catalog refreshes. It may
        // have disappeared because the account or provider changed upstream.
        // Show it as custom rather than silently substituting another model.
        if !self.model_choice.is_empty()
            && self.model_choice != "__custom"
            && !self
                .catalog_models
                .iter()
                .any(|m| m.id == self.model_choice)
        {
            let selected = self.model_choice.clone();
            self.model
                .update(cx, |input, cx| input.set_value(selected, window, cx));
            self.model_choice = "__custom".into();
        }
        let items = model_items(&self.catalog_models);
        self.model_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(items), window, cx);
            picker.set_selected_value(&self.model_choice, window, cx);
        });
    }

    pub(super) fn render_launch_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let catalog = &self.view.catalog;
        let current = catalog.key == self.catalog_key(cx);
        let loading = current && catalog.loading;
        let windows = self
            .catalog_models
            .iter()
            .find(|m| m.id == self.model_choice)
            .map(|m| m.windows.clone())
            .unwrap_or_else(|| self.context_window.into_iter().collect());
        div().flex().flex_col().gap_3()
            .child(div().flex().items_center().justify_between()
                .child("Model")
                .child(self.button("reload-models", if loading { "Loading…" } else { "Refresh models" }, !busy && !loading && self.view.connected)
                    .when(!busy && !loading && self.view.connected, |d| d.on_click(cx.listener(|this, _, _, cx| this.load_models(true, cx))))))
            .child(Select::new(&self.model_picker).disabled(busy).search_placeholder("Find a model…"))
            .when(self.model_choice == "__custom", |d| d.child(Input::new(&self.model).disabled(busy)))
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(if self.provider == "claude" {
                "Claude family aliases follow your installed CLI. Custom model accepts an exact ID."
            } else { "Models are queried from Codex on the connected hub." }))
            .when(current && catalog.error.is_some(), |d| d.child(div().text_size(px(12.)).text_color(rgb(p.warning))
                .child(catalog.error.clone().unwrap_or_default())))
            .when(!current && self.provider != "claude", |d| d.child(div().text_size(px(12.)).text_color(rgb(p.muted))
                .child("Models will load for the entered project directory.")))
            .when(!windows.is_empty(), |d| d.child(div().flex().flex_wrap().items_center().gap_2()
                .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child("Context"))
                .when(self.model_choice == "__custom", |d| d.child(self.button("default-context", "Default", !busy)
                    .when(!busy, |d| d.on_click(cx.listener(|this, _, _, cx| { this.context_window = None; cx.notify(); })))))
                .children(windows.into_iter().map(|tokens| {
                    let label = if tokens >= 1_000_000 { format!("{}M", tokens / 1_000_000) } else { format!("{}K", tokens / 1000) };
                    self.button("context", "", !busy).id(("context", tokens as usize)).child(label)
                        .when(self.context_window == Some(tokens), |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)))
                        .when(!busy, |d| d.on_click(cx.listener(move |this, _, _, cx| { this.context_window = Some(tokens); cx.notify(); })))
                }))))
            .child("Permissions")
            .child(div().flex().flex_wrap().gap_2().children(Permission::choices(self.provider).iter().copied().map(|permission| {
                self.button(permission.label(), permission.label(), !busy)
                    .when(self.permission == permission, |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)))
                    .when(!busy, |d| d.on_click(cx.listener(move |this, _, _, cx| { this.permission = permission; cx.notify(); })))
            })))
            .child(div().text_size(px(12.)).text_color(rgb(if self.permission == Permission::FullAccess { p.warning } else { p.muted }))
                .child(self.permission.description()))
    }
}
