use super::*;
use gpui_component::select::SelectItem;

#[derive(Clone)]
pub(super) struct PickerItem {
    id: String,
    label: String,
    /// Test selector prefix: `model-option` or `effort-option`.
    kind: &'static str,
}
impl SelectItem for PickerItem {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &String {
        &self.id
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (id, kind) = (self.id.clone(), self.kind);
        div()
            .debug_selector(move || format!("{kind}-{id}"))
            .child(self.title())
    }
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query) || self.id.to_lowercase().contains(&query)
    }
}

pub(super) fn model_items(models: &[ModelChoice]) -> Vec<PickerItem> {
    std::iter::once(PickerItem {
        id: String::new(),
        label: models
            .iter()
            .find(|m| m.is_default)
            .map(|m| format!("Provider default · {}", m.label))
            .unwrap_or_else(|| "Provider default".into()),
        kind: "model-option",
    })
    .chain(models.iter().map(|m| PickerItem {
        id: m.id.clone(),
        label: m.picker_label(),
        kind: "model-option",
    }))
    .chain(std::iter::once(PickerItem {
        id: "__custom".into(),
        label: "Custom model…".into(),
        kind: "model-option",
    }))
    .collect()
}

/// "Default" first (naming the level it resolves to when the catalog says),
/// then each level the chosen model accepts.
pub(super) fn effort_items(levels: &[String], default: Option<&str>) -> Vec<PickerItem> {
    std::iter::once(PickerItem {
        id: String::new(),
        label: default
            .map(|d| format!("Default · {}", wks_native::launch::effort_label(d)))
            .unwrap_or_else(|| "Default".into()),
        kind: "effort-option",
    })
    .chain(levels.iter().map(|id| PickerItem {
        id: id.clone(),
        label: wks_native::launch::effort_label(id),
        kind: "effort-option",
    }))
    .collect()
}

impl Workspace {
    pub(super) fn on_model_pick(
        &mut self,
        _: &Entity<SelectState<SearchableVec<PickerItem>>>,
        event: &SelectEvent<SearchableVec<PickerItem>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(value) = event;
        if self.spawn_pending || self.view.creating {
            return;
        }
        self.model_choice = value.clone().unwrap_or_default();
        self.context_window = self
            .catalog_models
            .iter()
            .find(|m| m.id == self.model_choice)
            .and_then(|m| m.windows.first())
            .copied();
        if self.model_choice == "__custom" {
            self.model.update(cx, |input, cx| input.focus(window, cx));
        }
        self.reconcile_effort(window, cx);
        cx.notify();
    }

    pub(super) fn on_effort_pick(
        &mut self,
        _: &Entity<SelectState<SearchableVec<PickerItem>>>,
        event: &SelectEvent<SearchableVec<PickerItem>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(value) = event;
        if self.spawn_pending || self.view.creating {
            return;
        }
        self.effort = value.clone().unwrap_or_default();
        cx.notify();
    }

    /// The levels the current provider/model accepts and the default it
    /// resolves to, when the catalog reports one.
    pub(super) fn effort_options(&self) -> (Vec<String>, Option<String>) {
        let model = self
            .catalog_models
            .iter()
            .find(|m| m.id == self.model_choice);
        let resolved = model.or_else(|| {
            (self.model_choice.is_empty())
                .then(|| self.catalog_models.iter().find(|m| m.is_default))
                .flatten()
        });
        (
            wks_native::launch::effort_levels(self.provider, model, &self.catalog_models),
            resolved.and_then(|m| m.default_effort.clone()),
        )
    }

