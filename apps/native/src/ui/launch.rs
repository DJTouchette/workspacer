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
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        div()
            .debug_selector(move || format!("model-option-{id}"))
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
    })
    .chain(models.iter().map(|m| PickerItem {
        id: m.id.clone(),
        label: m.picker_label(),
    }))
    .chain(std::iter::once(PickerItem {
        id: "__custom".into(),
        label: "Custom model…".into(),
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
        cx.notify();
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

    pub(super) fn render_new_session(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.appearance.palette();
        let busy = self.spawn_pending || self.view.creating;
        let can_create = self.view.connected && !busy;
        let cwd = self.project.read(cx).value().trim().to_owned();
        let suggestions = projects(
            &self.view.sessions,
            self.settings.bookmarks(&self.project_scope),
        )
        .into_iter()
        .filter(|project| wks_native::launch::absolute_directory(&project.path))
        .take(3)
        .collect::<Vec<_>>();
        let model = if self.model_choice == "__custom" {
            self.model.read(cx).value().to_string()
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
        let context = self
            .context_window
            .map(|tokens| format!(" · {}K context", tokens / 1000))
            .unwrap_or_default();
        let summary = format!(
            "{}{} · {}",
            if model.is_empty() {
                "Custom model"
            } else {
                &model
            },
            context,
            self.permission.label()
        );

        let content = div()
            .max_w(px(640.))
            .mx_auto()
            .py_2()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(overline(
                        if self.extras.resume.is_some() {
                            "CONTINUE A CONVERSATION"
                        } else {
                            "NEW AGENT"
                        },
                        p,
                    ))
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
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::BOLD)
                            .child(if self.extras.resume.is_some() {
                                "Pick up where you left off"
                            } else {
                                "What are we working on?"
                            }),
                    )
                    .child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(
                        "Pick your agent, choose a workspace, and give it a starting point.",
                    )),
            )
            .child(
                div().flex().gap_3().children(
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
                            .p_3()
                            .rounded(px(p.panel_radius))
                            .bg(rgb(if selected { p.selected } else { p.surface }))
                            .border_color(rgb(if selected { p.accent } else { p.border }))
                            .flex()
                            .flex_col()
                            .gap_3()
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
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Icon::new(icon).size(px(22.)).text_color(rgb(
                                            if selected { p.accent } else { p.muted },
                                        )),
                                    )
                                    .when(selected, |d| {
                                        d.child(
                                            Icon::new(IconName::Check)
                                                .size(px(14.))
                                                .text_color(rgb(p.accent)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(p.muted))
                                            .child(subtitle),
                                    ),
                            )
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Where should it work?"),
                            )
                            .when(self.extras.local_paths, |d| {
                                d.child(
                                    self.quiet_button(
                                        "browse-project",
                                        "Choose folder",
                                        IconName::FolderOpen,
                                        !busy,
                                    )
                                    .when(!busy, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_folder(false, window, cx)
                                        }))
                                    }),
                                )
                            }),
                    )
                    .when(!suggestions.is_empty(), |d| {
                        d.child(div().flex().flex_wrap().gap_2().children(
                            suggestions.into_iter().enumerate().map(|(index, project)| {
                                let title = project.title().to_owned();
                                let path = project.path;
                                let tooltip = path.clone();
                                self.button("workspace-pick", title, !busy)
                                    .id(("workspace-pick", index))
                                    .debug_selector(move || format!("launch-workspace-{index}"))
                                    .max_w_full()
                                    .truncate()
                                    .rounded_full()
                                    .bg(rgb(if cwd == path { p.selected } else { p.surface }))
                                    .tooltip(move |window, cx| {
                                        gpui_component::tooltip::Tooltip::new(tooltip.clone())
                                            .build(window, cx)
                                    })
                                    .when(!busy, |d| {
                                        d.on_click(cx.listener(move |this, _, window, cx| {
                                            this.project.update(cx, |input, cx| {
                                                input.set_value(path.clone(), window, cx)
                                            });
                                            this.load_models(false, cx);
                                            cx.notify();
                                        }))
                                    })
                            }),
                        ))
                    })
                    .child(Input::new(&self.project).disabled(busy))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child("Choose an existing folder on the connected hub."),
                    ),
            )
            .child(self.render_model_options(busy, cx))
            .child(
                div()
                    .p_4()
                    .rounded(px(p.panel_radius))
                    .bg(rgb(p.surface))
                    .border_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Bot)
                                    .size(px(16.))
                                    .text_color(rgb(p.accent)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Give it a starting point"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child("Optional"),
                            ),
                    )
                    .child(
                        Input::new(&self.prompt)
                            .appearance(false)
                            .h(px(90.))
                            .disabled(busy),
                    ),
            )
            .when(self.extras.resume.is_some(), |d| {
                d.child(
                    div().text_size(px(12.)).text_color(rgb(p.accent)).child(
                        "Your previous conversation will be resumed with the choices below.",
                    ),
                )
            })
            .child(
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
                                    if self.launch_details_open {
                                        "Hide options"
                                    } else {
                                        "Customize"
                                    },
                                    IconName::Settings2,
                                    !busy,
                                )
                                .debug_selector(|| "launch-customize".into())
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        if this.launch_details_open {
                                            this.project
                                                .update(cx, |input, cx| input.focus(window, cx));
                                        }
                                        this.launch_details_open = !this.launch_details_open;
                                        cx.notify();
                                    }))
                                }),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(if self.permission == Permission::FullAccess {
                                        p.warning
                                    } else {
                                        p.muted
                                    }))
                                    .child(summary),
                            ),
                    )
                    .when(self.launch_details_open, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "launch-details".into())
                                .p_4()
                                .rounded(px(p.panel_radius))
                                .bg(rgb(p.surface))
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(self.render_access_options(busy, cx))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(p.muted))
                                        .child("Name this session (optional)"),
                                )
                                .child(Input::new(&self.label).disabled(busy))
                                .child(
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
                        )
                    }),
            )
            .when(!self.spawn_error.is_empty(), |d| {
                d.child(
                    div()
                        .text_color(rgb(p.warning))
                        .child(self.spawn_error.clone()),
                )
            })
            .when(!self.view.connected, |d| {
                d.child(
                    div()
                        .text_color(rgb(p.warning))
                        .child("Waiting for the hub connection…"),
                )
            });
        let footer = div()
            .w_full()
            .max_w(px(640.))
            .mx_auto()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                if cfg!(target_os = "macos") {
                    "⌘ Enter to start"
                } else {
                    "Ctrl Enter to start"
                },
            ))
            .child(
                self.primary_button(
                    "create-session",
                    if busy {
                        "Starting…"
                    } else if self.extras.resume.is_some() {
                        "Continue conversation"
                    } else {
                        "Start working"
                    },
                    can_create,
                )
                .debug_selector(|| "launch-start".into())
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::ArrowRight).size(px(14.)))
                .when(can_create, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.create(cx)))
                }),
            );
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
                    .p_5()
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
                    .child(footer),
            )
    }

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
        self.permission = self.settings.default_access(provider);
        self.catalog_models.clear();
        self.model
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reset_model_picker(window, cx);
        self.sync_models(window, cx);
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

    pub(super) fn render_model_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
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
            .child(div().debug_selector(|| "launch-model-picker".into()).child(Select::new(&self.model_picker).disabled(busy).search_placeholder("Find a model…")))
            .when(!loading, |d| d.child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                if self.catalog_models.is_empty() { "No model catalog loaded yet. Provider default and Custom model are still available.".to_owned() }
                else { format!("{} models available", self.catalog_models.len()) })))
            .when(self.model_choice == "__custom", |d| d.child(Input::new(&self.model).disabled(busy)))
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(if self.provider == "claude" {
                "Claude family aliases follow your installed CLI. Custom model accepts an exact ID."
            } else { "Models are queried from Codex on the connected hub." }))
            .when(current && catalog.error.is_some(), |d| d.child(div().text_size(px(12.)).text_color(rgb(p.warning))
                .child(catalog.error.clone().unwrap_or_default())))
            .when(current && !loading && catalog.error.is_none() && catalog.models.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(rgb(p.warning))
                .child("No models were returned. Refresh models, use Provider default, or enter a custom model ID.")))
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
    }

    pub(super) fn render_access_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child("Access mode")
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
