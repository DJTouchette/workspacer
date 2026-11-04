use super::*;
use gpui_component::{scroll::ScrollableElement, select::SelectItem};
use wks_native::hosts::{self, HostChoice};

#[derive(Clone)]
pub(super) struct PickerItem {
    id: String,
    label: String,
    /// Test selector prefix: `model-option` or `effort-option`.
    kind: &'static str,
}
impl PickerItem {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn new(id: impl Into<String>, label: impl Into<String>, kind: &'static str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
        }
    }
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

    // r3-pages
    /// The model picker's rows. Under v2 they match the composer's model
    /// popover (R3.5): human names, the curated models the catalog does not
    /// list, and "Custom id…" last.
    pub(super) fn picker_models(&self) -> Vec<PickerItem> {
        let mut items = model_items(&self.catalog_models);
        if !self.design().is_v2() {
            return items;
        }
        let custom = items.pop();
        for item in items.iter_mut().skip(1) {
            let default = self
                .catalog_models
                .iter()
                .any(|m| m.id == item.id && m.is_default);
            let name = wks_native::format::model_name(&item.id);
            item.label = if default {
                format!("{name} (default)")
            } else {
                name
            };
        }
        items.extend(
            super::model_popover::listed_models(self.provider, &self.catalog_models)
                .into_iter()
                .skip(self.catalog_models.len())
                .map(|m| PickerItem {
                    label: wks_native::format::model_name(&m.id),
                    id: m.id,
                    kind: "model-option",
                }),
        );
        items.extend(custom.map(|mut c| {
            c.label = "Custom id…".into();
            c
        }));
        items
    }

    /// A new launch/provider gets a fresh query and scroll state, not the old
    /// dropdown's filtered list. Replacing the subscription also releases it.
    pub(super) fn reset_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.picker_models();
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
        let p = self.palette();
        let busy = self.spawn_pending || self.view.creating || self.extras.queue_sent;
        let can_create = self.view.connected && !busy && !self.demo;
        let resume = self.extras.resume.is_some();
        // Two columns only where both stay readable beside the sidebar.
        let wide = window.viewport_size().width >= px(1100.);
        // r2-pages: v2 is task-first (R2.6).
        if let Some(p) = self.v2_palette() {
            return self.render_new_session_v2(window, busy, can_create, wide, p, cx);
        }
        // Short windows give the form, not the heading, the height.
        let short = window.viewport_size().height < px(620.);
        let v2 = self.v2_palette();
        let card = || {
            if let Some(p) = v2 {
                return primitives::card(&p).gap(px(p.tokens.space.lg));
            }
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
        let close = self
            .icon_button(
                "cancel-create",
                "Back to conversation",
                IconName::Close,
                !busy,
            )
            .when(!busy, |d| {
                d.on_click(cx.listener(|this, _, window, cx| {
                    this.show_screen(Screen::Conversation, window, cx)
                }))
            });
        let header = if let Some(p) = v2 {
            let close = primitives::icon_button_with_shortcut(
                "cancel-create",
                IconName::Close,
                "Back to the conversation",
                Some("Esc"),
                ButtonVariant::Tertiary,
                !busy,
                &p,
            )
            .when(!busy, |d| {
                d.on_click(cx.listener(|this, _, window, cx| {
                    this.show_screen(Screen::Conversation, window, cx)
                }))
            });
            primitives::page_header(
                None,
                if resume {
                    "Continue a session"
                } else {
                    "New session"
                },
                Some(if resume {
                    "Pick up where you left off, on the same agent and folder.".into()
                } else {
                    "Choose where it runs, the project and the agent, then describe the task."
                        .into()
                }),
                vec![close.into_any_element()],
                &p,
            )
            .debug_selector(|| "launch-title".into())
        } else {
            self.page_header(
                None,
                Some(if resume {
                    "CONTINUE A CONVERSATION"
                } else {
                    "NEW AGENT"
                }),
                if resume {
                    "Pick up where you left off"
                } else {
                    "Start an agent"
                },
                None,
                Some(close.into_any_element()),
                short,
            )
            .debug_selector(|| "launch-title".into())
        };
        let label = |text: &'static str| match v2 {
            Some(p) => card_title(text, &p),
            None => projects::section_label(text, p),
        };
        let agent = card()
            .child(label("Agent"))
            .child(self.render_provider_choice(busy, cx))
            .child(self.render_model_select(busy, true, cx));
        let task = card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label("Task"))
                    .child(
                        div()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(if resume {
                                "Optional · sent after the conversation resumes"
                            } else {
                                "Optional · you can also start empty"
                            }),
                    ),
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
        let gutter = if short { 12. } else { 20. };
        let content = div()
            .debug_selector(|| "launch-content".into())
            .max_w(px(self.launch_width(wide)))
            .mx_auto()
            .py_2()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .children(
                if v2.is_some() {
                    self.render_host_picker_v2(busy, cx)
                } else {
                    self.render_host_picker(busy, cx)
                }
                .map(|picker| card().child(picker)),
            )
            .child(card().child(match self.launch_peer() {
                Some(peer) => self.render_peer_folders(&peer, busy, cx),
                None => self.render_project_section(busy, cx),
            }))
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
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.accent))
                        .child(Icon::new(IconName::Info).size(px(12.)))
                        .child(
                            "Your previous conversation will be resumed with the choices above.",
                        ),
                )
            });
        div()
            .id("new-session-form")
            .relative()
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
                    .p(px(gutter))
                    .when(chrome::custom_caption(), |d| {
                        d.pt(px(chrome::PAGE_CAPTION_INSET))
                    })
                    .child(content),
            )
            .child(
                // The form's own gutters, so the footer panel lines up with
                // the cards above it and floats on the page like the composer.
                div()
                    .flex_shrink_0()
                    .px(px(gutter))
                    .pt_2()
                    .pb(px(gutter))
                    // A flex row gives the footer a definite width, so a long
                    // error wraps instead of widening it.
                    .flex()
                    .justify_center()
                    .child(self.render_launch_footer(busy, can_create, wide, short, cx)),
            )
            .children(chrome::page_drag_strip())
    }

    fn render_provider_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        div().flex().gap_2().children(
            [
                ("claude", "Claude", "Claude Code"),
                ("codex", "Codex", "OpenAI Codex"),
            ]
            .into_iter()
            .map(|(provider, name, subtitle)| {
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
                    .child(chrome::provider_mark(provider, 28., p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(chrome::scale::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(chrome::scale::CAPTION))
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
        let p = self.palette();
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
        let p = self.palette();
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
                                .text_size(px(chrome::scale::CAPTION))
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
        short: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
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
        let queued = self.queued_launch();
        let project = match self.launch_peer() {
            Some(peer) => (!self.extras.launch_host_cwd.is_empty()).then(|| {
                format!(
                    "{} on {peer}",
                    wks_native::projects::basename(&self.extras.launch_host_cwd)
                )
            }),
            None => (!self.projects.cwd.is_empty()).then(|| {
                let title = self
                    .known_project(&self.projects.cwd)
                    .map(|p| p.title().to_owned())
                    .unwrap_or_else(|| {
                        wks_native::projects::basename(&self.projects.cwd).to_owned()
                    });
                match &queued {
                    Some(HostChoice::Best) => format!("{title} · best fit"),
                    Some(_) => format!("{title} · queued here"),
                    None => title,
                }
            }),
        };
        // Short validation fits the footer line; a hub/launch error is shown
        // whole above it, since its wording says what may have happened.
        let uncertain = wks_native::launch::uncertain_outcome(&self.spawn_error);
        let detailed = uncertain || self.spawn_error.chars().count() > 72;
        let (icon, color, status): (Option<IconName>, u32, String) = if !self.spawn_error.is_empty()
        {
            (
                Some(IconName::TriangleAlert),
                p.warning,
                if uncertain {
                    "Outcome unknown".into()
                } else if detailed {
                    "Couldn't start the agent".into()
                } else {
                    self.spawn_error.clone()
                },
            )
        } else if self.extras.queue_sent {
            (
                None,
                p.muted,
                format!(
                    "Submitting {provider} to the queue · {}…",
                    project.clone().unwrap_or_default()
                ),
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
        } else if self.design().is_v2() && project.is_some() {
            // r2-pages: the chips in the task box already say it.
            (None, p.muted, String::new())
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
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(color))
                    .when_some(icon, |d, icon| {
                        d.child(Icon::new(icon).size(px(13.)).flex_shrink_0())
                    })
                    .when(busy, |d| d.child(work::spinner(13.)))
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
                            if self.extras.queue_sent {
                                "Submitting…"
                            } else if busy {
                                "Starting…"
                            } else if queued.is_some() && self.design().is_v2() {
                                "Queue session"
                            } else if queued.is_some() {
                                "Queue agent"
                            } else if uncertain {
                                // Still allowed: the hub keeps no retry block, and
                                // the user may have checked. The label says so.
                                "Start anyway"
                            } else if self.extras.resume.is_some() {
                                "Continue"
                            } else if self.design().is_v2() {
                                "Start session"
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
        // r2-pages: v2 ends the form with a plain row under the task.
        if let Some(v2) = self.v2_palette() {
            return div()
                .debug_selector(|| "launch-footer".into())
                .w_full()
                .flex()
                .flex_col()
                .gap(px(v2.tokens.space.md))
                .when(detailed, |d| {
                    d.child(self.render_launch_error(uncertain, short))
                })
                .child(row);
        }
        // The shared card surface, raised like the chat composer: the one
        // action dock on this page, not a separate band at the window edge.
        chrome::card(p)
            .debug_selector(|| "launch-footer".into())
            .shadow(chrome::floating_shadow(p))
            .flex_1()
            .min_w_0()
            .max_w(px(self.launch_width(wide)))
            .px(px(if short { 12. } else { 16. }))
            .py(px(if short { 8. } else { 12. }))
            .flex()
            .flex_col()
            .gap_2()
            .when(detailed, |d| {
                d.child(self.render_launch_error(uncertain, short))
            })
            .child(row)
    }

    /// A launch failure in full. The recovery step is its own unclipped line;
    /// the hub's wording follows in a bounded, scrollable region, so a long
    /// error never pushes the Start button off a small window and never hides
    /// what it says about the outcome.
    fn render_launch_error(&self, uncertain: bool, short: bool) -> Div {
        let p = self.palette();
        // Only an outcome-unknown failure gets a recovery step; for anything
        // else the hub's own wording is the explanation, unembellished.
        let (headline, guidance) = if uncertain {
            (
                "The agent may already be running",
                Some("Check the session list before starting again, or it may run twice."),
            )
        } else {
            ("Couldn't start the agent", None)
        };
        div()
            .debug_selector(|| "launch-error".into())
            .w_full()
            .p_2()
            .rounded(px(p.control_radius))
            .border_1()
            .border_color(rgb(p.warning))
            .flex()
            .items_start()
            .gap_2()
            .text_size(px(chrome::scale::META))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(rgb(p.warning)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| "launch-error-headline".into())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(p.warning))
                            .child(headline),
                    )
                    .when_some(guidance, |d, guidance| {
                        d.child(
                            div()
                                .debug_selector(|| "launch-error-guidance".into())
                                .text_color(rgb(p.text))
                                .child(guidance),
                        )
                    })
                    .child(
                        div()
                            .id("launch-error-details")
                            .debug_selector(|| "launch-error-details".into())
                            .relative()
                            .w_full()
                            .mt_1()
                            .pr_2()
                            .max_h(px(if short { 52. } else { 96. }))
                            .overflow_y_scroll()
                            .track_scroll(&self.launch_error_scroll)
                            .vertical_scrollbar(&self.launch_error_scroll)
                            .child(
                                div()
                                    .debug_selector(|| "launch-error-text".into())
                                    .w_full()
                                    .text_color(rgb(p.muted))
                                    .child(self.spawn_error.clone()),
                            ),
                    ),
            )
    }

    pub(super) fn catalog_key(&self, _cx: &App) -> CatalogKey {
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
        if self.demo
            || (!self.new_session && !matches!(self.screen, Screen::Model | Screen::Handoff))
        {
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
        let items = self.picker_models();
        self.model_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(items), window, cx);
            picker.set_selected_value(&self.model_choice, window, cx);
        });
        if self.screen == Screen::Model {
            self.select_exact_model(window, cx);
        }
        self.reconcile_effort(window, cx);
    }

    /// Change model opens on the session's exact model ID; once the catalog
    /// lists that exact ID, show it as the selected row instead of a custom
    /// entry. A near match (an alias, another context variant) stays custom.
    pub(super) fn select_exact_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_choice != "__custom" {
            return;
        }
        let custom = self.model.read(cx).value().trim().to_owned();
        if custom.is_empty() || !self.catalog_models.iter().any(|m| m.id == custom) {
            return;
        }
        self.model_choice = custom;
        self.model_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&self.model_choice, window, cx)
        });
    }

    /// Model choice with the catalog's live state; the exact ID is entered
    /// here when Custom is chosen, so it is never hidden behind the fold.
    /// `with_effort` places the effort menu beside the model (New Agent and
    /// the live Model screen).
    pub(super) fn render_model_select(
        &self,
        busy: bool,
        with_effort: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
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
        if let Some(v2) = self.v2_palette() {
            return self.render_model_select_v2(busy, with_effort, status, status_color, &v2, cx);
        }
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
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(status_color))
                    .child(status),
            )
    }

    /// Context windows the chosen model offers; Default only for a custom ID,
    /// whose windows the client cannot know.
    pub(super) fn render_context_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let windows = self
            .catalog_models
            .iter()
            .find(|m| m.id == self.model_choice)
            .map(|m| m.windows.clone())
            .unwrap_or_else(|| {
                // A custom ID's windows are unknown: offer the one chosen and,
                // on Change model, the session's current one, so picking
                // Default never removes the other choice.
                let session = (self.screen == Screen::Model && !self.new_session)
                    .then(|| self.selected_session().and_then(|s| s.context_window))
                    .flatten();
                let mut windows: Vec<u64> =
                    self.context_window.into_iter().chain(session).collect();
                windows.dedup();
                windows
            });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(projects::section_label("Context window", p))
            .when(windows.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(
                            "The model's default. Models with a choice of windows list them here.",
                        ),
                )
            })
            .when(!windows.is_empty(), |d| {
                let options = (self.model_choice == "__custom")
                    .then(|| (None, "Default".to_owned()))
                    .into_iter()
                    .chain(windows.into_iter().map(|t| (Some(t), context_label(t))))
                    .collect();
                d.child(div().flex().child(self.segmented_enabled(
                    "context-window",
                    options,
                    self.context_window,
                    !busy,
                    |this, tokens, _, cx| {
                        this.context_window = tokens;
                        cx.notify();
                    },
                    cx,
                )))
            })
    }

    pub(super) fn render_model_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.render_model_select(busy, true, cx))
            .child(self.render_context_choice(busy, cx))
    }

    pub(super) fn render_access_options(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(projects::section_label("Access", p))
            // One segmented control, like the Model page's live Access: the
            // chosen mode is the filled segment (Full access in the warning
            // tone), not blue text on a row of look-alike buttons.
            .child(
                div().flex().child(
                    div()
                        .debug_selector(|| "launch-access".into())
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .p(px(3.))
                        .rounded(px(18.))
                        .bg(rgb(p.base))
                        .border_1()
                        .border_color(rgb(p.border))
                        .children(Permission::choices(self.provider).iter().copied().map(
                            |permission| {
                                let active = self.permission == permission;
                                chrome::interactive_control(div().id(permission.label()), p, !busy)
                                    .debug_selector(move || {
                                        format!("access-{}", permission.label())
                                    })
                                    .h(px(28.))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .text_size(px(chrome::scale::META))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(match (active, busy) {
                                        (_, true) => p.disabled,
                                        (true, _) if permission == Permission::FullAccess => {
                                            p.warning
                                        }
                                        (true, _) => p.text,
                                        (false, false) => p.muted,
                                    }))
                                    .when(active, |d| d.bg(rgb(p.selected)))
                                    .child(permission.label())
                                    .when(!busy, |d| {
                                        d.hover(move |s| s.text_color(rgb(p.text))).on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.permission = permission;
                                                cx.notify();
                                            }),
                                        )
                                    })
                            },
                        )),
                ),
            )
            .child(
                div()
                    .text_size(px(chrome::scale::META))
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

impl Workspace {
    /// The queue route this launch takes, or `None` for today's direct
    /// `agents.spawn` on this machine. Always direct while the host picker
    /// is hidden (an older hub, or a resume).
    pub(super) fn queued_launch(&self) -> Option<HostChoice> {
        if self.extras.resume.is_some() {
            return None;
        }
        self.launch_targets()?;
        match &self.extras.launch_host {
            HostChoice::Local if !self.extras.launch_queue => None,
            other => Some(other.clone()),
        }
    }

    /// The peer the form launches on, when one is chosen.
    pub(super) fn launch_peer(&self) -> Option<String> {
        match self.queued_launch()? {
            HostChoice::Peer(peer) => Some(peer),
            _ => None,
        }
    }

    /// The local folder the latest `queue.targets` read asked about, while
    /// it runs too (so a choice does not ask twice).
    fn targets_asked_cwd(&self) -> Option<String> {
        match &self.view.requests.get("queue-targets")?.request {
            wks_native::features::Request::QueueTargets { cwd, .. } => cwd.clone(),
            _ => None,
        }
    }

    /// The shown targets were resolved for this form's local project: their
    /// `resolvedCwd` and `suggestions` are about it.
    pub(super) fn targets_for_local(&self) -> bool {
        !self.projects.cwd.is_empty()
            && self
                .view
                .requests
                .get("queue-targets")
                .is_some_and(|s| !s.loading && s.error.is_none())
            && self.targets_asked_cwd().as_deref() == Some(self.projects.cwd.as_str())
    }

    /// Re-read `queue.targets` for this provider and the local project, so
    /// each peer says where that project is there.
    pub(super) fn refresh_launch_targets(&mut self, cx: &mut Context<Self>) {
        let cwd = (!self.projects.cwd.is_empty()).then(|| self.projects.cwd.clone());
        self.command(
            Command::Request(wks_native::features::Request::QueueTargets {
                provider: Some(self.provider.to_owned()),
                cwd,
            }),
            cx,
        );
    }

    /// Select a peer's folder for the launch: the local project's resolved
    /// path there when known, else keep a still-offered choice.
    fn choose_launch_host(&mut self, choice: HostChoice, cx: &mut Context<Self>) {
        if let HostChoice::Peer(peer) = &choice {
            let target = self.launch_targets().and_then(|t| t.find(peer).cloned());
            let resolved = target
                .as_ref()
                .filter(|_| self.targets_for_local())
                .and_then(|t| t.resolved_cwd.clone());
            let paths = target.map(|t| t.paths()).unwrap_or_default();
            if let Some(resolved) = resolved {
                self.extras.launch_host_cwd = resolved;
            } else if !paths.contains(&self.extras.launch_host_cwd) {
                self.extras.launch_host_cwd = paths.first().cloned().unwrap_or_default();
            }
            self.extras.launch_host_cwd_picked = false;
        }
        let ask = !matches!(choice, HostChoice::Local)
            && !self.projects.cwd.is_empty()
            && self.targets_asked_cwd().as_deref() != Some(self.projects.cwd.as_str());
        self.extras.launch_host = choice;
        self.spawn_error.clear();
        if ask {
            self.refresh_launch_targets(cx);
        }
        cx.notify();
    }

    /// "Link & use": save the suggested folder as the local project's link
    /// on `peer`; `sync_hosts` selects it once the hub saved it.
    fn link_and_use(&mut self, peer: String, path: String, cx: &mut Context<Self>) {
        let local_path = self.projects.cwd.clone();
        if local_path.is_empty() || self.extras.link_use.is_some() {
            return;
        }
        let after = self
            .view
            .requests
            .get("link-action")
            .map_or(0, |s| s.number);
        self.extras.link_use = Some((after, peer.clone(), path.clone()));
        self.spawn_error.clear();
        self.command(
            Command::Request(wks_native::features::Request::LinkAction(
                wks_native::features::LinkAction::Set {
                    local_path,
                    peer,
                    peer_path: path,
                },
            )),
            cx,
        );
        cx.notify();
    }

    /// Run on: This machine (with "Queue if busy"), each peer, Best fit.
    /// Hidden for a hub that has no `queue.targets`.
    fn render_host_picker(&self, busy: bool, cx: &mut Context<Self>) -> Option<Div> {
        if self.extras.resume.is_some() {
            return None;
        }
        let targets = self.launch_targets()?;
        let p = self.palette();
        let options = hosts::host_options(&targets);
        let chips = options.iter().enumerate().map(|(ix, option)| {
            let selected = self.extras.launch_host == option.choice;
            let enabled = option.enabled && !busy;
            let choice = option.choice.clone();
            chrome::interactive_control(div().id(("launch-host", ix)), p, enabled)
                .debug_selector(move || format!("launch-host-{ix}"))
                .px_3()
                .py(px(6.))
                .rounded(px(p.control_radius))
                .bg(rgb(if selected { p.selected } else { p.chat }))
                .border_color(rgb(if selected { p.accent } else { p.border }))
                .flex()
                .flex_col()
                .min_w(px(120.))
                .when(!option.enabled, |d| d.opacity(0.55))
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgb(p.selected)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_launch_host(choice.clone(), cx)
                        }))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(chrome::scale::META))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(option.label.clone())
                        .when(selected, |d| {
                            d.child(
                                Icon::new(IconName::Check)
                                    .size(px(12.))
                                    .text_color(rgb(p.accent)),
                            )
                        }),
                )
                .when(!option.detail.is_empty(), |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(option.detail.clone()),
                    )
                })
        });
        let reasons: Vec<String> = options
            .iter()
            .filter(|o| !o.enabled && !o.reasons.is_empty())
            .map(|o| format!("{}: {}", o.label, o.reasons.join("; ")))
            .collect();
        let local = self.extras.launch_host == HostChoice::Local;
        let queue = self.extras.launch_queue;
        Some(
            div()
                .debug_selector(|| "launch-hosts".into())
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(projects::section_label("Run on", p))
                        .child(
                            self.quiet_button(
                                "launch-hosts-refresh",
                                "Refresh",
                                IconName::Redo,
                                !busy && self.view.connected,
                            )
                            .when(!busy && self.view.connected, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_launch_targets(cx)),
                                )
                            }),
                        ),
                )
                .child(div().flex().flex_wrap().gap_2().children(chips))
                .children(reasons.into_iter().enumerate().map(|(ix, line)| {
                    div()
                        .debug_selector(move || format!("launch-host-reason-{ix}"))
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(line)
                }))
                .when(local, |d| {
                    d.child(
                        chrome::interactive_control(div().id("launch-queue"), p, !busy)
                            .debug_selector(|| "launch-queue".into())
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(chrome::scale::META))
                            .when(!busy, |d| {
                                d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                                    this.extras.launch_queue = !this.extras.launch_queue;
                                    cx.notify();
                                }))
                            })
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(rgb(if queue { p.accent } else { p.border }))
                                    .bg(rgb(if queue { p.accent } else { p.chat }))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(queue, |d| {
                                        d.child(
                                            Icon::new(IconName::Check)
                                                .size(px(10.))
                                                .text_color(rgb(p.on_primary)),
                                        )
                                    }),
                            )
                            .child("Queue if busy")
                            .child(
                                div()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child("waits for a free slot and an open gate"),
                            ),
                    )
                }),
        )
    }

    /// A peer's project folders (its target's `cwds`): the local folder
    /// picker does not apply to another machine. The local project's own
    /// folder there is preselected and says how it was matched; a folder
    /// that may be it is offered as "Link & use".
    fn render_peer_folders(&self, peer: &str, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let target = self.launch_targets().and_then(|t| t.find(peer).cloned());
        let for_local = self.targets_for_local();
        let dirs = target.as_ref().map(|t| t.cwds.clone()).unwrap_or_default();
        let resolved = target
            .as_ref()
            .filter(|_| for_local)
            .and_then(|t| t.resolved_cwd.clone());
        let how = target
            .as_ref()
            .filter(|_| for_local)
            .and_then(|t| t.match_label().map(str::to_owned));
        let suggestions = target
            .as_ref()
            .filter(|_| for_local && resolved.is_none())
            .map(|t| t.suggestions.clone())
            .unwrap_or_default();
        let local_name = wks_native::projects::basename(&self.projects.cwd).to_owned();
        let linking = self.extras.link_use.is_some();
        let mut list = div().flex().flex_col().gap_1();
        for (ix, dir) in dirs.iter().enumerate() {
            let selected = self.extras.launch_host_cwd == dir.path;
            let matched = resolved.as_deref() == Some(dir.path.as_str());
            let chosen = dir.path.clone();
            list = list.child(
                chrome::interactive_control(div().id(("launch-host-cwd", ix)), p, !busy)
                    .debug_selector(move || format!("launch-host-cwd-{ix}"))
                    .px_3()
                    .py(px(6.))
                    .rounded(px(p.control_radius))
                    .bg(rgb(if selected { p.selected } else { p.chat }))
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!busy, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(rgb(p.selected)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.extras.launch_host_cwd = chosen.clone();
                                this.extras.launch_host_cwd_picked = true;
                                this.spawn_error.clear();
                                cx.notify();
                            }))
                    })
                    .child(Icon::new(IconName::Folder).size(px(13.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(chrome::scale::META))
                            .child(dir.path.clone()),
                    )
                    .when_some(dir.remote.clone(), |d, remote| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .max_w(px(220.))
                                .truncate()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child(remote),
                        )
                    })
                    .when_some(how.clone().filter(|_| matched), |d, how| {
                        d.child(
                            super::hosts::pill(how, p.success, p)
                                .debug_selector(move || format!("launch-host-cwd-match-{ix}")),
                        )
                    })
                    .when(selected, |d| {
                        d.child(
                            Icon::new(IconName::Check)
                                .size(px(13.))
                                .text_color(rgb(p.accent)),
                        )
                    }),
            );
        }
        let caption = |text: String| {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(text)
        };
        let mut hints = div().flex().flex_col().gap_1();
        if for_local {
            hints = match (&resolved, &how) {
                (Some(path), Some(how)) => hints.child(
                    caption(format!("{local_name} is {path} here ({how})."))
                        .debug_selector(|| "launch-host-match".into()),
                ),
                (Some(path), None) => hints.child(
                    caption(format!("{local_name} is {path} here."))
                        .debug_selector(|| "launch-host-match".into()),
                ),
                (None, _) if suggestions.is_empty() => hints.child(
                    caption(format!(
                        "No folder on {peer} matches {local_name}. Pick one, or link it in Hosts → Projects."
                    ))
                    .debug_selector(|| "launch-host-nomatch".into()),
                ),
                (None, _) => hints,
            };
        }
        for (ix, suggestion) in suggestions.iter().enumerate() {
            let enabled = !busy && !linking && self.view.connected;
            let (peer, path) = (peer.to_owned(), suggestion.path.clone());
            hints = hints.child(
                div()
                    .debug_selector(move || format!("launch-link-suggestion-{ix}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .text_size(px(chrome::scale::META))
                    .child(div().flex_shrink_0().child("Suggested:"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .child(suggestion.path.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(suggestion.label().to_owned()),
                    )
                    .child(
                        self.quiet_button(
                            SharedString::from(format!("launch-link-use-{ix}")),
                            "Link & use",
                            IconName::Check,
                            enabled,
                        )
                        .debug_selector(move || format!("launch-link-use-{ix}"))
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.link_and_use(peer.clone(), path.clone(), cx)
                            }))
                        }),
                    ),
            );
        }
        div()
            .debug_selector(|| "launch-peer-folders".into())
            .flex()
            .flex_col()
            .gap_3()
            .child(projects::section_label("Project on this peer", p))
            .child(hints)
            .child(if dirs.is_empty() {
                div()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child("This peer offers no project folders.")
            } else {
                list
            })
    }

    /// Send the form to the queue instead of starting an agent here; the
    /// confirmation and the Hosts view follow in `sync_hosts`.
    pub(super) fn submit_queued(
        &mut self,
        host: HostChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = if matches!(host, HostChoice::Peer(_)) {
            self.extras.launch_host_cwd.clone()
        } else {
            self.adopt_typed_project(window, cx);
            if self.projects.cwd.is_empty() {
                self.spawn_error = "Choose a project folder first.".into();
                self.open_project_picker(window, cx);
                cx.notify();
                return;
            }
            self.projects.cwd.clone()
        };
        let model = if self.model_choice == "__custom" {
            let custom = self.model.read(cx).value().trim().to_owned();
            if custom.is_empty() {
                self.spawn_error = "Enter a custom model or choose Provider default.".into();
                cx.notify();
                return;
            }
            custom
        } else {
            self.model_choice.clone()
        };
        let submit = hosts::SpawnSubmit {
            host,
            provider: self.provider.to_owned(),
            cwd,
            prompt: self.prompt.read(cx).value().to_string(),
            model,
            effort: self.effort.clone(),
            permission_mode: self
                .permission
                .wire(self.provider)
                .map(str::to_owned)
                .unwrap_or_default(),
            label: self.label.read(cx).value().to_string(),
        };
        match submit.params() {
            Ok(params) => {
                self.spawn_error.clear();
                self.extras.queue_sent = true;
                self.command(
                    Command::Request(wks_native::features::Request::QueueSubmit(params)),
                    cx,
                );
            }
            Err(error) => self.spawn_error = error,
        }
        cx.notify();
    }
}