    /// Keep the effort valid for the chosen model: a level it does not accept
    /// returns to Default rather than being sent and rejected, and the menu
    /// lists exactly what the model offers.
    pub(super) fn reconcile_effort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (levels, default) = self.effort_options();
        if !self.effort.is_empty() && !levels.contains(&self.effort) {
            self.effort.clear();
        }
        let key = format!(
            "{}|{}|{}",
            levels.join(","),
            default.as_deref().unwrap_or(""),
            self.effort
        );
        if key == self.effort_key {
            return;
        }
        self.effort_key = key;
        let items = effort_items(&levels, default.as_deref());
        let choice = self.effort.clone();
        let picker = cx.new(|cx| {
            let mut picker = SelectState::new(SearchableVec::new(items), None, window, cx);
            picker.set_selected_value(&choice, window, cx);
            picker
        });
        self.effort_picker_subscription = cx.subscribe_in(&picker, window, Self::on_effort_pick);
        self.effort_picker = picker;
    }

    /// A new launch/provider gets a fresh query and scroll state, not the old
    /// dropdown's filtered list. Replacing the subscription also releases it.
    pub(super) fn reset_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = model_items(&self.catalog_models);
        let choice = self.model_choice.clone();
        let picker = cx.new(|cx| {
            let mut picker =
                SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true);
            picker.set_selected_value(&choice, window, cx);
            picker
        });
        self.model_picker_subscription = cx.subscribe_in(&picker, window, Self::on_model_pick);
        self.model_picker = picker;
    }

    /// The New Agent screen: project first, then who does the work and what
    /// it should do, with consequential options summarized above their fold.
    pub(super) fn render_new_session(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let busy = self.spawn_pending || self.view.creating;
        let can_create = self.view.connected && !busy && !self.demo;
        let resume = self.extras.resume.is_some();
        // Two columns only where both stay readable beside the sidebar.
        let wide = window.viewport_size().width >= px(1100.);
        // Short windows give the form, not the heading, the height.
        let short = window.viewport_size().height < px(620.);
        let card = || {
            div()
                .p(px(if short { 12. } else { 16. }))
                .rounded(px(p.panel_radius))
                .bg(rgb(p.surface))
                .border_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_col()
                .gap_3()
        };
        let header = div()
            .flex()
            .items_start()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(overline(
                        if resume {
                            "CONTINUE A CONVERSATION"
                        } else {
                            "NEW AGENT"
                        },
                        p,
                    ))
                    .child(
                        div()
                            .debug_selector(|| "launch-title".into())
                            .text_size(px(if short { 17. } else { 22. }))
                            .font_weight(FontWeight::BOLD)
                            .child(if resume {
                                "Pick up where you left off"
                            } else {
                                "Start an agent"
                            }),
                    ),
            )
            .child(
                self.icon_button(
                    "cancel-create",
                    "Back to conversation",
                    IconName::Close,
                    !busy,
                )
                .when(!busy, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| {
                        this.show_screen(Screen::Conversation, window, cx)
                    }))
                }),
            );
        let agent = card()
            .child(projects::section_label("Agent", p))
            .child(self.render_provider_choice(busy, cx))
            .child(self.render_model_select(busy, true, cx));
        let task =
            card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(projects::section_label("Task", p))
                        .child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                            if resume {
                                "Optional · sent after the conversation resumes"
                            } else {
                                "Optional · you can also start empty"
                            },
                        )),
                )
                .child(
                    div()
                        .debug_selector(|| "launch-prompt".into())
                        .flex_1()
                        .min_h(px(96.))
                        .flex()
                        .flex_col()
                        .child(
                            Input::new(&self.prompt)
                                .appearance(false)
                                .flex_1()
                                .min_h(px(96.))
                                .disabled(busy),
                        ),
                );
        let content = div()
            .max_w(px(if wide { 980. } else { 720. }))
            .mx_auto()
            .py_2()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(card().child(self.render_project_section(busy, cx)))
            .map(|d| {
                if wide {
                    d.child(
                        div()
                            .flex()
                            .gap_4()
                            .child(agent.w(px(380.)).flex_shrink_0())
                            .child(task.flex_1().min_w_0()),
                    )
                } else {
                    d.child(agent).child(task)
                }
            })
            .child(self.render_launch_disclosure(busy, cx))
            .when(resume, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(p.accent))
                        .child(Icon::new(IconName::Info).size(px(12.)))
                        .child(
                            "Your previous conversation will be resumed with the choices above.",
                        ),
                )
            });
        div()
            .id("new-session-form")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("launch-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(if short { 12. } else { 20. }))
                    .child(content),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_5()
                    .py_3()
                    .bg(rgb(p.base))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    // A flex row gives the footer a definite width, so a long
                    // error wraps instead of widening it.
                    .flex()
                    .justify_center()
                    .child(self.render_launch_footer(busy, can_create, wide, cx)),
            )
    }

    fn render_provider_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div().flex().gap_2().children(
            [
                ("claude", "Claude", "Claude Code", IconName::Bot),
                ("codex", "Codex", "OpenAI Codex", IconName::SquareTerminal),
            ]
            .into_iter()
            .map(|(provider, name, subtitle, icon)| {
                let selected = self.provider == provider;
                chrome::interactive_control(div().id(provider), p, !busy)
                    .debug_selector(move || format!("launch-provider-{provider}"))
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .rounded(px(p.control_radius))
                    .bg(rgb(if selected { p.selected } else { p.chat }))
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!busy, |d| {
                        d.hover(|s| s.bg(rgb(p.selected))).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.choose_provider(provider, window, cx);
                                this.load_models(false, cx);
                                cx.notify();
                            },
                        ))
                    })
                    .child(
                        Icon::new(icon)
                            .size(px(18.))
                            .flex_shrink_0()
                            .text_color(rgb(if selected { p.accent } else { p.muted })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child(subtitle),
                            ),
                    )
                    .when(selected, |d| {
                        d.child(
                            Icon::new(IconName::Check)
                                .size(px(14.))
                                .text_color(rgb(p.accent)),
                        )
                    })
            }),
        )
    }

    /// Non-default options, readable without opening the fold.
    fn launch_option_chips(&self, cx: &App) -> Vec<(String, u32)> {
        let p = self.appearance.palette();
        let mut chips = Vec::new();
        let default_access = self.settings.default_access(self.provider);
        chips.push((
            self.permission.label().to_owned(),
            if self.permission == Permission::FullAccess {
                p.warning
            } else if self.permission != default_access {
                p.accent
            } else {
                p.muted
            },
        ));
        if let Some(tokens) = self.context_window {
            chips.push((format!("{} context", context_label(tokens)), p.accent));
        }
        let label = self.label.read(cx).value().trim().to_owned();
        if !label.is_empty() {
            chips.push((format!("Named “{label}”"), p.accent));
        }
        chips
    }

    fn render_launch_disclosure(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let open = self.launch_details_open;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        self.quiet_button(
                            "launch-customize",
                            "Options",
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            !busy,
                        )
                        .debug_selector(|| "launch-customize".into())
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                if this.launch_details_open {
                                    window.focus(&this.focus);
                                }
                                this.launch_details_open = !this.launch_details_open;
                                cx.notify();
                            }))
                        }),
                    )
                    .children(self.launch_option_chips(cx).into_iter().enumerate().map(
                        |(ix, (text, color))| {
                            div()
                                .debug_selector(move || format!("launch-chip-{ix}"))
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(p.border))
                                .text_size(px(11.))
                                .text_color(rgb(color))
                                .child(text)
                        },
                    )),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .debug_selector(|| "launch-details".into())
                        .p_4()
                        .rounded(px(p.panel_radius))
                        .bg(rgb(p.surface))
                        .border_1()
                        .border_color(rgb(p.border))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(self.render_access_options(busy, cx))
                        .child(self.render_context_choice(busy, cx))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(projects::section_label("Session name", p))
                                .child(Input::new(&self.label).disabled(busy)),
                        )
                        .child(
                            div().flex().flex_wrap().gap_2().child(
                                self.quiet_button(
                                    "launch-setup",
                                    "Agent setup",
                                    IconName::Settings2,
                                    !busy,
                                )
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.open_feature(Screen::Setup, window, cx)
                                    }))
                                }),
                            ),
                        ),
                )
            })
    }

    fn render_launch_footer(
        &self,
        busy: bool,
        can_create: bool,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let model = if self.model_choice == "__custom" {
            let custom = self.model.read(cx).value().trim().to_owned();
            if custom.is_empty() {
                "Custom model".to_owned()
            } else {
                custom
            }
        } else {
            self.catalog_models
                .iter()
                .find(|model| model.id == self.model_choice)
                .map(|model| model.label.clone())
                .unwrap_or_else(|| {
                    if self.model_choice.is_empty() {
                        "Provider default".into()
                    } else {
                        self.model_choice.clone()
                    }
                })
        };
        let provider = if self.provider == "codex" {
            "Codex"
        } else {
            "Claude"
        };
        let model = if self.effort.is_empty() {
            model
        } else {
            format!(
                "{model} · {} effort",
                wks_native::launch::effort_label(&self.effort)
            )
        };
        let project = (!self.projects.cwd.is_empty()).then(|| {
            self.known_project(&self.projects.cwd)
                .map(|p| p.title().to_owned())
                .unwrap_or_else(|| wks_native::projects::basename(&self.projects.cwd).to_owned())
        });
        // Short validation fits the footer line; a hub/launch error is shown
        // whole above it, since its wording says what may have happened.
        let detailed = self.spawn_error.chars().count() > 72;
        let (icon, color, status): (Option<IconName>, u32, String) = if !self.spawn_error.is_empty()
        {
            (
                Some(IconName::TriangleAlert),
                p.warning,
                if detailed {
                    "Couldn't start the agent".into()
                } else {
                    self.spawn_error.clone()
                },
            )
        } else if busy {
            (
                None,
                p.muted,
                format!(
                    "Starting {provider} in {}…",
                    project.clone().unwrap_or_default()
                ),
            )
        } else if !self.view.connected {
            (
                Some(IconName::TriangleAlert),
                p.warning,
                "Waiting for the hub connection…".into(),
            )
        } else if let Some(project) = &project {
            (None, p.muted, format!("{provider} · {model} · {project}"))
        } else {
            (None, p.muted, "Choose a project to start".into())
        };
        let row = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .id("launch-status")
                    .debug_selector(|| "launch-status".into())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .when_some(icon, |d, icon| {
                        d.child(Icon::new(icon).size(px(13.)).flex_shrink_0())
                    })
                    .when(busy, |d| {
                        d.child(work::spinner("launch-spinner".into(), 13.))
                    })
                    .child(div().min_w_0().truncate().child(status)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .child(keycap(
                        if cfg!(target_os = "macos") {
                            "⌘ Enter"
                        } else {
                            "Ctrl Enter"
                        },
                        p,
                    ))
                    .child(
                        self.primary_button(
                            "create-session",
                            if busy {
                                "Starting…"
                            } else if self.extras.resume.is_some() {
                                "Continue"
                            } else {
                                "Start agent"
                            },
                            can_create,
                        )
                        .debug_selector(|| "launch-start".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(IconName::ArrowRight).size(px(14.)))
                        .when(can_create, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| this.create(window, cx)))
                        }),
                    ),
            );
        div()
            .flex_1()
            .min_w_0()
            .max_w(px(if wide { 980. } else { 720. }))
            .flex()
            .flex_col()
            .gap_2()
            .when(detailed, |d| {
                d.child(
                    div()
                        .debug_selector(|| "launch-error".into())
                        .w_full()
                        // Definite height: wrapped text is measured after the
                        // footer's height is settled, so it must not grow it.
                        .h(px(70.))
                        .overflow_hidden()
                        .p_2()
                        .rounded(px(p.control_radius))
                        .border_1()
                        .border_color(rgb(p.warning))
                        .flex()
                        .items_start()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(p.warning))
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(px(13.))
                                .flex_shrink_0(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_clamp(3)
                                .child(self.spawn_error.clone()),
                        ),
                )
            })
            .child(row)
    }

    fn catalog_key(&self, _cx: &App) -> CatalogKey {
        CatalogKey {
            provider: self.provider.into(),
            cwd: if self.provider == "claude" {
                String::new()
            } else {
                self.projects.cwd.clone()
            },
        }
    }

    pub(super) fn load_models(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.demo || (!self.new_session && self.screen != Screen::Model) {
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
        // Ladders differ per provider; a level chosen for one is not carried.
        self.effort.clear();
        self.permission = self.settings.default_access(provider);
        self.catalog_models.clear();
        self.model
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reset_model_picker(window, cx);
        self.sync_models(window, cx);
        self.reconcile_effort(window, cx);
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
        self.reconcile_effort(window, cx);
    }

    /// Model choice with the catalog's live state; the exact ID is entered
    /// here when Custom is chosen, so it is never hidden behind the fold.
    /// `with_effort` places the effort menu beside the model (New Agent); the
    /// live Model screen switches model only, so it leaves effort out.
    pub(super) fn render_model_select(
        &self,
        busy: bool,
        with_effort: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let catalog = &self.view.catalog;
        let current = catalog.key == self.catalog_key(cx);
        let loading = current && catalog.loading;
        let can_reload = !busy && !loading && self.view.connected;
        let (status, status_color) = if loading {
            ("Loading models…".to_owned(), p.muted)
        } else if current && catalog.error.is_some() {
            (catalog.error.clone().unwrap_or_default(), p.warning)
        } else if current && catalog.models.is_empty() {
            (
                "No models were returned. Use Provider default or enter a custom model ID."
                    .to_owned(),
                p.warning,
            )
        } else if !current && self.provider != "claude" && self.projects.cwd.is_empty() {
            (
                "Codex lists models for the chosen project.".to_owned(),
                p.muted,
            )
        } else if self.catalog_models.is_empty() {
            (
                "Provider default and Custom model are available.".to_owned(),
                p.muted,
            )
        } else if self.provider == "claude" {
            (
                format!(
                    "{} model families from your installed Claude CLI",
                    self.catalog_models.len()
                ),
                p.muted,
            )
        } else {
            (
                format!("{} models from Codex on the hub", self.catalog_models.len()),
                p.muted,
            )
        };
        let model = div()
            .flex_1()
            .min_w(px(180.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(projects::section_label("Model", p))
                    .child(
                        self.quiet_button(
                            "reload-models",
                            if loading { "Loading…" } else { "Refresh" },
                            IconName::Redo,
                            can_reload,
                        )
                        .py(px(2.))
                        .when(can_reload, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.load_models(true, cx)))
                        }),
                    ),
            )
            .child(
                div().debug_selector(|| "launch-model-picker".into()).child(
                    Select::new(&self.model_picker)
                        .disabled(busy)
                        .search_placeholder("Find a model…"),
                ),
            );
        let effort = with_effort.then(|| {
            div()
                .w(px(150.))
                .flex_grow()
                .max_w(px(220.))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .child(projects::section_label("Effort", p)),
                )
                .child(
                    div()
                        .debug_selector(|| "launch-effort-picker".into())
                        .child(Select::new(&self.effort_picker).disabled(busy)),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            // Side by side, wrapping to stacked when the card is narrow.
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(model)
                    .children(effort),
            )
            .when(self.model_choice == "__custom", |d| {
                d.child(
                    div()
                        .debug_selector(|| "launch-custom-model".into())
                        .child(Input::new(&self.model).disabled(busy)),
                )
            })
            .child(
                div()
                    .debug_selector(|| "launch-model-status".into())
                    .text_size(px(11.))
                    .text_color(rgb(status_color))
                    .child(status),
            )
    }

    /// Context windows the chosen model offers; Default only for a custom ID,
    /// whose windows the client cannot know.
    pub(super) fn render_context_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let windows = self
            .catalog_models
            .iter()
            .find(|m| m.id == self.model_choice)
            .map(|m| m.windows.clone())
            .unwrap_or_else(|| self.context_window.into_iter().collect());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(projects::section_label("Context window", p))
            .when(windows.is_empty(), |d| {
                d.child(
                    div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                        "The model's default. Models with a choice of windows list them here.",
                    ),
                )
            })
            .when(!windows.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .when(self.model_choice == "__custom", |d| {
                            d.child(
                                self.button("default-context", "Default", !busy)
                                    .when(self.context_window.is_none(), |d| {
                                        d.bg(rgb(p.selected)).text_color(rgb(p.accent))
                                    })
                                    .when(!busy, |d| {
                                        d.on_click(cx.listener(|this, _, _, cx| {
                                            this.context_window = None;
                                            cx.notify();
                                        }))
                                    }),
                            )
                        })
                        .children(windows.into_iter().map(|tokens| {
                            self.button("context", "", !busy)
                                .id(("context", tokens as usize))
                                .child(context_label(tokens))
                                .when(self.context_window == Some(tokens), |d| {
                                    d.bg(rgb(p.selected)).text_color(rgb(p.accent))
                                })
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.context_window = Some(tokens);
                                        cx.notify();
                                    }))
                                })
                        })),
                )
            })
    }

    pub(super) fn render_model_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.render_model_select(busy, false, cx))
            .child(self.render_context_choice(busy, cx))
    }

    pub(super) fn render_access_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(projects::section_label("Access", p))
            .child(
                div().flex().flex_wrap().gap_2().children(
                    Permission::choices(self.provider)
                        .iter()
                        .copied()
                        .map(|permission| {
                            self.button(permission.label(), permission.label(), !busy)
                                .when(self.permission == permission, |d| {
                                    d.bg(rgb(p.selected)).text_color(rgb(p.accent))
                                })
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.permission = permission;
                                        cx.notify();
                                    }))
                                })
                        }),
                ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(if self.permission == Permission::FullAccess {
                        p.warning
                    } else {
                        p.muted
                    }))
                    .child(self.permission.description()),
            )
    }

    pub(super) fn render_launch_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        self.render_model_options(busy, cx)
            .when(self.screen != Screen::Model, |d| {
                d.child(self.render_access_options(busy, cx))
            })
    }
}

/// `200K`, `1M`.
pub(super) fn context_label(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else {
        format!("{}K", tokens / 1000)
    }
}
