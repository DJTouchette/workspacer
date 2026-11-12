use super::*;
use gpui_component::{Sizable, scroll::ScrollableElement, select::SelectItem};
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
            .unwrap_or_else(|| "Default effort".into()),
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
            // New Agent keeps the exact-ID field in the agent's settings.
            if self.new_session {
                self.launch_details_open = true;
                if !self.projects.cwd.is_empty() {
                    self.projects.picker_open = false;
                }
            }
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

    /// The New Agent screen is a conversation that has not started: the
    /// title capsule names the project, the page shows where the agent will
    /// work, and the composer at the bottom takes the first message. The
    /// agent's own settings open from the composer only when asked for.
    pub(super) fn render_new_session(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let busy = self.spawn_pending || self.view.creating || self.extras.queue_sent;
        let can_create = self.view.connected && !busy && !self.demo;
        let resume = self.extras.resume.is_some();
        // Short windows give the page, not its margins, the height.
        let short = window.viewport_size().height < px(620.);
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
        let peer = self.launch_peer();
        let details = self.launch_details_open;
        let choosing = !details
            && peer.is_none()
            && (self.projects.picker_open || self.projects.cwd.is_empty());
        let panel = || chrome::card(p).w_full().max_w(px(PANEL_WIDTH)).p_4();
        let title = self.render_launch_title(peer.as_deref(), busy, cx);
        let composer = self.render_launch_composer(busy, can_create, short, cx);
        let stage: Vec<gpui::AnyElement> = if details {
            vec![self.render_launch_details(busy, cx).into_any_element()]
        } else {
            let identity = (!choosing).then(|| {
                self.render_launch_identity(peer.as_deref(), busy, cx)
                    .into_any_element()
            });
            // Changing the project is one question at a time; with none
            // chosen yet, another machine's folders are still an answer.
            let hosts = (!choosing || self.projects.cwd.is_empty())
                .then(|| self.render_host_picker(busy, cx))
                .flatten()
                .map(IntoElement::into_any_element);
            let chooser = choosing.then(|| {
                panel()
                    .child(self.render_project_picker(busy, cx))
                    .into_any_element()
            });
            let folders = peer.as_deref().map(|peer| {
                panel()
                    .child(self.render_peer_folders(peer, busy, cx))
                    .into_any_element()
            });
            let notice = self
                .render_project_notice(cx)
                .map(IntoElement::into_any_element);
            identity
                .into_iter()
                .chain(hosts)
                .chain(chooser)
                .chain(folders)
                .chain(notice)
                .collect()
        };
        let gutter = if short { 12. } else { 20. };
        div()
            .id("new-session-form")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                // The capsule sits where a conversation's title does.
                div()
                    .flex_shrink_0()
                    .px(px(gutter))
                    .pt(px(if short { 8. } else { 10. }))
                    .when(chrome::custom_caption(), |d| {
                        d.pt(px(chrome::PAGE_CAPTION_INSET))
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(control::HEIGHT_SM)).flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_center()
                            .child(title),
                    )
                    .child(close),
            )
            .child(
                div()
                    .id("launch-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        // Centred like an empty conversation (auto margins,
                        // so a chooser taller than the page still scrolls
                        // from its top).
                        div()
                            .debug_selector(|| "launch-content".into())
                            .w_full()
                            .flex_shrink_0()
                            .my_auto()
                            .p(px(gutter))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_4()
                            .children(stage)
                            .when(resume, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_size(px(chrome::scale::META))
                                        .text_color(rgb(p.accent))
                                        .child(Icon::new(IconName::Info).size(px(icon_size::SM)))
                                        .child(
                                            "Your previous conversation will be resumed with these choices.",
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(gutter))
                    .pb(px(if short { 8. } else { 12. }))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(CHAT_WIDTH))
                            .mx_auto()
                            .child(composer),
                    ),
            )
            .children(chrome::page_drag_strip())
    }

    /// The title capsule of a conversation that has no title yet: the
    /// project (which reopens the chooser), then what this is.
    fn render_launch_title(&self, peer: Option<&str>, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let project = match peer {
            Some(peer) => {
                let folder = &self.extras.launch_host_cwd;
                div()
                    .h(px(control::HEIGHT_SM))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(Icon::new(IconName::Folder).size(px(icon_size::MD)))
                    .child(if folder.is_empty() {
                        format!("Pick a folder on {peer}")
                    } else {
                        format!("{} on {peer}", wks_native::projects::basename(folder))
                    })
                    .into_any_element()
            }
            None => self.render_project_summary(busy, cx).into_any_element(),
        };
        div()
            .debug_selector(|| "launch-title".into())
            .max_w_full()
            .min_w_0()
            .h(px(40.))
            .pl(px(6.))
            .pr_4()
            .flex()
            .items_center()
            .gap_2()
            .rounded_full()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .shadow(chrome::floating_shadow(p))
            .child(project)
            .child(
                div()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.disabled))
                    .child("/"),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(chrome::scale::BODY))
                    .text_color(rgb(p.muted))
                    .child(if self.extras.resume.is_some() {
                        "Continue"
                    } else {
                        "New agent"
                    }),
            )
    }

    /// Where the agent will work, in the middle of the page like an empty
    /// conversation's greeting: the project's mark and name, and what the
    /// hub reports about its folder.
    fn render_launch_identity(
        &self,
        peer: Option<&str>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let cwd = self.projects.cwd.clone();
        let project = self.known_project(&cwd);
        let (mark, name) = match peer {
            Some(peer) => {
                let folder = &self.extras.launch_host_cwd;
                (
                    div()
                        .size(px(HERO_MARK))
                        .rounded_full()
                        .bg(chrome::tonal(p, alpha::REST))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Folder).size(px(icon_size::HERO))),
                    if folder.is_empty() {
                        peer.to_owned()
                    } else {
                        format!("{} on {peer}", wks_native::projects::basename(folder))
                    },
                )
            }
            None => (
                self.project_mark(project.as_ref(), &cwd, HERO_MARK),
                project
                    .as_ref()
                    .map(|p| p.title().to_owned())
                    .unwrap_or_else(|| wks_native::projects::basename(&cwd).to_owned()),
            ),
        };
        div()
            .debug_selector(|| "launch-hero".into())
            .max_w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(mark)
            .child(
                div()
                    .mt_1()
                    .max_w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(chrome::scale::TITLE))
                            .font_weight(FontWeight::BOLD)
                            .child(name),
                    )
                    .children(
                        peer.is_none()
                            .then(|| self.render_project_pin(busy, cx))
                            .flatten(),
                    ),
            )
            .children(
                peer.is_none()
                    .then(|| self.render_project_status())
                    .flatten(),
            )
    }

    /// Claude or Codex, as one two-way switch.
    fn render_provider_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        div()
            .flex_shrink_0()
            .flex()
            .gap_1()
            .p(px(3.))
            .rounded_full()
            .bg(rgb(p.base))
            .border_1()
            .border_color(rgb(p.border))
            .children([("claude", "Claude"), ("codex", "Codex")].into_iter().map(
                |(provider, name)| {
                    let selected = self.provider == provider;
                    chrome::interactive_control(div().id(provider), p, !busy)
                        .debug_selector(move || format!("launch-provider-{provider}"))
                        .h(px(control::HEIGHT_SM))
                        .pl(px(8.))
                        .pr(px(10.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(chrome::scale::META))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(match (busy, selected) {
                            (true, _) => p.disabled,
                            (false, true) => p.text,
                            (false, false) => p.muted,
                        }))
                        .when(selected, |d| d.bg(rgb(p.selected)))
                        .when(!busy, |d| {
                            d.hover(move |s| s.text_color(rgb(p.text)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose_provider(provider, window, cx);
                                    this.load_models(false, cx);
                                    cx.notify();
                                }))
                        })
                        .child(chrome::provider_glyph(provider, icon_size::LG, p))
                        .child(name)
                },
            ))
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

    /// The agent's settings, opened from the composer: who it is, how hard
    /// it thinks, what it may do, and what to call it.
    fn render_launch_details(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let (status, status_color, loading, notable) = self.model_catalog_status(cx);
        let can_reload = !busy && !loading && self.view.connected;
        chrome::card(p)
            .debug_selector(|| "launch-details".into())
            .w_full()
            .max_w(px(PANEL_WIDTH))
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(self.render_provider_choice(busy, cx))
                    .child(
                        self.icon_button("launch-details-close", "Done", IconName::Check, !busy)
                            .when(!busy, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    window.focus(&this.focus);
                                    this.launch_details_open = false;
                                    cx.notify();
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
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(projects::section_label("Effort", p))
                            .child(
                                div()
                                    .debug_selector(|| "launch-effort-picker".into())
                                    .w(px(170.))
                                    .flex_shrink_0()
                                    .child(Select::new(&self.effort_picker).disabled(busy)),
                            )
                            .child(
                                self.icon_button(
                                    "reload-models",
                                    "Reload the model list",
                                    IconName::Redo,
                                    can_reload,
                                )
                                .when(can_reload, |d| {
                                    d.on_click(
                                        cx.listener(|this, _, _, cx| this.load_models(true, cx)),
                                    )
                                }),
                            ),
                    )
                    .when(self.model_choice == "__custom", |d| {
                        d.child(
                            div()
                                .debug_selector(|| "launch-custom-model".into())
                                .child(Input::new(&self.model).disabled(busy)),
                        )
                    })
                    .when(notable, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "launch-model-status".into())
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(status_color))
                                .child(status),
                        )
                    }),
            )
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
                div().ml(px(-8.)).flex().child(
                    self.quiet_button("launch-setup", "Agent setup", IconName::Settings2, !busy)
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.open_feature(Screen::Setup, window, cx)
                            }))
                        }),
                ),
            )
    }

    /// The catalog's state in words, its tone, and whether it is loading.
    fn model_catalog_status(&self, cx: &App) -> (String, u32, bool, bool) {
        let p = self.palette();
        let catalog = &self.view.catalog;
        let current = catalog.key == self.catalog_key(cx);
        let loading = current && catalog.loading;
        // The last flag: worth a line of its own on a compact form (a plain
        // count of what the menu already lists is not).
        if loading {
            ("Loading models…".to_owned(), p.muted, true, true)
        } else if current && catalog.error.is_some() {
            (
                catalog.error.clone().unwrap_or_default(),
                p.warning,
                false,
                true,
            )
        } else if current && catalog.models.is_empty() {
            (
                "No models were returned. Use Provider default or enter a custom model ID."
                    .to_owned(),
                p.warning,
                false,
                true,
            )
        } else if !current && self.provider != "claude" && self.projects.cwd.is_empty() {
            (
                "Codex lists models for the chosen project.".to_owned(),
                p.muted,
                false,
                true,
            )
        } else if self.catalog_models.is_empty() {
            (
                "Provider default and Custom model are available.".to_owned(),
                p.muted,
                false,
                true,
            )
        } else if self.provider == "claude" {
            (
                format!(
                    "{} model families from your installed Claude CLI",
                    self.catalog_models.len()
                ),
                p.muted,
                false,
                false,
            )
        } else {
            (
                format!("{} models from Codex on the hub", self.catalog_models.len()),
                p.muted,
                false,
                false,
            )
        }
    }

    /// What the draft has to say before Start: a short failure, progress, or
    /// what is still missing. Nothing when it is simply ready.
    fn launch_status(&self, busy: bool, detailed: bool) -> Option<(Option<IconName>, u32, String)> {
        let p = self.palette();
        let provider = if self.provider == "codex" {
            "Codex"
        } else {
            "Claude"
        };
        let project = match self.launch_peer() {
            Some(peer) => (!self.extras.launch_host_cwd.is_empty()).then(|| {
                format!(
                    "{} on {peer}",
                    wks_native::projects::basename(&self.extras.launch_host_cwd)
                )
            }),
            None => (!self.projects.cwd.is_empty()).then(|| {
                self.known_project(&self.projects.cwd)
                    .map(|p| p.title().to_owned())
                    .unwrap_or_else(|| {
                        wks_native::projects::basename(&self.projects.cwd).to_owned()
                    })
            }),
        };
        if !self.spawn_error.is_empty() {
            // A long or outcome-unknown failure is shown whole in its card.
            (!detailed).then(|| {
                (
                    Some(IconName::TriangleAlert),
                    p.warning,
                    self.spawn_error.clone(),
                )
            })
        } else if self.extras.queue_sent {
            Some((
                None,
                p.muted,
                format!(
                    "Submitting {provider} to the queue · {}…",
                    project.unwrap_or_default()
                ),
            ))
        } else if busy {
            Some((
                None,
                p.muted,
                format!("Starting {provider} in {}…", project.unwrap_or_default()),
            ))
        } else if !self.view.connected {
            Some((
                Some(IconName::TriangleAlert),
                p.warning,
                "Waiting for the hub connection…".into(),
            ))
        } else if project.is_none() {
            Some((None, p.muted, "Choose a project to start".into()))
        } else {
            None
        }
    }

    /// The chat composer, before there is a chat: the first message, the
    /// agent that will read it (one chip, which opens its settings), and
    /// Start where Send will be.
    fn render_launch_composer(
        &self,
        busy: bool,
        can_create: bool,
        short: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let open = self.launch_details_open;
        let uncertain = wks_native::launch::uncertain_outcome(&self.spawn_error);
        let detailed = uncertain || self.spawn_error.chars().count() > 72;
        let queued = self.queued_launch().is_some();
        let status = self.launch_status(busy, detailed);
        let provider = if self.provider == "codex" {
            "Codex"
        } else {
            "Claude"
        };
        let effort =
            (!self.effort.is_empty()).then(|| wks_native::launch::effort_label(&self.effort));
        let start = self
            .pill_button(
                "create-session",
                if self.extras.queue_sent {
                    "Submitting…"
                } else if busy {
                    "Starting…"
                } else if queued {
                    "Queue agent"
                } else if uncertain {
                    // Still allowed: the hub keeps no retry block, and
                    // the user may have checked. The label says so.
                    "Start anyway"
                } else if self.extras.resume.is_some() {
                    "Continue"
                } else {
                    "Start agent"
                },
                IconName::ArrowUp,
                can_create,
                true,
            )
            .debug_selector(|| "launch-start".into())
            .when(can_create, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.create(window, cx)))
            });
        let agent = chrome::interactive_control(div().id("launch-customize"), p, !busy)
            .debug_selector(|| "launch-customize".into())
            .h(px(control::HEIGHT_SM))
            .min_w_0()
            .pl(px(8.))
            .pr(px(8.))
            .rounded_full()
            .bg(rgb(p.selected))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(chrome::scale::META))
            .font_weight(FontWeight::MEDIUM)
            .when(!busy, |d| {
                d.hover(|s| s.bg(rgb(p.border)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.launch_details_open {
                            window.focus(&this.focus);
                        } else if !this.projects.cwd.is_empty() {
                            this.projects.picker_open = false;
                        }
                        this.launch_details_open = !this.launch_details_open;
                        cx.notify();
                    }))
            })
            .child(chrome::provider_glyph(self.provider, icon_size::LG, p))
            .child(provider)
            .when_some(effort, |d, effort| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(p.muted))
                        .child(format!("· {effort}")),
                )
            })
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronUp
                })
                .size(px(icon_size::SM))
                .text_color(rgb(p.muted)),
            );
        let composer = div()
            .debug_selector(|| "launch-task".into())
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .rounded(px(p.composer_radius))
            .shadow(chrome::floating_shadow(p))
            .px(px(if short { 10. } else { 14. }))
            .pt(px(if short { 8. } else { 14. }))
            .pb(px(if short { 8. } else { 10. }))
            .flex()
            .flex_col()
            .gap_2()
            .when(detailed, |d| {
                d.child(self.render_launch_error(uncertain, short))
            })
            .child(
                div()
                    .debug_selector(|| "launch-prompt".into())
                    .child(Input::new(&self.prompt).appearance(false).disabled(busy)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(agent)
                            .child(
                                // The model is the choice made most often,
                                // so its menu is here, not behind the chip.
                                div()
                                    .debug_selector(|| "launch-model-picker".into())
                                    .w(px(MODEL_MENU))
                                    .max_w_full()
                                    .rounded_full()
                                    .bg(rgb(p.selected))
                                    .child(
                                        Select::new(&self.model_picker)
                                            .small()
                                            .appearance(false)
                                            .disabled(busy)
                                            .search_placeholder("Find a model…"),
                                    ),
                            )
                            .children(self.launch_option_chips(cx).into_iter().enumerate().map(
                                |(ix, (text, color))| {
                                    super::hosts::pill(text, color, p)
                                        .debug_selector(move || format!("launch-chip-{ix}"))
                                },
                            )),
                    )
                    .child(div().flex_shrink_0().child(start)),
            );
        // Under it, the line a conversation uses for what is going on and
        // how to send.
        let hint = div()
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .text_size(px(chrome::scale::OVERLINE))
            .text_color(rgb(p.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when_some(status, |d, (icon, color, text)| {
                        d.child(
                            div()
                                .id("launch-status")
                                .debug_selector(|| "launch-status".into())
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(color))
                                .when_some(icon, |d, icon| {
                                    d.child(Icon::new(icon).size(px(icon_size::SM)).flex_shrink_0())
                                })
                                .when(busy, |d| d.child(work::spinner(11.)))
                                .child(div().min_w_0().truncate().child(text)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(keycap(
                        if cfg!(target_os = "macos") {
                            "⌘ Enter"
                        } else {
                            "Ctrl Enter"
                        },
                        p,
                    ))
                    .child("start")
                    .child(div().w_2())
                    .child(keycap("Esc", p))
                    .child("back"),
            );
        div().flex().flex_col().gap_2().child(composer).child(hint)
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
                    .size(px(icon_size::MD))
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
                            // Its own wheel: the page behind it stays put.
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
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
        let items = model_items(&self.catalog_models);
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
        let (status, status_color, loading, _) = self.model_catalog_status(cx);
        let can_reload = !busy && !loading && self.view.connected;
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
                        .rounded(px(radius::SEGMENTED_GROUP))
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

/// A chooser's or the agent settings' measure in the middle of the page.
const PANEL_WIDTH: f32 = 640.;
/// The project's mark above its name.
const HERO_MARK: f32 = 56.;
/// The composer's model menu: room for "Provider default · Sonnet 4.6".
const MODEL_MENU: f32 = 250.;

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
        let pills = options.iter().enumerate().map(|(ix, option)| {
            let selected = self.extras.launch_host == option.choice;
            let enabled = option.enabled && !busy;
            let choice = option.choice.clone();
            chrome::interactive_control(div().id(("launch-host", ix)), p, enabled)
                .debug_selector(move || format!("launch-host-{ix}"))
                .h(px(control::HEIGHT_SM))
                .px(px(10.))
                .rounded_full()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(chrome::scale::META))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if !enabled {
                    p.disabled
                } else if selected {
                    p.text
                } else {
                    p.muted
                }))
                .when(selected, |d| d.bg(rgb(p.selected)))
                .when(enabled, |d| {
                    d.hover(move |s| s.text_color(rgb(p.text)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_launch_host(choice.clone(), cx)
                        }))
                })
                .child(option.label.clone())
                .when(selected, |d| {
                    d.child(
                        Icon::new(IconName::Check)
                            .size(px(icon_size::SM))
                            .text_color(rgb(p.accent)),
                    )
                })
        });
        // The chosen machine's state in words; the others say why not.
        let detail = options
            .iter()
            .find(|o| o.choice == self.extras.launch_host && !o.detail.is_empty())
            .map(|o| format!("{} · {}", o.label, o.detail));
        let reasons: Vec<String> = options
            .iter()
            .filter(|o| !o.enabled && !o.reasons.is_empty())
            .map(|o| format!("{}: {}", o.label, o.reasons.join("; ")))
            .collect();
        let local = self.extras.launch_host == HostChoice::Local;
        let queue = self.extras.launch_queue;
        let caption = |text: String| {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(text)
        };
        Some(
            div()
                .debug_selector(|| "launch-hosts".into())
                .max_w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(p.muted))
                                .child("Run on"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .p(px(3.))
                                .rounded(px(radius::SEGMENTED_GROUP))
                                .bg(rgb(p.base))
                                .border_1()
                                .border_color(rgb(p.border))
                                .children(pills),
                        )
                        .child(
                            self.icon_button(
                                "launch-hosts-refresh",
                                "Check the hosts again",
                                IconName::Redo,
                                !busy && self.view.connected,
                            )
                            .when(!busy && self.view.connected, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_launch_targets(cx)),
                                )
                            }),
                        )
                        .when(local, |d| {
                            d.child(
                                chrome::interactive_control(div().id("launch-queue"), p, !busy)
                                    .debug_selector(|| "launch-queue".into())
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(chrome::scale::META))
                                    .text_color(rgb(p.muted))
                                    .when(!busy, |d| {
                                        d.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| {
                                                this.extras.launch_queue =
                                                    !this.extras.launch_queue;
                                                cx.notify();
                                            },
                                        ))
                                    })
                                    .child(
                                        div()
                                            .size(px(14.))
                                            .rounded(px(radius::MARK))
                                            .border_1()
                                            .border_color(rgb(if queue {
                                                p.accent
                                            } else {
                                                p.border
                                            }))
                                            .bg(rgb(if queue { p.accent } else { p.chat }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(queue, |d| {
                                                d.child(
                                                    Icon::new(IconName::Check)
                                                        .size(px(icon_size::XS))
                                                        .text_color(rgb(p.on_primary)),
                                                )
                                            }),
                                    )
                                    .child("Queue if busy"),
                            )
                        }),
                )
                .children(detail.map(|text| {
                    caption(if local && queue {
                        format!("{text} · waits for a free slot and an open gate")
                    } else {
                        text
                    })
                    .debug_selector(|| "launch-host-detail".into())
                }))
                .children(reasons.into_iter().enumerate().map(|(ix, line)| {
                    caption(line).debug_selector(move || format!("launch-host-reason-{ix}"))
                })),
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
                    .child(Icon::new(IconName::Folder).size(px(icon_size::MD)))
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
                                .size(px(icon_size::MD))
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
            .child(caption(format!("Folders on {peer}")))
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