// area-pages: the New session form under design v2. Machines are equal-width
// cards with their status in the shared vocabulary (a closed gate is muted,
// not hidden: queued work waits for it), and Best fit is a separate
// Auto-place switch rather than a fifth "machine".
use super::primitives::{self, ButtonVariant, StatusBadge, StatusKind};
use gpui_component::{Disableable as _, switch::Switch};

/// One machine card's facts, from `queue.targets`.
struct HostCard {
    choice: HostChoice,
    name: String,
    status: StatusBadge,
    /// Selectable: connected and dispatching.
    enabled: bool,
    /// A closed gate or an offline peer: drawn quietly.
    muted: bool,
}

fn host_cards(targets: &hosts::Targets) -> Vec<HostCard> {
    let status = |t: &hosts::Target| -> (StatusBadge, bool) {
        if !t.local && !t.connected {
            // r2-pages: R2.1 offline is a fault, as on Machines.
            return (StatusBadge::new(StatusKind::Failed).label("Offline"), true);
        }
        if !t.local && !t.eligible {
            let why = t.reasons.first().cloned().unwrap_or_else(|| {
                if t.dispatch {
                    "Unavailable".into()
                } else {
                    "Not taking work".into()
                }
            });
            return (StatusBadge::new(StatusKind::Idle).label(why), true);
        }
        match &t.resources {
            Some(r) => match &r.gate {
                Some(g) if !g.open => (
                    StatusBadge::new(StatusKind::Paused)
                        .label(super::hosts::paused_label(&g.reasons)),
                    true,
                ),
                _ => {
                    let label = match (r.free_agents(), r.agent_slots) {
                        (Some(free), Some(_)) => format!("{free} free"),
                        _ => "Accepting work".to_owned(),
                    };
                    (StatusBadge::new(StatusKind::Idle).label(label), false)
                }
            },
            None => (
                StatusBadge::new(StatusKind::Idle).label("No sample yet"),
                false,
            ),
        }
    };
    let mut cards = Vec::new();
    if let Some(local) = targets.targets.iter().find(|t| t.local) {
        let (badge, muted) = status(local);
        cards.push(HostCard {
            choice: HostChoice::Local,
            name: "This machine".into(),
            status: badge,
            enabled: true,
            muted,
        });
    } else {
        cards.push(HostCard {
            choice: HostChoice::Local,
            name: "This machine".into(),
            status: StatusBadge::new(StatusKind::Idle).label("Accepting work"),
            enabled: true,
            muted: false,
        });
    }
    for peer in targets.peers() {
        let (badge, muted) = status(peer);
        cards.push(HostCard {
            choice: HostChoice::Peer(peer.host.clone()),
            name: peer.host.clone(),
            status: badge,
            enabled: peer.eligible,
            muted,
        });
    }
    cards
}

impl Workspace {
    /// "Run on": Auto-place, then one equal-width card per machine.
    fn render_host_picker_v2(&self, busy: bool, cx: &mut Context<Self>) -> Option<Div> {
        if self.extras.resume.is_some() {
            return None;
        }
        let targets = self.launch_targets()?;
        let p = self.palette();
        let t = p.tokens;
        let auto = self.extras.launch_host == HostChoice::Best;
        let any = targets.targets.iter().any(|t| t.eligible);
        let best_now = match (&targets.best, any) {
            (_, false) => "No machine can take it now".to_owned(),
            (Some(best), true) if best == hosts::LOCAL => "Now: this machine".to_owned(),
            (Some(best), true) => format!("Now: {best}"),
            (None, true) => "Chosen when it starts".to_owned(),
        };
        let cards = host_cards(&targets);
        let card_views = cards.into_iter().enumerate().map(|(ix, card)| {
            let selected = !auto && self.extras.launch_host == card.choice;
            let enabled = card.enabled && !busy && !auto;
            let choice = card.choice.clone();
            chrome::interactive_control(div().id(("launch-host", ix)), p, enabled)
                .debug_selector(move || format!("launch-host-{ix}"))
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .px(px(t.space.lg))
                .py(px(t.space.md + t.space.xxs))
                .rounded(px(t.radius.control))
                .bg(rgb(if selected { p.selected } else { p.surface }))
                .border_color(rgb(if selected {
                    p.accent
                } else {
                    p.roles.border_strong
                }))
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .when(enabled && !selected, |d| {
                    d.hover(move |s| s.bg(rgb(p.roles.hover)))
                })
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_launch_host(choice.clone(), cx)
                    }))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .min_w_0()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(if card.muted || auto {
                            p.roles.text_tertiary
                        } else {
                            p.text
                        }))
                        .child(div().min_w_0().truncate().child(card.name.clone()))
                        .child(div().flex_1())
                        .when(selected, |d| {
                            d.child(
                                Icon::new(IconName::Check)
                                    .size(px(t.icon.sm))
                                    .text_color(rgb(p.accent)),
                            )
                        }),
                )
                .child(card.status.render(&p))
        });
        let local = self.extras.launch_host == HostChoice::Local;
        let queue = self.extras.launch_queue;
        let can_refresh = !busy && self.view.connected;
        Some(
            div()
                .debug_selector(|| "launch-hosts".into())
                .flex()
                .flex_col()
                .gap(px(t.space.lg))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.md))
                        .child(card_title("Run on", &p))
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("launch-auto-place")
                                .debug_selector(|| "launch-auto-place".into())
                                .flex()
                                .items_center()
                                .gap(px(t.space.md))
                                .tooltip(primitives::tooltip_view(
                                    format!("Starts it on whichever machine has room. {best_now}"),
                                    None,
                                    &p,
                                ))
                                .child(
                                    div()
                                        .text_size(px(t.text.body))
                                        .text_color(rgb(p.roles.text_secondary))
                                        .child("Pick a machine for me"),
                                )
                                .child(
                                    Switch::new("launch-auto-place-switch")
                                        .checked(auto)
                                        .disabled(busy || (!any && !auto))
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.choose_launch_host(
                                                if *checked {
                                                    HostChoice::Best
                                                } else {
                                                    HostChoice::Local
                                                },
                                                cx,
                                            )
                                        })),
                                ),
                        )
                        .child(
                            primitives::icon_button(
                                "launch-hosts-refresh",
                                IconName::Redo,
                                "Refresh machines",
                                ButtonVariant::Tertiary,
                                can_refresh,
                                &p,
                            )
                            .debug_selector(|| "launch-hosts-refresh".into())
                            .when(can_refresh, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_launch_targets(cx)),
                                )
                            }),
                        ),
                )
                .child(div().flex().gap(px(t.space.md)).children(card_views))
                .when(auto, |d| {
                    d.child(
                        div()
                            .debug_selector(|| "launch-auto-place-note".into())
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(format!("Starts on whichever machine has room. {best_now}.")),
                    )
                })
                .when(local, |d| {
                    d.child(
                        div()
                            .debug_selector(|| "launch-queue".into())
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .child(
                                Switch::new("launch-queue-switch")
                                    .checked(queue)
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.extras.launch_queue = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(t.text.body))
                                    .text_color(rgb(p.text))
                                    .child("Queue if busy"),
                            )
                            .child(
                                div()
                                    .text_size(px(t.text.meta))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child("Waits for a free agent instead of starting now"),
                            ),
                    )
                }),
        )
    }
}

// r2-pages: R2.6 task-first New session. The task box leads; machine,
// project, agent and model follow as chips inside it (the Codex pattern),
// and a chip opens its choices in one card below.

/// Which chip's choices are open under the task box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchPanel {
    Machine,
    Project,
    Agent,
}

impl Workspace {
    fn render_new_session_v2(
        &self,
        window: &Window,
        busy: bool,
        can_create: bool,
        wide: bool,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = p.tokens;
        let short = window.viewport_size().height < px(620.);
        let resume = self.extras.resume.is_some();
        let close =
            primitives::icon_button_with_shortcut(
                "cancel-create",
                IconName::Close,
                "Back to the conversation",
                Some("Esc"),
                ButtonVariant::Tertiary,
                !busy,
                &p,
            )
            .when(!busy, |d| {
                d.on_click(cx.listener(|this, _, window, cx| {
                    this.show_screen(Screen::Conversation, window, cx)
                }))
            });
        let header = primitives::page_header(
            None,
            if resume {
                "Continue a session"
            } else {
                "New session"
            },
            Some(if resume {
                "Pick up where you left off, on the same agent and folder.".into()
            } else {
                "Say what you want done. Where it runs and which agent works on it are below the task."
                    .into()
            }),
            vec![close.into_any_element()],
            &p,
        )
        .debug_selector(|| "launch-title".into());
        // No project yet: its picker is the one thing to do first.
        let panel = if self.projects.cwd.is_empty() && self.launch_peer().is_none() {
            Some(LaunchPanel::Project)
        } else {
            self.extras.launch_panel
        };
        let task = div()
            .debug_selector(|| "launch-task".into())
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(t.radius.composer))
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .shadow(primitives::elevation(&p, t.elevation.raised))
            .child(
                div()
                    .debug_selector(|| "launch-prompt".into())
                    .px(px(t.space.xl))
                    .pt(px(t.space.lg))
                    .min_h(px(t.layout.control_height * 3.))
                    .flex()
                    .flex_col()
                    .text_size(px(t.text.ui))
                    .child(
                        Input::new(&self.prompt)
                            .appearance(false)
                            .flex_1()
                            .min_h(px(t.layout.control_height * 3.))
                            .disabled(busy),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "launch-chips".into())
                    .px(px(t.space.md))
                    .py(px(t.space.md))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(t.space.xs))
                    .children(self.launch_chips_v2(panel, busy, &p, cx))
                    .child(div().flex_1())
                    .child(
                        div()
                            .px(px(t.space.sm))
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(if resume {
                                "Optional · sent after it resumes"
                            } else {
                                "Optional · you can start empty"
                            }),
                    ),
            );
        let panel_card = panel.and_then(|panel| {
            let body = match panel {
                LaunchPanel::Machine => self.render_host_picker_v2(busy, cx)?,
                LaunchPanel::Project => match self.launch_peer() {
                    Some(peer) => self.render_peer_folders(&peer, busy, cx),
                    None => self.render_project_section(busy, cx),
                },
                LaunchPanel::Agent => div()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.lg))
                    .child(card_title("Agent", &p))
                    .child(self.render_provider_choice(busy, cx))
                    .child(self.render_model_select(busy, true, cx)),
            };
            Some(
                primitives::card(&p)
                    .debug_selector(|| "launch-panel".into())
                    .gap(px(t.space.lg))
                    .child(body),
            )
        });
        let gutter = if short {
            t.space.lg
        } else {
            t.layout.page_gutter
        };
        let content = div()
            .debug_selector(|| "launch-content".into())
            .w_full()
            .max_w(px(self.launch_width(wide)))
            .mx_auto()
            .pt(px(if short { 0. } else { t.layout.page_top }))
            .flex()
            .flex_col()
            .gap(px(t.space.xl))
            .child(header)
            .child(task)
            .children(panel_card)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(t.space.lg))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_launch_disclosure(busy, cx)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(t.layout.page_width / 2.))
                            .child(self.render_launch_footer(busy, can_create, wide, short, cx)),
                    ),
            )
            .when(resume, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.sm))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(Icon::new(IconName::Info).size(px(t.icon.sm)))
                        .child(
                            "Your previous conversation will be resumed with the choices above.",
                        ),
                )
            });
        div()
            .id("new-session-form")
            .relative()
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
                    .px(px(gutter))
                    .pb(px(gutter))
                    .when(chrome::custom_caption(), |d| {
                        d.pt(px(chrome::PAGE_CAPTION_INSET))
                    })
                    .child(content),
            )
            .children(chrome::page_drag_strip())
    }

    /// The chips inside the task box: machine (when the hub has a queue),
    /// project, agent and model. Each opens its choices below; the open one
    /// reads as selected.
    fn launch_chips_v2(
        &self,
        open: Option<LaunchPanel>,
        busy: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let t = p.tokens;
        let chip = |id: &'static str,
                    panel: LaunchPanel,
                    leading: Option<gpui::AnyElement>,
                    label: String,
                    tooltip: &'static str| {
            let selected = open == Some(panel);
            primitives::toolbar_chip(id, leading, label, true, !busy, p)
                .debug_selector(move || id.into())
                .when(selected, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
                .tooltip(primitives::tooltip_view(tooltip, None, p))
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.launch_panel =
                            (this.extras.launch_panel != Some(panel)).then_some(panel);
                        cx.notify();
                    }))
                })
                .into_any_element()
        };
        let icon = |name: IconName| {
            Some(
                Icon::new(name)
                    .size(px(t.icon.sm))
                    .text_color(rgb(p.roles.text_tertiary))
                    .into_any_element(),
            )
        };
        let mut chips = Vec::new();
        if self.extras.resume.is_none() && self.launch_targets().is_some() {
            let machine = match &self.extras.launch_host {
                HostChoice::Best => "Pick a machine for me".to_owned(),
                HostChoice::Local => "This machine".to_owned(),
                HostChoice::Peer(peer) => peer.clone(),
            };
            chips.push(chip(
                "launch-chip-machine",
                LaunchPanel::Machine,
                icon(IconName::LayoutDashboard),
                machine,
                "Where it runs",
            ));
        }
        let cwd = match self.launch_peer() {
            Some(_) => self.extras.launch_host_cwd.clone(),
            None => self.projects.cwd.clone(),
        };
        let project = if cwd.is_empty() {
            "Choose a project".to_owned()
        } else {
            self.known_project(&cwd)
                .map(|p| p.title().to_owned())
                .unwrap_or_else(|| wks_native::projects::basename(&cwd).to_owned())
        };
        chips.push(chip(
            "launch-chip-project",
            LaunchPanel::Project,
            icon(IconName::Folder),
            project,
            "The project's folder",
        ));
        let provider = if self.provider == "codex" {
            "Codex"
        } else {
            "Claude"
        };
        chips.push(chip(
            "launch-chip-agent",
            LaunchPanel::Agent,
            Some(chrome::provider_glyph(self.provider, t.icon.sm, *p)),
            provider.to_owned(),
            "The agent that works on it",
        ));
        let model = match self.model_choice.as_str() {
            "__custom" => {
                let typed = self.model.read(cx).value().trim().to_owned();
                if typed.is_empty() {
                    "Custom model".to_owned()
                } else {
                    wks_native::model::model_display_name(&typed)
                }
            }
            "" => self
                .catalog_models
                .iter()
                .find(|m| m.is_default)
                .map(|m| wks_native::model::model_display_name(&m.id))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Default model".to_owned()),
            id => wks_native::model::model_display_name(id),
        };
        let model = if self.effort.is_empty() {
            model
        } else {
            let mut effort = self.effort.clone();
            if let Some(first) = effort.get(0..1) {
                effort = first.to_uppercase() + &effort[1..];
            }
            format!("{model} · {effort}")
        };
        chips.push(chip(
            "launch-chip-model",
            LaunchPanel::Agent,
            None,
            model,
            "Model and effort",
        ));
        chips
    }
}

/// A launch card's title, sentence case (v2).
fn card_title(text: &'static str, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .text_size(px(t.text.subheading))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.text))
        .child(text)
}

impl Workspace {
    /// The form's and footer's width: v1's two widths, v2's page width.
    fn launch_width(&self, wide: bool) -> f32 {
        match self.v2_palette() {
            Some(p) => p.tokens.layout.page_width,
            None if wide => 980.,
            None => 720.,
        }
    }

    /// Model and effort under v2: the labels above, Refresh inside the
    /// model select (models come from the agent's CLI and can change).
    fn render_model_select_v2(
        &self,
        busy: bool,
        with_effort: bool,
        status: String,
        status_color: u32,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = p.tokens;
        let catalog = &self.view.catalog;
        let loading = catalog.key == self.catalog_key(cx) && catalog.loading;
        let can_reload = !busy && !loading && self.view.connected;
        let field_label = |text: &'static str| {
            div()
                .text_size(px(t.text.meta))
                .font_weight(FontWeight(t.weight.medium))
                .text_color(rgb(p.roles.text_secondary))
                .child(text)
        };
        let refresh = primitives::icon_button(
            "reload-models",
            IconName::Redo,
            if loading {
                "Loading models…"
            } else {
                "Refresh the model list"
            },
            ButtonVariant::Tertiary,
            can_reload,
            p,
        )
        .debug_selector(|| "reload-models".into())
        .when(can_reload, |d| {
            d.on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.load_models(true, cx)
            }))
        });
        let model = div()
            .flex_1()
            .min_w(px(t.layout.status_column * 2.))
            .flex()
            .flex_col()
            .gap(px(t.space.sm))
            .child(field_label("Model"))
            .child(
                div()
                    .debug_selector(|| "launch-model-picker".into())
                    .relative()
                    .child(
                        Select::new(&self.model_picker)
                            .disabled(busy)
                            .search_placeholder("Find a model…"),
                    )
                    // Inside the select, left of its chevron.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(t.layout.icon_button))
                            .flex()
                            .items_center()
                            .child(refresh),
                    ),
            );
        let effort = with_effort.then(|| {
            div()
                .w(px(t.layout.status_column * 1.5))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(t.space.sm))
                .child(field_label("Effort"))
                .child(
                    div()
                        .debug_selector(|| "launch-effort-picker".into())
                        .child(Select::new(&self.effort_picker).disabled(busy)),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(t.space.lg))
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
                    .text_size(px(t.text.meta))
                    .text_color(rgb(if status_color == self.palette().muted {
                        p.roles.text_tertiary
                    } else {
                        status_color
                    }))
                    .child(status),
            )
    }
}
