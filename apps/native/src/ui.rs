mod bus_commands;
mod chrome;
mod features;
mod launch;
mod markdown;
mod navigation;
mod scroll;
mod sidebar;
mod states;
mod tools;
mod transcript;
mod typography;
use gpui::{
    Animation, AnimationExt, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, KeyBinding, ListAlignment, ListOffset, ListScrollEvent, ListState, Render,
    SharedString, Stateful, Task, Window, actions, canvas, div, list, prelude::*, px, rgb,
    uniform_list,
};
use gpui_component::{
    Icon, IconName,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    text::TextView,
};
use launch::{PickerItem, model_items};
use navigation::Screen;
use std::{collections::HashMap, sync::Arc};
use wks_native::appearance::{Appearance, Palette, preference_path};
use wks_native::controller::{Action, Command, Controller, NewSession, View};
use wks_native::launch::{CatalogKey, ModelChoice, Permission};
use wks_native::model::Session;
use wks_native::navigation::{Project, Provider, Settings, projects};
use wks_native::timing::{self, TurnClock};

const CHAT_WIDTH: f32 = 900.;

pub fn configure_theme(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    typography::register_fonts(cx);
    use gpui_component::{Theme, ThemeMode};
    Theme::change(
        if appearance == Appearance::Light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        window,
        cx,
    );
    let p = appearance.palette();
    let theme = Theme::global_mut(cx);
    theme.colors.background = rgb(p.base).into();
    theme.colors.foreground = rgb(p.text).into();
    theme.colors.border = rgb(p.border).into();
    theme.colors.input = rgb(p.border).into();
    theme.colors.primary = rgb(p.primary).into();
    theme.colors.primary_hover = rgb(p.primary_hover).into();
    theme.colors.secondary_hover = rgb(p.selected).into();
    theme.colors.ring = rgb(p.accent).into();
    theme.colors.primary_foreground = rgb(p.on_primary).into();
    theme.colors.caret = rgb(p.accent).into();
    theme.colors.link = rgb(p.accent).into();
    theme.colors.muted = rgb(p.surface).into();
    theme.colors.muted_foreground = rgb(p.muted).into();
    theme.colors.switch = rgb(p.selected).into();
    theme.colors.switch_thumb = rgb(if appearance == Appearance::Light {
        p.surface
    } else {
        p.text
    })
    .into();
    theme.colors.popover = rgb(p.surface).into();
    theme.colors.popover_foreground = rgb(p.text).into();
    theme.colors.selection = rgb(p.selected).into();
    theme.font_size = px(15.);
    theme.font_family = "Inter".into();
    theme.mono_font_family = "JetBrains Mono".into();
    theme.mono_font_size = px(13.);
    theme.radius = px(p.control_radius);
}

fn mono_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "JetBrains Mono"
    }
}

// Same brace-and-cursor geometry as desktop components/Brand.tsx.
fn brand_mark(size: f32, p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(1.))
        .font_family(mono_font())
        .text_size(px(size))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(p.accent))
        .child("{")
        .child(
            div()
                .w(px(size * 0.22))
                .h(px(size * 0.72))
                .rounded(px(1.))
                .bg(rgb(p.accent)),
        )
        .child("}")
}

// Electron BrandSpinner: an 0.8s eased journey in each direction.
fn brand_spinner(size: f32, p: Palette, id: impl Into<gpui::ElementId>) -> Div {
    let bar_width = (size * 0.18).max(2.);
    let track_width = (size * 0.85).round();
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px((size * 0.06).max(1.)))
        .font_family(mono_font())
        .text_size(px(size))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(p.accent))
        .child("{")
        .child(
            div()
                .relative()
                .w(px(track_width))
                .h(px(size * 0.72))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .w(px(bar_width))
                        .h_full()
                        .rounded(px((size * 0.05).max(1.)))
                        .bg(rgb(p.accent))
                        .with_animation(
                            id,
                            Animation::new(std::time::Duration::from_millis(1600))
                                .repeat()
                                .with_easing(gpui::bounce(gpui::ease_in_out)),
                            move |bar, progress| bar.left(px((track_width - bar_width) * progress)),
                        ),
                ),
        )
        .child("}")
}

fn status_dot(color: u32) -> Div {
    div()
        .size(px(6.))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(color))
}

fn overline(label: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.muted))
        .child(label.into())
}

fn keycap(label: &'static str, p: Palette) -> Div {
    div()
        .px_1()
        .rounded(px(4.))
        .bg(rgb(p.surface))
        .text_size(px(10.))
        .font_family(mono_font())
        .text_color(rgb(p.muted))
        .child(label)
}

fn session_status(session: &Session, p: Palette) -> (&str, u32) {
    if session.stopped() {
        return ("Ended", p.muted);
    }
    if session.approval.is_some() {
        return ("Needs approval", p.warning);
    }
    if session.questions.is_some() {
        return ("Needs your input", p.warning);
    }
    match session.state.as_str() {
        "responding" | "working" | "thinking" | "streaming" | "running" => ("Working", p.busy),
        "approval" | "waiting_approval" => ("Needs approval", p.warning),
        "question" | "waiting_input" => ("Needs your input", p.warning),
        "starting" | "initializing" => ("Starting", p.muted),
        "background" => ("Background", p.muted),
        "input" | "idle" => ("Ready", p.success),
        "stopped" | "ended" => ("Ended", p.muted),
        "" => ("Session", p.muted),
        state => (state, p.muted),
    }
}

fn session_badge(session: &Session, p: Palette, connected: bool) -> Div {
    let (label, color) = if connected {
        session_status(session, p)
    } else {
        ("Offline", p.muted)
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(if connected && session.working() {
            brand_spinner(
                12.,
                p,
                SharedString::from(format!("session-working-{}", session.id)),
            )
        } else {
            status_dot(color)
        })
        .child(label.to_owned())
}

actions!(
    native,
    [
        SendMessage,
        CreateSession,
        NextSession,
        PreviousSession,
        FocusComposer,
        Refresh,
        NormalMode,
        ShowProjects,
        ShowSettings,
        ShowConversation,
        ShowHistory,
        ShowChanges,
        ShowSetup,
        ShowSessionDetails,
        ShowModel,
        Search,
        OpenProject,
        FirstItem,
        LastItem,
        PageUp,
        PageDown,
        CycleTheme,
        ToggleVim,
        CycleProvider,
        Quit
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", NormalMode, Some("Workspace")),
        KeyBinding::new("escape", NormalMode, Some("Workspace > Input")),
        KeyBinding::new("ctrl-,", ShowSettings, Some("Workspace")),
        KeyBinding::new("cmd-,", ShowSettings, Some("Workspace")),
        KeyBinding::new("ctrl-p", ShowProjects, Some("Workspace")),
        KeyBinding::new("cmd-p", ShowProjects, Some("Workspace")),
        KeyBinding::new("t", CycleTheme, Some("VimNormal")),
        KeyBinding::new("v", ToggleVim, Some("VimNormal")),
        KeyBinding::new("a", CycleProvider, Some("VimNormal")),
        KeyBinding::new("j", NextSession, Some("VimNormal")),
        KeyBinding::new("k", PreviousSession, Some("VimNormal")),
        KeyBinding::new("i", FocusComposer, Some("VimNormal")),
        KeyBinding::new("n", CreateSession, Some("VimNormal")),
        KeyBinding::new("/", Search, Some("VimNormal")),
        KeyBinding::new("g p", ShowProjects, Some("VimNormal")),
        KeyBinding::new("g s", ShowSettings, Some("VimNormal")),
        KeyBinding::new("g c", ShowConversation, Some("VimNormal")),
        KeyBinding::new("g h", ShowHistory, Some("VimNormal")),
        KeyBinding::new("g d", ShowChanges, Some("VimNormal")),
        KeyBinding::new("g a", ShowSetup, Some("VimNormal")),
        KeyBinding::new("g e", ShowSessionDetails, Some("VimNormal")),
        KeyBinding::new("g m", ShowModel, Some("VimNormal")),
        KeyBinding::new("h", ShowProjects, Some("VimNormal")),
        KeyBinding::new("l", OpenProject, Some("VimNormal")),
        KeyBinding::new("enter", OpenProject, Some("VimNormal")),
        KeyBinding::new("g g", FirstItem, Some("VimNormal")),
        KeyBinding::new("shift-g", LastItem, Some("VimNormal")),
        KeyBinding::new("ctrl-u", PageUp, Some("VimNormal")),
        KeyBinding::new("ctrl-d", PageDown, Some("VimNormal")),
        KeyBinding::new("ctrl-n", CreateSession, Some("Workspace")),
        KeyBinding::new("cmd-n", CreateSession, Some("Workspace")),
        KeyBinding::new("ctrl-enter", SendMessage, Some("Workspace")),
        KeyBinding::new("cmd-enter", SendMessage, Some("Workspace")),
        // Input owns its own Enter bindings, so match the focused editor's
        // context as well; a root-only binding loses to its newline action.
        KeyBinding::new("ctrl-enter", SendMessage, Some("Workspace > Input")),
        KeyBinding::new("cmd-enter", SendMessage, Some("Workspace > Input")),
        KeyBinding::new("alt-down", NextSession, Some("Workspace")),
        KeyBinding::new("alt-up", PreviousSession, Some("Workspace")),
        KeyBinding::new("ctrl-l", FocusComposer, Some("Workspace")),
        KeyBinding::new("cmd-l", FocusComposer, Some("Workspace")),
        KeyBinding::new("ctrl-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-shift-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub struct Workspace {
    ui_bus: bus_commands::UiState,
    extras: features::Extras,
    chat: transcript::ChatUi,
    screen: Screen,
    settings: Settings,
    fonts: typography::FontControls,
    settings_path: Option<std::path::PathBuf>,
    project_scope: String,
    settings_error: String,
    project_filter: Option<String>,
    project_cursor: usize,
    project_path: Entity<InputState>,
    search: Entity<InputState>,
    navigation_selected: Option<String>,
    sidebar_collapsed: bool,
    has_connected: bool,
    sidebar_scroll: gpui::UniformListScrollHandle,
    projects_scroll: gpui::UniformListScrollHandle,
    _focus_watch: Vec<gpui::Subscription>,
    appearance: Appearance,
    theme_error: String,
    controller: Controller,
    view: Arc<View>,
    composer: Entity<InputState>,
    composer_dock_bounds: gpui::Bounds<gpui::Pixels>,
    header_bounds: gpui::Bounds<gpui::Pixels>,
    tool_expansion: HashMap<String, bool>,
    turn_clocks: HashMap<String, TurnClock>,
    duration_labels: HashMap<u64, String>,
    new_session: bool,
    provider: &'static str,
    project: Entity<InputState>,
    label: Entity<InputState>,
    model: Entity<InputState>,
    model_picker: Entity<SelectState<SearchableVec<PickerItem>>>,
    model_choice: String,
    context_window: Option<u64>,
    permission: Permission,
    catalog_models: Vec<ModelChoice>,
    model_reload: Option<Task<()>>,
    prompt: Entity<InputState>,
    spawn_pending: bool,
    last_spawn_receipt: u64,
    spawn_error: String,
    focus: FocusHandle,
    list: ListState,
    drafts: HashMap<String, String>,
    last_receipt: u64,
    local_notice: String,
    follow: bool,
    demo: bool,
    requested_session: Option<String>,
    selection_requested: bool,
    _updates: Task<()>,
    #[cfg(feature = "ui-tests")]
    rendered_rows: std::rc::Rc<std::cell::RefCell<std::collections::BTreeSet<usize>>>,
}

impl Workspace {
    pub fn new(
        controller: Controller,
        demo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fonts = typography::FontControls::new(window, cx);
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 4)
                .placeholder("Ask anything, or describe a task…")
        });
        let project =
            cx.new(|cx| InputState::new(window, cx).placeholder("Absolute project directory"));
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("Optional session name"));
        let model = cx.new(|cx| InputState::new(window, cx).placeholder("Exact model ID or alias"));
        let model_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(model_items(&[])),
                Some(gpui_component::IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(3)
                .placeholder("What would you like to work on? (optional)")
        });
        let project_path = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Absolute project directory on this hub")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
        let mut focus_watch = vec![cx.subscribe(
            &search,
            |this, _, event: &gpui_component::input::InputEvent, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    this.project_cursor = 0;
                    this.projects_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    this.sidebar_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    cx.notify();
                }
            },
        )];
        focus_watch.push(cx.subscribe_in(
            &model_picker,
            window,
            |this, _, event: &SelectEvent<SearchableVec<PickerItem>>, window, cx| {
                let SelectEvent::Confirm(value) = event;
                if this.spawn_pending || this.view.creating {
                    return;
                }
                this.model_choice = value.clone().unwrap_or_default();
                this.context_window = this
                    .catalog_models
                    .iter()
                    .find(|m| m.id == this.model_choice)
                    .and_then(|m| m.windows.first())
                    .copied();
                if this.model_choice == "__custom" {
                    this.model.update(cx, |input, cx| input.focus(window, cx));
                }
                cx.notify();
            },
        ));
        focus_watch.push(cx.subscribe_in(
            &project,
            window,
            |this, _, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) && this.new_session {
                    this.model_reload = Some(cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(400))
                            .await;
                        let _ = this.update_in(cx, |this, _, cx| this.load_models(false, cx));
                    }));
                    cx.notify();
                }
            },
        ));
        let mut incoming = controller.views.clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            while incoming.changed().await.is_ok() {
                let view = incoming.borrow_and_update().clone();
                if this
                    .update_in(cx, |this, window, cx| this.update_view(view, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let list = ListState::new(0, ListAlignment::Bottom, px(250.));
        list.set_scroll_handler(cx.listener(|this, event: &ListScrollEvent, window, cx| {
            this.follow = !event.is_scrolled;
            cx.defer_in(window, |this, window, cx| this.capture_reading(window, cx));
            cx.notify();
        }));
        let focus = cx.focus_handle();
        focus_watch.push(cx.on_focus(&focus, window, |_, _, cx| cx.notify()));
        focus_watch.push(cx.on_blur(&focus, window, |_, _, cx| cx.notify()));
        focus_watch.push(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.chat.restored = false;
                this.restore_reading(window, cx);
            } else {
                this.capture_reading(window, cx);
            }
            cx.notify();
        }));
        focus_watch.push(cx.on_release(|this, _| {
            if let Some(path) = &this.settings_path
                && let Err(error) = this.settings.save(path)
            {
                eprintln!("Could not save native reading positions: {error}");
            }
        }));
        window.focus(&focus);
        Self {
            extras: features::Extras::new(window, cx),
            chat: transcript::ChatUi::default(),
            ui_bus: Default::default(),
            screen: Screen::Conversation,
            settings: Settings::default(),
            fonts,
            settings_path: None,
            project_scope: "test".into(),
            settings_error: String::new(),
            project_filter: None,
            project_cursor: 0,
            project_path,
            search,
            navigation_selected: None,
            sidebar_collapsed: false,
            has_connected: false,
            sidebar_scroll: gpui::UniformListScrollHandle::new(),
            projects_scroll: gpui::UniformListScrollHandle::new(),
            _focus_watch: focus_watch,
            appearance: Appearance::default(),
            theme_error: String::new(),
            controller,
            view: Arc::new(View::default()),
            composer,
            composer_dock_bounds: Default::default(),
            header_bounds: Default::default(),
            tool_expansion: HashMap::new(),
            turn_clocks: HashMap::new(),
            duration_labels: HashMap::new(),
            new_session: false,
            provider: "claude",
            project,
            label,
            model,
            model_picker,
            model_choice: String::new(),
            context_window: None,
            permission: Permission::Ask,
            catalog_models: Vec::new(),
            model_reload: None,
            prompt,
            spawn_pending: false,
            last_spawn_receipt: 0,
            spawn_error: String::new(),
            focus,
            list,
            drafts: HashMap::new(),
            last_receipt: 0,
            local_notice: String::new(),
            follow: true,
            demo,
            requested_session: None,
            selection_requested: false,
            _updates: updates,
            #[cfg(feature = "ui-tests")]
            rendered_rows: Default::default(),
        }
    }

    pub fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        configure_theme(appearance, Some(window), cx);
        self.apply_typography(cx);
        cx.notify();
    }

    fn choose_theme(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_appearance(appearance, window, cx);
        self.theme_error = match preference_path() {
            Some(path) => appearance
                .save(&path)
                .err()
                .map(|e| format!("Theme applied, but could not save: {e}"))
                .unwrap_or_default(),
            None => "Theme applied, but no settings directory is available.".into(),
        };
    }

    pub fn open_session(&mut self, session: Option<String>) {
        self.requested_session = session;
    }

    fn update_view(&mut self, view: Arc<View>, window: &mut Window, cx: &mut Context<Self>) {
        self.has_connected |= view.connected;
        self.capture_reading(window, cx);
        self.receive_chat_requests(&view, cx);
        self.sync_features(&view, window, cx);
        if let Some(receipt) = &view.spawn_receipt
            && receipt.number > self.last_spawn_receipt
        {
            self.last_spawn_receipt = receipt.number;
            self.spawn_pending = false;
            self.spawn_error = receipt.error.clone().unwrap_or_default();
            if let Some(id) = &receipt.session {
                self.new_session = false;
                self.screen = Screen::Conversation;
                if let Some(message) = &receipt.unsent_message {
                    self.drafts.insert(id.clone(), message.clone());
                    if self.view.selected.as_ref() == Some(id) {
                        self.composer
                            .update(cx, |input, cx| input.set_value(message.clone(), window, cx));
                    }
                }
                self.prompt
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.composer
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
        if let Some(id) = &self.requested_session {
            if view.selected.as_ref() == Some(id) {
                self.selection_requested = false;
            } else {
                let available = view.sessions.iter().any(|session| &session.id == id);
                if !available {
                    self.selection_requested = false;
                }
                if !self.selection_requested && available {
                    self.selection_requested =
                        self.controller.command(Command::Select(id.clone())).is_ok();
                }
                // A requested target must never silently turn into a different
                // session, especially when an external harness drives input.
                self.local_notice = if view.connected {
                    if available {
                        format!("Opening requested session {id}…")
                    } else {
                        format!("Requested session {id} is unavailable; controls are disabled.")
                    }
                } else {
                    view.notice.clone()
                };
                self.view = Arc::new(View {
                    connected: false,
                    ui_requests: view.ui_requests.clone(),
                    ui_request_warning: view.ui_request_warning.clone(),
                    ..(*self.view).clone()
                });
                self.apply_ui_requests(window, cx);
                cx.notify();
                return;
            }
        }
        self.local_notice.clear();
        if self.view.selected != view.selected {
            self.tool_expansion.clear();
            if let Some(id) = &self.view.selected {
                self.drafts
                    .insert(id.clone(), self.composer.read(cx).value().to_string());
            }
            let draft = view
                .selected
                .as_ref()
                .and_then(|id| self.drafts.get(id))
                .cloned()
                .unwrap_or_default();
            self.composer
                .update(cx, |input, cx| input.set_value(draft, window, cx));
            self.list.reset(view.transcript.rows.len());
            self.chat.restored = false;
            self.chat.wanted.clear();
            self.chat.unread = None;
            self.follow = true;
        } else {
            let anchor = (!self.follow).then(|| self.scroll_anchor());
            let old = &self.view.transcript.rows;
            let new = &view.transcript.rows;
            let first = old
                .iter()
                .zip(new)
                .position(|(a, b)| !Arc::ptr_eq(a, b) && !a.same_content(b))
                .unwrap_or(old.len().min(new.len()));
            // Invalidate only changed measurements. Retain the scroll anchor
            // while reading history; bottom alignment follows when at the tail.
            // Revisions restart after a reconnect/reselection. Row identities
            // and count remain authoritative even if two revisions collide.
            let changed = first < old.len() || first < new.len();
            if changed {
                let restored = anchor.map(|anchor| scroll::remap_anchor(anchor, old, new));
                self.list.splice(first..old.len(), new.len() - first);
                if let Some(anchor) = restored {
                    self.list.scroll_to(anchor);
                }
            }
            if changed
                && self.follow
                && window.is_window_active()
                && self.screen == Screen::Conversation
            {
                self.list.scroll_to(ListOffset {
                    item_ix: new.len(),
                    offset_in_item: px(0.),
                });
            }
        }
        if let Some(receipt) = &view.receipt
            && receipt.number > self.last_receipt
        {
            self.last_receipt = receipt.number;
            if receipt.error.is_none()
                && matches!(receipt.action, Action::Stop)
                && let Some(clock) = self.turn_clocks.get_mut(&receipt.session)
            {
                clock.interrupt();
            }
            if receipt.error.is_none()
                && let Action::Send(sent) | Action::Answer(sent) = &receipt.action
            {
                if view.selected.as_ref() == Some(&receipt.session) {
                    if self.composer.read(cx).value().as_ref() == sent
                        || self.attachment_text(
                            &receipt.session,
                            self.composer.read(cx).value().as_ref(),
                        ) == *sent
                    {
                        self.composer
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                } else if self.drafts.get(&receipt.session) == Some(sent) {
                    self.drafts.remove(&receipt.session);
                }
            }
        }
        if self.navigation_selected.as_ref() == view.selected.as_ref()
            || self
                .navigation_selected
                .as_ref()
                .is_some_and(|id| !view.sessions.iter().any(|s| &s.id == id))
        {
            self.navigation_selected = None;
        }
        let reconnected = !self.view.connected && view.connected;
        if !view.loading {
            let identities = view
                .transcript
                .rows
                .iter()
                .filter(|r| r.tool.is_some())
                .map(|r| tools::identity(r))
                .collect::<std::collections::HashSet<_>>();
            self.tool_expansion
                .retain(|key, _| identities.contains(key));
        }
        let now = timing::now_ms();
        if !view.connected {
            for clock in self.turn_clocks.values_mut() {
                clock.update(&Session::default(), None, false, now);
            }
        }
        if let Some(id) = &view.selected
            && !self.turn_clocks.contains_key(id)
        {
            if self.turn_clocks.len() >= 128
                && let Some(old) = self.turn_clocks.keys().next().cloned()
            {
                self.turn_clocks.remove(&old);
            }
            let clock = self
                .settings_path
                .as_ref()
                .map(|path| TurnClock::load(&timing::history_path(path, &self.project_scope, id)))
                .transpose()
                .unwrap_or_else(|error| {
                    eprintln!("Could not load native timing history: {error}");
                    None
                })
                .unwrap_or_default();
            self.turn_clocks.insert(id.clone(), clock);
        }
        for session in view.sessions.iter() {
            if let Some(clock) = self.turn_clocks.get_mut(&session.id) {
                let previous_end = clock.completed.back().map(|turn| turn.ended_ms);
                let transcript = (!view.loading && view.selected.as_ref() == Some(&session.id))
                    .then_some(&view.transcript);
                clock.update(session, transcript, view.connected, now);
                if previous_end != clock.completed.back().map(|turn| turn.ended_ms)
                    && let Some(path) = &self.settings_path
                    && let Err(error) = clock.save(&timing::history_path(
                        path,
                        &self.project_scope,
                        &session.id,
                    ))
                {
                    self.local_notice = format!("Could not save completed timing: {error}");
                }
            }
        }
        self.duration_labels = view
            .selected
            .as_ref()
            .and_then(|id| self.turn_clocks.get(id))
            .map(|clock| clock.message_labels(&view.transcript))
            .unwrap_or_default();
        self.view = view;
        self.restore_reading(window, cx);
        if self.new_session || self.screen == Screen::Model {
            self.sync_models(window, cx);
            if reconnected {
                self.load_models(true, cx);
            }
        }
        self.apply_ui_requests(window, cx);
        cx.notify();
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        let command = if matches!(command, Command::Refresh) && self.view.power_paused {
            Command::ResumePowerPause(self.view.power_pause_generation)
        } else {
            command
        };
        if let Command::Select(id) = &command
            && self
                .requested_session
                .as_ref()
                .is_some_and(|target| target != id)
        {
            self.local_notice =
                "This window is pinned by --session. Open another window to browse sessions."
                    .into();
            cx.notify();
            return;
        }
        let selection = match &command {
            Command::Select(id) => Some(id.clone()),
            _ => None,
        };
        match self.controller.command(command) {
            Ok(()) => {
                if let Some(id) = selection {
                    self.navigation_selected = Some(id);
                }
                self.local_notice.clear();
            }
            Err(error) => self.local_notice = error.to_string(),
        }
        cx.notify();
    }

    fn send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_session {
            self.create(cx);
            return;
        }
        if self.screen == Screen::Projects {
            if self
                .project_path
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                self.add_project(window, cx);
            }
            return;
        }
        if self.screen != Screen::Conversation
            || self
                .navigation_selected
                .as_ref()
                .is_some_and(|id| Some(id) != self.view.selected.as_ref())
        {
            return;
        }
        let draft = self.composer.read(cx).value().to_string();
        let Some(id) = self.view.selected.clone() else {
            return;
        };
        let text = self.attachment_text(&id, &draft);
        if text.trim().is_empty() || self.view.busy || !self.view.connected || self.uploading() {
            return;
        }
        self.extras.sent_drafts.insert(id.clone(), draft);
        self.extras.sent_attachments.insert(
            id,
            self.extras
                .attachments
                .get(self.view.selected.as_ref().unwrap())
                .cloned()
                .unwrap_or_default(),
        );
        self.act(Action::Send(text), cx);
    }

    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if let Some(session) = self.view.selected.clone() {
            self.command(Command::Act { session, action }, cx);
        }
    }

    fn show_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.resume = None;
        self.extras.return_launch = false;
        if self.demo {
            return;
        }
        // --session protects automatic startup/refresh selection. An explicit
        // New session action is the user's decision to leave that pinned view.
        if self.requested_session.take().is_some() {
            self.selection_requested = false;
            self.navigation_selected = None;
            self.local_notice.clear();
            self.command(Command::Refresh, cx);
        }
        if !self.new_session {
            self.choose_provider(self.settings.default_provider.id(), window, cx);
        }
        self.new_session = true;
        self.screen = Screen::Conversation;
        if let Some(path) = self.project_filter.clone() {
            self.project
                .update(cx, |input, cx| input.set_value(path, window, cx));
        }
        if self.project.read(cx).value().is_empty() {
            let cwd = self
                .view
                .sessions
                .iter()
                .find(|s| Some(&s.id) == self.view.selected.as_ref())
                .map(|s| s.cwd.clone())
                .unwrap_or_default();
            self.project
                .update(cx, |input, cx| input.set_value(cwd, window, cx));
        }
        self.project.update(cx, |input, cx| input.focus(window, cx));
        self.load_models(false, cx);
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.demo
            || self.requested_session.is_some()
            || self.spawn_pending
            || self.view.creating
            || !self.view.connected
        {
            return;
        }
        let request = NewSession {
            provider: self.provider.into(),
            cwd: self.project.read(cx).value().to_string(),
            label: self.label.read(cx).value().to_string(),
            model: if self.model_choice == "__custom" {
                self.model.read(cx).value().to_string()
            } else {
                self.model_choice.clone()
            },
            context_window: self.context_window,
            permission: self.permission,
            message: self.prompt.read(cx).value().to_string(),
            resume_session_id: self.extras.resume.clone(),
        };
        self.spawn_error.clear();
        if self.model_choice == "__custom" && request.model.trim().is_empty() {
            self.spawn_error = "Enter a custom model or choose Provider default.".into();
            cx.notify();
            return;
        }
        match request
            .params()
            .and_then(|_| self.controller.command(Command::Create(request)))
        {
            Ok(()) => self.spawn_pending = true,
            Err(error) => self.spawn_error = error.to_string(),
        }
        cx.notify();
    }

    fn move_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.new_session || !matches!(self.screen, Screen::Conversation | Screen::Projects) {
            return;
        }
        if self.screen == Screen::Projects {
            let len = self.project_rows(cx).len();
            if len > 0 {
                self.project_cursor =
                    (self.project_cursor as isize + step).rem_euclid(len as isize) as usize;
                self.projects_scroll
                    .scroll_to_item(self.project_cursor, gpui::ScrollStrategy::Center);
                cx.notify();
            }
            return;
        }
        let rows = self.visible_sessions(cx);
        if rows.is_empty() {
            return;
        }
        let selected = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref());
        let current = rows
            .iter()
            .position(|&ix| Some(&self.view.sessions[ix].id) == selected);
        let index = current
            .map(|ix| (ix as isize + step).rem_euclid(rows.len() as isize) as usize)
            .unwrap_or(if step < 0 { rows.len() - 1 } else { 0 });
        self.sidebar_scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Center);
        self.command(
            Command::Select(self.view.sessions[rows[index]].id.clone()),
            cx,
        );
    }

    fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let id = id.into();
        let primary = matches!(
            id.as_ref(),
            "send" | "create-session" | "new-session" | "welcome-new"
        );
        self.button_style(id, label, enabled, primary)
    }

    fn primary_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        self.button_style(id, label, enabled, true)
    }

    fn button_style(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
        primary: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        chrome::interactive_control(div().id(id.into()), p, enabled)
            .px_3()
            .py_2()
            .font_weight(FontWeight::MEDIUM)
            .rounded(px(p.control_radius))
            .text_color(rgb(if enabled { p.text } else { p.disabled }))
            .text_size(px(12.))
            .when(primary, |d| {
                d.bg(rgb(if enabled { p.primary } else { p.selected }))
                    .text_color(rgb(if enabled { p.on_primary } else { p.disabled }))
            })
            .when(enabled, |d| {
                d.hover(|s| {
                    s.bg(rgb(if primary { p.primary_hover } else { p.selected }))
                        .text_color(rgb(if primary { p.on_primary } else { p.text }))
                })
                .active(|s| {
                    s.bg(rgb(if primary { p.primary_pressed } else { p.border }))
                        .text_color(rgb(if primary { p.on_primary } else { p.text }))
                })
            })
            .child(label.into())
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.appearance.palette();
        let compact = window.viewport_size().height < px(620.);
        let narrow = window.viewport_size().width < px(900.);
        let selected = self
            .view
            .sessions
            .iter()
            .find(|s| Some(&s.id) == self.view.selected.as_ref())
            .cloned();
        let enabled = self.view.connected
            && !self.view.loading
            && self
                .navigation_selected
                .as_ref()
                .is_none_or(|id| Some(id) == self.view.selected.as_ref())
            && !self.view.busy
            && selected.as_ref().is_some_and(|s| !s.stopped());
        let now = timing::now_ms();
        let clock = self
            .view
            .selected
            .as_ref()
            .and_then(|id| self.turn_clocks.get(id));
        let working = self.view.connected && selected.as_ref().is_some_and(Session::working);
        let animate_activity = (!self.view.connected && self.connection_copy().animated)
            || self.view.loading
            || self.view.busy
            || working;
        let activity = if !self.view.connected {
            Some(self.connection_copy().label.to_owned())
        } else if self.view.loading {
            Some("Loading conversation…".to_owned())
        } else if self.view.busy {
            Some("Sending…".to_owned())
        } else if working {
            Some(
                clock
                    .and_then(|clock| clock.elapsed_label(now))
                    .map(|elapsed| format!("Working · {elapsed}"))
                    .unwrap_or_else(|| "Working…".into()),
            )
        } else if clock.and_then(|clock| clock.elapsed_label(now)).is_some() {
            Some("Waiting…".to_owned())
        } else {
            clock.and_then(|clock| clock.latest_completion_for(&self.view.transcript))
        };
        let dock_view = cx.entity().downgrade();
        let header_view = cx.entity().downgrade();
        let title = selected
            .as_ref()
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Your sessions".into());
        let notice = if !self.local_notice.is_empty() {
            self.local_notice.clone()
        } else if !self.view.connected && self.view.notice.starts_with("Starting Rust backend") {
            String::new()
        } else if !self.view.connected {
            self.view
                .notice
                .trim_end_matches(". Reconnecting…")
                .to_owned()
        } else {
            self.view.notice.clone()
        };
        let entity = cx.entity().downgrade();
        let transcript = list(self.list.clone(), move |ix, window, cx| {
            entity
                .update(cx, |this, cx| this.render_chat_row(ix, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0()
        .pt(self.header_bounds.size.height + px(16.))
        // This is scrollable tail space, not a smaller viewport: history still
        // paints behind the floating dock, while the last message can clear it.
        .pb(if selected.is_some() {
            self.composer_dock_bounds.size.height + px(12.)
        } else {
            px(0.)
        });

        let sidebar = self.render_sidebar(narrow, compact, window, cx);

        if self.new_session {
            let busy = self.spawn_pending || self.view.creating;
            let can_create = self.view.connected && !busy;
            return self.shell(window, cx)
                .child(sidebar)
                .child(
                    div()
                        .id("new-session-form")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .p_5()
                        .child(
                            div()
                                .max_w(px(560.))
                                .mx_auto()
                                .flex()
                                .flex_col()
                                .gap_3().py_4()
                                .child(overline("NEW SESSION", p))
                                .child(brand_mark(36., p))
                                .child(
                                    div()
                                        .text_size(px(24.))
                                        .font_weight(FontWeight::BOLD)
                                        .child("Start something new"),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(p.muted))
                                        .text_size(px(12.))
                                        .child("Choose an agent and a project directory to begin."),
                                )
                                .child("Provider")
                                .child(div().flex().gap_2().children(
                                    [("claude", "Claude"), ("codex", "Codex")].into_iter().map(
                                        |(provider, label)| {
                                            self.button(provider, label, !busy).flex_1().py_3().flex().justify_center()
                                                .when(self.provider == provider, |d| {
                                                    d.bg(rgb(p.selected))
                                                })
                                                .when(!busy, |d| {
                                                    d.on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.choose_provider(provider, window, cx);
                                                            this.load_models(false, cx);
                                                            cx.notify();
                                                        },
                                                    ))
                                                })
                                        },
                                    ),
                                ))
                                .child(div().flex().justify_between().child("Project directory")
                                    .when(self.extras.local_paths, |d| d.child(self.button("browse-project", "Browse…", !busy).when(!busy, |d| d.on_click(cx.listener(|this, _, window, cx| this.pick_folder(false, window, cx)))))))
                                .when(self.extras.resume.is_some(), |d| d.child(div().text_color(rgb(p.accent)).child("Resume this conversation. Review the model and permissions before continuing.")))
                                .child(Input::new(&self.project).disabled(busy))
                                .child(
                                    div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                                        "Use an existing absolute path on the hub's machine.",
                                    ),
                                )
                                .child("Session name")
                                .child(Input::new(&self.label).disabled(busy))
                                .child(self.button("launch-setup", "Agent setup…", !busy).when(!busy, |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Setup, window, cx)))))
                                .child(self.render_launch_options(busy, cx))
                                .child("First message")
                                .child(Input::new(&self.prompt).h(px(110.)).disabled(busy))
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
                                })
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .child(self.button("cancel-create", "Back", !busy).when(
                                            !busy,
                                            |d| {
                                                d.on_click(cx.listener(|this, _, window, cx| {
                                                    this.show_screen(Screen::Conversation, window, cx);
                                                }))
                                            },
                                        ))
                                        .child(
                                            self.button(
                                                "create-session",
                                                if busy {
                                                    "Creating session…"
                                                } else {
                                                    "Create session"
                                                },
                                                can_create,
                                            )
                                            .when(
                                                can_create,
                                                |d| {
                                                    d.on_click(
                                                        cx.listener(|this, _, _, cx| {
                                                            this.create(cx)
                                                        }),
                                                    )
                                                },
                                            ),
                                        ),
                                ),
                        ),
                )
                .into_any_element();
        }

        if matches!(
            self.screen,
            Screen::Recent
                | Screen::Changes
                | Screen::History
                | Screen::Session
                | Screen::Setup
                | Screen::Model
        ) {
            let content = self.render_feature(window, cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .into_any_element();
        }
        if self.screen == Screen::Projects {
            let content = self.render_projects(cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .into_any_element();
        }
        if self.screen == Screen::Settings {
            let content = self.render_settings(cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .into_any_element();
        }

        let header = div().absolute().top_0().left_0().w_full().occlude().bg(rgb(p.chat)).border_b_1().border_color(rgb(p.border)).flex().justify_center()
            .child(chrome::chat_column().relative().py_3().flex().flex_col().gap_2()
                .child(canvas(move |bounds, _, cx| {
                    cx.defer(move |cx| {
                        let _ = header_view.update(cx, |this, cx| {
                            if this.header_bounds != bounds {
                                let delta = bounds.size.height - this.header_bounds.size.height;
                                if !this.follow && delta != px(0.) {
                                    let mut anchor = this.scroll_anchor();
                                    anchor.offset_in_item += delta;
                                    this.list.scroll_to(anchor);
                                }
                                this.header_bounds = bounds;
                                cx.notify();
                            }
                        });
                    });
                }, |_, _, _, _| {}).absolute().top_0().left_0().size_full())
                .child(div().flex().items_center().gap_3()
                    .child(div().flex_1().min_w_0().flex().items_center().gap_3()
                        .when(!narrow, |d| d.when_some(selected.as_ref(), |d, session| d
                            .child(div().max_w(px(160.)).truncate().text_size(px(12.)).text_color(rgb(p.muted)).child(chrome::project_label(&session.cwd).to_owned()))
                            .child(div().text_color(rgb(p.border)).child("/"))))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title)))
                    .when(!narrow, |d| d.child(self.chat_actions(enabled, cx))))
                .when(narrow, |d| d.child(div().flex().items_center().justify_between().gap_2()
                    .when_some(selected.as_ref(), |d, session| d.child(session_badge(session, p, self.view.connected)))
                    .child(self.chat_actions(enabled, cx))))
                .when(!self.extras.notice.is_empty(), |d| d.child(div().px_5().text_color(rgb(p.warning)).child(self.extras.notice.clone())))
                .when(!notice.is_empty(), |d| d.child(div().occlude().py_1().text_size(px(11.)).text_color(rgb(p.warning)).child(notice)))
                .when(!self.view.connected && !self.view.transcript.rows.is_empty(), |d| d.child(self.render_connection_banner(cx)))
                .when(self.view.transcript.omitted, |d| d.child(div().occlude().rounded_md().bg(rgb(p.surface)).px_3().text_size(px(11.)).text_color(rgb(p.muted)).child("Showing recent messages. Open History to browse older retained messages.")))
                .when(self.view.loading && !self.view.transcript.rows.is_empty(), |d| d.child(div().occlude().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(p.muted)).child(brand_spinner(12., p, "conversation-refresh")).child("Refreshing conversation…")))
                .when(self.view.connected && !self.view.loading && self.view.notice.starts_with("Conversation unavailable:"), |d| d.child(self.button("retry-conversation", "Retry conversation", true).debug_selector(|| "retry-conversation".into()).on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))))
);

        self.shell(window, cx)
            .child(sidebar)
            .child(div().relative().flex_1().min_w_0().h_full().flex().flex_col().bg(rgb(p.chat))
                .when(self.view.transcript.rows.is_empty(), |d| d.child(self.render_empty_state(compact, window, cx)))
                .when(!self.view.transcript.rows.is_empty(), |d| d.child(transcript))
                .child(header)
                .when(!self.follow, |d| d.child(div().absolute().left_0().w_full().bottom(self.composer_dock_bounds.size.height + px(6.)).flex().justify_center().child(self.button("latest", "Jump to latest", true).shadow(chrome::floating_shadow(p)).debug_selector(|| "jump-latest".into()).mx_auto().rounded_full().bg(rgb(p.surface)).occlude().on_click(cx.listener(|this, _, window, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset { item_ix: this.view.transcript.rows.len(), offset_in_item: px(0.) }); this.chat.unread = None; this.capture_reading(window, cx); cx.notify();
                })))))
                .child(div().absolute().bottom_0().left_0().w_full().flex().justify_center()
                    .child(chrome::chat_column().id("conversation-dock").relative().pt(px(if compact { 8. } else { 12. })).pb(px(if compact { 8. } else { 16. })).max_h(window.viewport_size().height * if compact { 0.45 } else { 0.55 }).overflow_y_scroll().flex().flex_col().gap(px(if compact { 4. } else { 8. }))
                .child(canvas(move |bounds, _, cx| {
                    cx.defer(move |cx| {
                        let _ = dock_view.update(cx, |this, cx| {
                            if this.composer_dock_bounds != bounds {
                                this.composer_dock_bounds = bounds;
                                cx.notify();
                            }
                        });
                    });
                }, |_, _, _, _| {}).absolute().top_0().left_0().size_full())
                .when_some(self.chat.unread, |d, ix| d.child(self.button("new-activity", "New activity · jump to first unread", true).on_click(cx.listener(move |this, _, _, cx| {
                    this.follow = false;
                    this.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) }); cx.notify();
                }))))
                .child(self.render_pending(window, cx))
                .when_some(selected.as_ref().and_then(|s| s.approval.as_ref()), |d, approval| {
                    let label = approval.get("toolName").or_else(|| approval.get("tool")).and_then(serde_json::Value::as_str).unwrap_or("Tool");
                    let summary = approval.pointer("/toolInput/command").or_else(|| approval.pointer("/toolInput/file_path")).and_then(serde_json::Value::as_str).unwrap_or("").lines().next().unwrap_or("").to_owned();
                    let details = serde_json::to_string_pretty(approval.get("toolInput").or_else(|| approval.get("raw")).unwrap_or(approval)).unwrap_or_default();
                    d.child(div().occlude().w_full().p(px(if compact { 8. } else { 12. })).rounded(px(p.panel_radius)).shadow(chrome::floating_shadow(p)).bg(rgb(p.surface)).flex_shrink_0().flex().flex_col().gap_2().text_size(px(12.))
                        .child(div().flex().items_center().gap_2().text_color(rgb(p.warning)).child(status_dot(p.warning)).child(format!("Permission needed · {label}"))
                            .when(compact, |d| d.child(div().flex_1().min_w_0().truncate().text_color(rgb(p.text)).font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).child(summary.clone())).child(self.button("approval-toggle-compact", if self.extras.approval_details { "Hide" } else { "Details" }, true).on_click(cx.listener(|this, _, _, cx| { this.extras.approval_details = !this.extras.approval_details; cx.notify(); })))))
                        .when(!compact, |d| d.child(div().flex().items_center().gap_2().child(div().flex_1().min_w_0().truncate().font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).child(if summary.is_empty() { "Review request details".to_owned() } else { summary })).child(self.button("approval-toggle", if self.extras.approval_details { "Hide details" } else { "Details" }, true).on_click(cx.listener(|this, _, _, cx| { this.extras.approval_details = !this.extras.approval_details; cx.notify(); })))))
                        .when(self.extras.approval_details, |d| d.child(div().id("approval-details").max_h(px(if compact { 52. } else { 120. })).overflow_y_scroll().p_3().rounded_md().bg(rgb(p.surface)).font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).text_color(rgb(p.muted)).child(details)))
                        .child(div().flex().gap_2()
                            .child(self.primary_button("approve", "Allow once", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(true), cx)))))
                            .child(self.button("deny", "Deny", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(false), cx)))))))
                })
                .when_some(selected.as_ref().filter(|s| s.questions.is_some()), |d, session| d.child(self.render_questions(session, enabled, compact, cx)))
                .when(selected.is_some(), |d| d.child(div().w_full().flex_shrink_0().flex().flex_col().gap_2()
                    .child(div().id("floating-composer").debug_selector(|| "chat-composer".into()).occlude().bg(rgb(p.surface)).border_1()
                        .border_color(if self.composer.read(cx).focus_handle(cx).is_focused(window) { rgb(p.accent).into() } else { gpui::Hsla::from(rgb(p.border)).opacity(0.55) })
                        .rounded(px(p.composer_radius)).shadow(chrome::floating_shadow(p)).p(px(if compact { 8. } else { 12. })).flex().flex_col().gap_2()
                        .when(self.view.selected.as_ref().and_then(|id| self.extras.attachments.get(id)).is_some_and(|v| !v.is_empty()), |d| d.child(div().flex().flex_wrap().gap_2()
                            .children(self.view.selected.as_ref().and_then(|id| self.extras.attachments.get(id)).into_iter().flatten().enumerate().map(|(ix, (name, _))| {
                                div().id(("attachment", ix)).pl_2().pr_1().py_1().rounded_md().bg(rgb(p.selected)).max_w_full().flex().items_center().gap_2()
                                    .child(Icon::new(IconName::File).size(px(14.)).text_color(rgb(p.accent)))
                                    .child(div().min_w_0().truncate().text_size(px(12.)).child(name.clone()))
                                    .child(self.icon_button("remove-attachment", "Remove attachment", IconName::Close, !self.view.busy).size(px(24.)).when(!self.view.busy, |d| d.on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(id) = &this.view.selected && let Some(files) = this.extras.attachments.get_mut(id) && ix < files.len() { files.remove(ix); } cx.notify();
                                    }))))
                            }))))
                        .child(Input::new(&self.composer).appearance(false).disabled(self.view.busy))
                        .child(div().flex().items_center().justify_between().gap_2()
                            .child(div().flex().items_center().gap_1()
                                .child(self.icon_button("attach-file", if self.uploading() { "Attaching file…" } else { "Attach a file" }, IconName::Plus, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, window, cx| this.pick_attachment(window, cx)))))
                                .child(self.icon_button("paste-image", "Paste an image from the clipboard", IconName::GalleryVerticalEnd, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, _, cx| { if !this.paste_image(cx) { this.extras.notice = "No supported image on the clipboard.".into(); cx.notify(); } })))))
                            .child(div().flex().items_center().gap_2()
                                .when(working, |d| d.child(self.quiet_button("stop", "Interrupt", IconName::WindowClose, enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Stop, cx))))))
                                .child(self.primary_icon_button("send", if self.view.busy {"Sending…"} else if working {"Queue message"} else {"Send message"}, IconName::ArrowUp, enabled && !self.uploading()).size(px(32.)).rounded_full()
                                    .when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx))))))))
                    .child(div().when(compact, |d| d.hidden()).occlude().bg(rgb(p.chat)).rounded_md().px_2().py_1().flex().items_center().justify_between().gap_2().text_size(px(10.)).text_color(rgb(p.muted))
                        .child(div().flex_1().min_w_0().flex().items_center().gap_2()
                            .when(animate_activity, |d| d.child(brand_spinner(12., p, "composer-activity")))
                            .child(div().truncate().child(activity.unwrap_or_else(|| if enabled { "Ready" } else { "Session unavailable" }.into()))))
                        .child(div().flex().gap_1().items_center().flex_shrink_0().child(keycap(if cfg!(target_os = "macos") { "⌘ Enter" } else { "Ctrl Enter" }, p)).child("to send"))
                        .when(!narrow, |d| d.child("Enter for a new line")))))) ))
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, size};
    use gpui_component::Root;
    use wks_native::{
        controller::Receipt,
        model::{ConversationSnapshot, Item, Session, Transcript},
    };

    fn fixture(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Workspace>,
        VisualTestContext,
        tokio::sync::mpsc::Receiver<Command>,
        tokio::sync::watch::Sender<Arc<View>>,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            bind_keys(cx);
        });
        let (controller, commands, updates) = Controller::test_channels();
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| Workspace::new(controller, true, window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1000.), px(700.)));
        (workspace.unwrap(), visual, commands, updates)
    }

    fn state(id: &str) -> View {
        View {
            connected: true,
            selected: Some(id.into()),
            sessions: Arc::new(vec![
                Session {
                    id: "a".into(),
                    label: "Alpha".into(),
                    state: "input".into(),
                    ..Default::default()
                },
                Session {
                    id: "b".into(),
                    label: "Beta".into(),
                    state: "input".into(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }
    }

    #[gpui::test]
    fn markdown_file_link_requests_preview_and_shows_the_result(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: "[Open README](docs/README.md:12)".into(),
                        ..Default::default()
                    }],
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        let bounds = visual
            .debug_bounds("markdown-inline-live:a:0-0")
            .expect("native inline file link");
        let start = bounds.origin + gpui::point(px(2.), bounds.size.height / 2.);
        let end = bounds.origin + gpui::point(px(90.), bounds.size.height / 2.);
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(
            end,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        visual.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(
            commands.try_recv().is_err(),
            "selecting a link must not open it"
        );
        visual.simulate_click(
            bounds.origin + gpui::point(px(25.), bounds.size.height / 2.),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        let request = commands.try_recv().expect("file click issues a request");
        let Command::Request(wks_native::features::Request::FilePreview { session, path }) =
            request
        else {
            panic!("file click must request a native preview");
        };
        assert_eq!(session, "a");
        assert_eq!(path, "/fixture/repo/docs/README.md");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert("file-preview", wks_native::features::RequestState {
                request: wks_native::features::Request::FilePreview { session, path },
                number: 1, loading: false, error: None,
                        value: Arc::new(serde_json::json!({"contents":"# Native preview\nFile content loaded."})),
            });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("file-preview-panel").is_some());
    }

    #[gpui::test]
    fn font_controls_preserve_drafts_and_survive_theme_changes(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 50,
                    first_seq: 1,
                    items: (0..50)
                        .map(|i| Item {
                            kind: "user_message".into(),
                            text: format!("Message {i}"),
                            ..Default::default()
                        })
                        .collect(),
                });
                this.update_view(Arc::new(view), window, cx);
                this.follow = false;
                this.list.scroll_to(ListOffset {
                    item_ix: 12,
                    offset_in_item: px(7.),
                });
                this.composer.update(cx, |input, cx| {
                    input.set_value("Keep this draft 🦀", window, cx)
                });
                this.fonts.interface.update(cx, |_, cx| {
                    cx.emit(
                        SelectEvent::<SearchableVec<typography::FontChoice>>::Confirm(Some(
                            String::new(),
                        )),
                    )
                });
                this.fonts.code.update(cx, |_, cx| {
                    cx.emit(
                        SelectEvent::<SearchableVec<typography::FontChoice>>::Confirm(Some(
                            "Inter".into(),
                        )),
                    )
                });
            });
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                assert!(this.settings.interface_font.is_empty());
                assert_eq!(this.settings.code_font, "Inter");
                let anchor = this.list.logical_scroll_top();
                this.settings.text_size = 19;
                this.set_appearance(Appearance::Nord, window, cx);
                let theme = gpui_component::Theme::global(cx);
                assert_eq!(theme.font_family.as_ref(), ".SystemUIFont");
                assert_eq!(theme.mono_font_family.as_ref(), "Inter");
                assert_eq!(theme.font_size, px(19.));
                assert_eq!(this.list.logical_scroll_top().item_ix, 12);
                assert_eq!(
                    this.list.logical_scroll_top().offset_in_item,
                    anchor.offset_in_item
                );
                assert_eq!(
                    this.composer.read(cx).value().as_ref(),
                    "Keep this draft 🦀"
                );
                this.follow = true;
                this.apply_typography(cx);
                assert_eq!(
                    this.list.logical_scroll_top().item_ix,
                    this.list.item_count()
                );
            });
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn empty_sidebar_filters_can_be_cleared_without_switching_sessions(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.project_filter = Some("/missing-project".into());
                this.search
                    .update(cx, |input, cx| input.set_value("missing", window, cx));
                cx.notify();
            });
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(this.visible_sessions(cx).is_empty())
        });
        let reset = visual.debug_bounds("clear-session-filters").unwrap();
        assert!(reset.left() >= px(0.) && reset.right() <= px(232.));
        assert!(reset.top() >= px(0.) && reset.bottom() <= px(480.));
        visual.simulate_click(reset.center(), gpui::Modifiers::default());
        workspace.read_with(&visual, |this, cx| {
            assert!(this.project_filter.is_none());
            assert!(this.search.read(cx).value().is_empty());
            assert_eq!(this.visible_sessions(cx), vec![0, 1]);
            assert_eq!(this.view.selected.as_deref(), Some("a"));
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn vim_navigation_never_steals_composer_text(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("j k");
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
        visual.simulate_keystrokes("shift-g g g");
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
        visual.simulate_keystrokes("ctrl-d ctrl-u i");
        visual.simulate_input("jkgn/hello");
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "jkgn/hello")
        });
        assert!(commands.try_recv().is_err());
        visual.simulate_keystrokes("escape g s");
        workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Settings));
        visual.simulate_keystrokes("ctrl-enter");
        assert!(
            commands.try_recv().is_err(),
            "settings must never send a hidden draft"
        );
        visual.simulate_keystrokes("escape i");
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "jkgn/hello")
        });
    }

    #[gpui::test]
    fn project_navigation_filters_and_seeds_new_sessions(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                let rows = Arc::make_mut(&mut view.sessions);
                rows[0].cwd = "/one/app".into();
                rows[1].cwd = "/two/app".into();
                this.demo = false;
                this.settings.default_provider = Provider::Codex;
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.simulate_keystrokes("g p j enter");
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.project_filter.as_deref(), Some("/two/app"));
            assert_eq!(this.visible_sessions(cx), vec![1]);
        });
        visual.simulate_keystrokes("n");
        workspace.read_with(&visual, |this, cx| {
            assert!(this.new_session);
            assert_eq!(this.provider, "codex");
            assert_eq!(this.project.read(cx).value().as_ref(), "/two/app");
        });
        visual.simulate_input("jkgn");
        workspace.read_with(&visual, |this, cx| {
            assert!(this.project.read(cx).value().ends_with("jkgn"))
        });
        while let Ok(command) = commands.try_recv() {
            assert!(
                matches!(command, Command::LoadModels { .. }),
                "typing in form must not launch or select agents"
            );
        }
    }

    #[gpui::test]
    fn search_and_disabled_vim_keep_input_and_modifier_shortcuts(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("/");
        visual.simulate_input("Beta");
        visual.simulate_keystrokes("escape j");
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.settings.vim_navigation = false;
                cx.notify();
            })
        });
        visual.simulate_keystrokes("j g s");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Conversation)
        });
        assert!(commands.try_recv().is_err());
        visual.simulate_keystrokes("ctrl-,");
        workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Settings));
        visual.simulate_keystrokes("ctrl-p");
        workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Projects));
    }

    #[gpui::test]
    fn project_bookmark_can_be_saved_without_launching_an_agent(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_keystrokes("g p i");
        visual.simulate_input("/work/jk-project");
        visual.simulate_keystrokes("ctrl-enter");
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.settings.bookmarks("test"), ["/work/jk-project"]);
            assert!(this.project_path.read(cx).value().is_empty());
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn settings_shortcuts_update_defaults_without_touching_sessions(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_keystrokes("g s a v");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.settings.default_provider, Provider::Codex);
            assert!(!this.settings.vim_navigation);
        });
        assert!(commands.try_recv().is_err());
        visual.simulate_keystrokes("escape g p");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Conversation)
        });
        visual.simulate_keystrokes("ctrl-p");
        workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Projects));
    }

    #[gpui::test]
    fn failed_selection_does_not_leave_navigation_pending(cx: &mut TestAppContext) {
        let (workspace, mut visual, commands, _updates) = fixture(cx);
        drop(commands);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.command(Command::Select("b".into()), cx);
                assert!(this.navigation_selected.is_none());
                assert!(!this.local_notice.is_empty());
            })
        });
    }

    #[gpui::test]
    fn switching_themes_preserves_session_and_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Keep my draft", window, cx));
                for appearance in Appearance::ALL {
                    this.set_appearance(appearance, window, cx);
                    assert_eq!(this.appearance, appearance);
                    assert_eq!(this.view.selected.as_deref(), Some("a"));
                    assert_eq!(this.composer.read(cx).value().as_ref(), "Keep my draft");
                    let theme = gpui_component::Theme::global(cx);
                    assert_eq!(theme.is_dark(), appearance != Appearance::Light);
                    assert_eq!(
                        theme.colors.background,
                        gpui::Hsla::from(rgb(appearance.palette().base))
                    );
                }
            });
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn new_session_click_leaves_a_pinned_view_and_creates_the_selected_session(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.open_session(Some("a".into()));
                let mut initial = state("a");
                Arc::make_mut(&mut initial.sessions)[0].cwd = "/work/project".into();
                this.update_view(Arc::new(initial), window, cx);
            })
        });
        visual.run_until_parked();
        let button = visual.debug_bounds("new-session-button").unwrap();
        visual.simulate_click(button.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(this.new_session);
            assert!(this.requested_session.is_none());
            assert_eq!(this.project.read(cx).value().as_ref(), "/work/project");
        });
        while let Ok(command) = commands.try_recv() {
            assert!(
                matches!(command, Command::Refresh | Command::LoadModels { .. }),
                "opening the form must not launch anything"
            );
        }
        visual.simulate_keystrokes("ctrl-enter");
        let Command::Create(request) = commands.try_recv().expect("create from the pinned window")
        else {
            panic!("expected create")
        };
        assert_eq!(request.cwd, "/work/project");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut created = state("b");
                created.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 1,
                    session: Some("b".into()),
                    error: None,
                    unsent_message: None,
                });
                this.update_view(Arc::new(created), window, cx);
                assert_eq!(this.view.selected.as_deref(), Some("b"));
                assert!(!this.new_session);
                assert!(this.requested_session.is_none());
            })
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn new_session_form_creates_once_and_keeps_failed_input(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
            })
        });
        visual.simulate_keystrokes("ctrl-n");
        visual.simulate_input("/work/project");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                assert!(this.new_session);
                this.provider = "codex";
                this.prompt
                    .update(cx, |input, cx| input.set_value("Hello", window, cx));
            })
        });
        visual.simulate_keystrokes("ctrl-enter");
        visual.simulate_keystrokes("ctrl-enter");
        let request = loop {
            match commands.try_recv().expect("create command") {
                Command::Create(request) => break request,
                Command::LoadModels { .. } => {}
                _ => panic!("wrong command"),
            }
        };
        assert_eq!(request.cwd, "/work/project");
        assert_eq!(request.provider, "codex");
        assert_eq!(request.message, "Hello");
        assert!(commands.try_recv().is_err());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut failed = state("a");
                failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 1,
                    session: None,
                    error: Some("Launch failed".into()),
                    unsent_message: None,
                });
                this.update_view(Arc::new(failed), window, cx);
                assert!(this.new_session);
                assert!(!this.spawn_pending);
                assert_eq!(this.prompt.read(cx).value().as_ref(), "Hello");
                assert_eq!(this.project.read(cx).value().as_ref(), "/work/project");
                assert_eq!(this.spawn_error, "Launch failed");
                let mut success = state("b");
                success.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 2,
                    session: Some("b".into()),
                    error: None,
                    unsent_message: Some("Hello".into()),
                });
                this.update_view(Arc::new(success), window, cx);
                assert!(!this.new_session);
                assert_eq!(this.composer.read(cx).value().as_ref(), "Hello");
            })
        });
    }

    #[gpui::test]
    fn keyboard_send_preserves_unicode_and_failed_drafts(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("résumé 🦀");
        visual.simulate_keystrokes("ctrl-enter");
        let Command::Act {
            session,
            action: Action::Send(text),
        } = commands
            .try_recv()
            .expect("send shortcut reaches controller")
        else {
            panic!("wrong action");
        };
        assert_eq!(session, "a");
        assert_eq!(text, "résumé 🦀");
        let mut failed = state("a");
        failed.receipt = Some(Receipt {
            number: 1,
            session: "a".into(),
            action: Action::Send(text.clone()),
            error: Some("Disconnected; outcome unknown".into()),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(failed), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), text);
            })
        });
    }

    #[gpui::test]
    fn late_send_completion_does_not_clear_another_sessions_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Alpha draft", window, cx));
                this.update_view(Arc::new(state("b")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Beta draft", window, cx));
                let mut acknowledged = state("b");
                acknowledged.receipt = Some(Receipt {
                    number: 1,
                    session: "a".into(),
                    action: Action::Send("Alpha draft".into()),
                    error: None,
                });
                this.update_view(Arc::new(acknowledged), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "Beta draft");
                assert!(!this.drafts.contains_key("a"));
            })
        });
    }

    #[gpui::test]
    fn long_transcript_only_builds_visible_rows(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 2000,
            first_seq: 1,
            items: (0..2000)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nA paragraph in a long transcript."),
                    ..Default::default()
                })
                .collect(),
        });
        view.transcript = transcript;
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        visual.simulate_resize(size(px(900.), px(650.)));
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            let rendered = this.rendered_rows.borrow().len();
            assert!(
                rendered > 0 && rendered < 100,
                "constructed {rendered} of 2000 rows"
            );
        });
    }

    #[gpui::test]
    fn keyboard_controls_skip_disabled_actions_and_keep_their_geometry(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer.update(cx, |input, cx| {
                    input.set_value("Keyboard draft", window, cx)
                });
                window.focus(&this.focus);
            })
        });
        visual.run_until_parked();
        let before = visual.debug_bounds("sidebar-toggle").unwrap();
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        assert_eq!(before, visual.debug_bounds("sidebar-toggle").unwrap());
        visual.simulate_keystrokes("enter");
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
        // New session is disabled in this demo fixture; Tab should skip it.
        visual.simulate_keystrokes("tab space");
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("space").unwrap(),
        });
        visual.run_until_parked();
        assert!(visual.update(|window, cx| {
            workspace
                .read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        workspace.read_with(&visual, |this, cx| {
            assert!(!this.sidebar_collapsed);
            assert!(!this.new_session);
            assert_eq!(this.composer.read(cx).value().as_str(), "Keyboard draft");
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn keyboard_session_selection_uses_one_explicit_command(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.settings.vim_navigation = false;
                window.focus(&this.focus);
            })
        });
        visual.run_until_parked();
        // Toggle, search, projects, history, then the first session.
        visual.simulate_keystrokes("tab tab tab tab tab enter");
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        visual.run_until_parked();
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn loading_and_empty_states_follow_the_real_request_state(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("state-connection").is_some());
        assert!(visual.debug_bounds("welcome-new").is_none());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(
                    Arc::new(View {
                        connected: true,
                        sessions_loading: true,
                        ..Default::default()
                    }),
                    window,
                    cx,
                );
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("state-sessions-loading").is_some());
        assert!(visual.debug_bounds("state-no-sessions").is_none());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(
                    Arc::new(View {
                        connected: true,
                        ..Default::default()
                    }),
                    window,
                    cx,
                );
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("state-no-sessions").is_some());
        assert!(visual.debug_bounds("welcome-setup").is_some());
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn conversation_retry_and_first_message_keep_the_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.loading = true;
                this.update_view(Arc::new(next), window, cx);
                this.composer.update(cx, |input, cx| {
                    input.set_value("Retain my draft", window, cx)
                });
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("state-conversation-loading").is_some());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.notice = "Conversation unavailable: request timed out".into();
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("state-conversation-error").is_some());
        let retry = visual.debug_bounds("retry-empty").unwrap();
        visual.simulate_click(retry.center(), gpui::Modifiers::default());
        assert!(matches!(commands.try_recv().unwrap(), Command::Refresh));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.run_until_parked();
        let write = visual.debug_bounds("focus-first-message").unwrap();
        visual.simulate_click(write.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.update(|window, cx| {
            workspace
                .read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_str(), "Retain my draft")
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn connection_pause_banner_requires_a_click_and_keeps_scrollback(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let mut next = state("a");
        next.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: "Saved conversation".into(),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx)
            })
        });
        next.connected = false;
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx)
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.connection_copy().label, "Reconnecting…");
            assert!(this.connection_copy().animated);
        });
        assert!(visual.debug_bounds("last-transcript-row").is_some());
        assert!(visual.debug_bounds("wake-workspace").is_none());
        next.power_paused = true;
        next.can_resume_power_pause = true;
        next.power_pause_generation = 42;
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Offline draft", window, cx));
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("last-transcript-row").is_some());
        assert!(visual.debug_bounds("connection-banner").is_some());
        workspace.read_with(&visual, |this, _| assert!(!this.connection_copy().animated));
        assert!(commands.try_recv().is_err());
        let wake = visual.debug_bounds("wake-workspace").unwrap();
        visual.simulate_click(wake.center(), gpui::Modifiers::default());
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ResumePowerPause(42)
        ));
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_str(), "Offline draft")
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn sidebar_collapse_preserves_draft_filter_and_search_focus(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer.update(cx, |input, cx| {
                    input.set_value("Keep this draft", window, cx)
                });
                this.search
                    .update(cx, |input, cx| input.set_value("Alpha", window, cx));
            });
        });
        visual.run_until_parked();
        let expanded = visual.debug_bounds("session-sidebar").unwrap();
        let toggle = visual.debug_bounds("sidebar-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            px(56.)
        );
        assert!(expanded.size.width > px(56.));
        workspace.read_with(&visual, |this, cx| {
            assert!(this.sidebar_collapsed);
            assert_eq!(this.composer.read(cx).value().as_str(), "Keep this draft");
            assert_eq!(this.search.read(cx).value().as_str(), "Alpha");
            assert_eq!(this.visible_sessions(cx), vec![0]);
            assert_eq!(this.view.selected.as_deref(), Some("a"));
        });
        assert!(commands.try_recv().is_err());
        let search = visual.debug_bounds("sidebar-search").unwrap();
        visual.simulate_click(search.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.update(|window, cx| {
            workspace
                .read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        workspace.read_with(&visual, |this, cx| {
            assert!(!this.sidebar_collapsed);
            assert_eq!(this.search.read(cx).value().as_str(), "Alpha");
            assert_eq!(this.composer.read(cx).value().as_str(), "Keep this draft");
        });
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            expanded.size.width
        );
    }

    #[gpui::test]
    fn collapsed_sidebar_selects_the_explicit_session(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.sidebar_collapsed = true;
                cx.notify();
            })
        });
        visual.run_until_parked();
        let row = visual.debug_bounds("sidebar-session-1").unwrap();
        visual.simulate_click(row.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn chat_and_composer_share_centered_edges_with_compact_approval(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.sessions = Arc::new(vec![Session {
            id: "a".into(),
            label: "Alpha".into(),
            approval: Some(serde_json::json!({
                "toolName": "Bash", "toolInput": {"command": "cargo test"}
            })),
            ..Default::default()
        }]);
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: "Ready for review".into(),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        for (width, height) in [(1600., 900.), (1000., 700.), (720., 480.)] {
            visual.simulate_resize(size(px(width), px(height)));
            visual.run_until_parked();
            let chat = visual.debug_bounds("chat-content-column").unwrap();
            let composer = visual.debug_bounds("chat-composer").unwrap();
            assert!(
                (f32::from(chat.left() - composer.left())).abs() < 1.,
                "{width}x{height}: chat={chat:?}, composer={composer:?}"
            );
            assert!((f32::from(chat.right() - composer.right())).abs() < 1.);
            workspace.read_with(&visual, |this, _| {
                assert!(
                    (f32::from(chat.center().x - this.list.viewport_bounds().center().x)).abs()
                        < 1.
                );
                assert!(this.composer_dock_bounds.top() - this.header_bounds.bottom() > px(140.));
            });
        }
    }

    #[gpui::test]
    fn floating_composer_keeps_the_last_message_clear_without_shortening_the_viewport(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 30,
            first_seq: 1,
            items: (0..30)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nA paragraph in the conversation."),
                    ..Default::default()
                })
                .collect(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let last_row = visual.debug_bounds("last-transcript-row").unwrap();
        let initial_height = workspace.read_with(&visual, |this, _| {
            let dock = this.composer_dock_bounds;
            assert!(dock.size.height > px(60.));
            assert!(this.header_bounds.size.height > px(30.));
            assert!(this.list.viewport_bounds().top() < this.header_bounds.bottom());
            assert!(this.list.viewport_bounds().bottom() > dock.top() + px(60.));
            assert!(last_row.bottom() <= dock.top());
            dock.size.height
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("First line\nSecond line\nThird line\nFourth line");
        visual.run_until_parked();
        let last_row = visual.debug_bounds("last-transcript-row").unwrap();
        workspace.read_with(&visual, |this, _| {
            assert!(this.composer_dock_bounds.size.height > initial_height);
            assert!(last_row.bottom() <= this.composer_dock_bounds.top());
        });
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.run_until_parked();
        let last_row = visual.debug_bounds("last-transcript-row").unwrap();
        workspace.read_with(&visual, |this, _| {
            assert!(this.composer_dock_bounds.top() > px(180.));
            assert!(last_row.bottom() <= this.composer_dock_bounds.top());
        });
    }

    #[gpui::test]
    fn tool_cards_expand_without_sending_and_code_languages_are_highlighted(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: vec![
                Item {
                    kind: "tool_use".into(),
                    id: "call".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command":"printf hello"}),
                    ..Default::default()
                },
                Item {
                    kind: "tool_result".into(),
                    tool_use_id: "call".into(),
                    content: "hello".into(),
                    ..Default::default()
                },
            ],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let collapsed = visual.debug_bounds("last-transcript-row").unwrap();
        let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height
                > collapsed.size.height
        );
        assert!(commands.try_recv().is_err());
        let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height,
            collapsed.size.height
        );
        for (language, code) in [
            ("rust", "fn main() { let count = 42; }"),
            ("typescript", "const count: number = 42;"),
            ("diff", "@@ -1 +1 @@\n-old\n+new\n"),
        ] {
            let mut highlighter = gpui_component::highlighter::SyntaxHighlighter::new(language);
            highlighter.update(None, &gpui_component::input::Rope::from_str(code));
            let styles = highlighter.styles(
                &(0..code.len()),
                &gpui_component::highlighter::HighlightTheme::default_dark(),
            );
            assert!(
                !styles.is_empty(),
                "{language} must have real syntax highlighting"
            );
        }
    }

    #[gpui::test]
    fn open_tool_cards_survive_reseeds_and_late_results(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let call = Item {
            kind: "tool_use".into(),
            id: "stable-call".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command":"cargo test"}),
            ..Default::default()
        };
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![call.clone()],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let closed_height = visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height;
        let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        for _ in 0..3 {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    let mut view = (*this.view).clone();
                    view.transcript.snapshot(ConversationSnapshot {
                        seq: 3,
                        first_seq: 1,
                        items: vec![
                            Item {
                                kind: "assistant_text".into(),
                                text: "Running the checks.".into(),
                                ..Default::default()
                            },
                            call.clone(),
                            Item {
                                kind: "tool_result".into(),
                                tool_use_id: "stable-call".into(),
                                content: "All tests passed".into(),
                                ..Default::default()
                            },
                        ],
                    });
                    this.update_view(Arc::new(view), window, cx);
                })
            });
            visual.run_until_parked();
            assert!(
                visual
                    .debug_bounds("last-transcript-row")
                    .unwrap()
                    .size
                    .height
                    > closed_height
            );
            workspace.read_with(&visual, |this, _| {
                assert_eq!(this.tool_expansion.get("call:stable-call"), Some(&true))
            });
        }
        // A deliberate collapse must also survive the next server snapshot.
        let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![call],
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.tool_expansion.get("call:stable-call"), Some(&false))
        });
        assert_eq!(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height,
            closed_height
        );
    }

    #[gpui::test]
    fn reading_history_stays_anchored_through_updates_and_scroll_stop(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut items: Vec<Item> = (0..40)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nA paragraph to read without the view jumping."),
                ..Default::default()
            })
            .collect();
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 40,
            first_seq: 1,
            items: items.clone(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let dock_height =
            workspace.read_with(&visual, |this, _| this.composer_dock_bounds.size.height);
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(600.), px(300.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(350.))),
            ..Default::default()
        });
        visual.run_until_parked();
        let (tracked_row, y) = workspace.read_with(&visual, |this, _| {
            assert!(!this.follow);
            assert_eq!(this.composer_dock_bounds.size.height, dock_height);
            let anchor = this.list.logical_scroll_top();
            // Header notices can change which row intersects the logical top.
            // Track content fully below the header, whose screen position must
            // remain fixed even when that first logical row advances.
            let tracked_row = (anchor.item_ix + 2).min(this.list.item_count() - 1);
            (
                tracked_row,
                this.list.bounds_for_item(tracked_row).unwrap().top()
                    + this.header_bounds.size.height
                    + px(16.),
            )
        });
        for round in 0..3 {
            items[0].text.push_str(" Changed earlier content.");
            items.push(Item {
                kind: "assistant_text".into(),
                text: format!("New tail {round}"),
                ..Default::default()
            });
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    let mut next = (*this.view).clone();
                    next.notice = if round % 2 == 0 {
                        "Refreshing conversation".into()
                    } else {
                        String::new()
                    };
                    next.transcript.snapshot(ConversationSnapshot {
                        seq: items.len() as u64,
                        first_seq: 1,
                        items: items.clone(),
                    });
                    this.update_view(Arc::new(next), window, cx);
                })
            });
            visual.run_until_parked();
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(px(600.), px(300.)),
                touch_phase: gpui::TouchPhase::Ended,
                ..Default::default()
            });
            visual.run_until_parked();
            workspace.read_with(&visual, |this, _| {
                assert!(!this.follow);
                assert!(
                    (this.list.bounds_for_item(tracked_row).unwrap().top()
                        + this.header_bounds.size.height
                        + px(16.)
                        - y)
                        .abs()
                        < px(1.)
                );
                assert_eq!(this.composer_dock_bounds.size.height, dock_height);
            });
        }
    }

    #[gpui::test]
    fn scrolling_to_the_end_hides_jump_and_resumes_following(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 40,
            first_seq: 1,
            items: (0..40)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nA paragraph."),
                    ..Default::default()
                })
                .collect(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        for delta in [350., -100_000.] {
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(px(600.), px(300.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
                ..Default::default()
            });
            visual.run_until_parked();
            workspace.read_with(&visual, |this, _| {
                assert_eq!(
                    this.follow,
                    delta < 0.,
                    "delta={delta}, viewport={:?}, dock={:?}, tail={:?}, offset={:?}",
                    this.list.viewport_bounds(),
                    this.composer_dock_bounds,
                    this.list.bounds_for_item(39),
                    this.list.logical_scroll_top()
                )
            });
            assert_eq!(visual.debug_bounds("jump-latest").is_none(), delta < 0.);
        }
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = (*this.view).clone();
                next.transcript.delta(
                    wks_native::model::Delta {
                        seq: 41,
                        items: vec![Item {
                            kind: "assistant_text".into(),
                            text: "New reply".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    false,
                );
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.follow);
            assert_eq!(
                this.list.logical_scroll_top().item_ix,
                this.list.item_count()
            );
        });
    }

    #[test]
    fn working_badges_understand_daemon_modes_and_pending_states() {
        let p = Appearance::Dark.palette();
        let mut session = Session {
            state: "responding".into(),
            ..Default::default()
        };
        assert_eq!(session_status(&session, p).0, "Working");
        assert!(session.working());
        session.approval = Some(serde_json::json!({"toolName":"Bash"}));
        assert_eq!(session_status(&session, p).0, "Needs approval");
        assert!(!session.working());
        session.state = "stopped".into();
        assert_eq!(session_status(&session, p).0, "Ended");
        assert!(!session.working());
        session.approval = None;
        session.state = "input".into();
        assert_eq!(session_status(&session, p).0, "Ready");
        assert!(!session.working());
    }

    #[gpui::test]
    fn reaching_the_tail_after_layout_resumes_follow_without_an_extra_wheel_event(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 60,
            first_seq: 1,
            items: (0..60)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nSome content."),
                    ..Default::default()
                })
                .collect(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("ctrl-u");
        visual.run_until_parked();
        assert!(visual.debug_bounds("jump-latest").is_some());
        for _ in 0..3 {
            visual.simulate_keystrokes("ctrl-d");
            visual.run_until_parked();
        }
        let tail = visual.debug_bounds("last-transcript-row").unwrap();
        workspace.read_with(&visual, |this, _| {
            assert!(tail.bottom() <= this.composer_dock_bounds.top());
            assert!(
                this.follow,
                "visible bottom must resume follow even when no wheel callback runs"
            );
        });
        // GPUI 0.2.2 retains debug_bounds entries across reused frames, so an
        // old selector's presence cannot establish whether a button was drawn.
    }

    #[gpui::test]
    fn page_up_leaves_the_tail_and_repeated_pages_keep_moving(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 100,
            first_seq: 1,
            items: (0..100)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nSome content."),
                    ..Default::default()
                })
                .collect(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("ctrl-u");
        visual.run_until_parked();
        let first = workspace.read_with(&visual, |this, _| {
            assert!(!this.follow);
            this.list.logical_scroll_top().item_ix
        });
        assert!(first < 99);
        visual.simulate_keystrokes("ctrl-u");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.list.logical_scroll_top().item_ix < first)
        });
        let jump = visual.debug_bounds("jump-latest").unwrap();
        visual.simulate_click(jump.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.follow);
            assert_eq!(
                this.list.logical_scroll_top().item_ix,
                this.list.item_count()
            );
        });
    }

    #[gpui::test]
    fn reselecting_the_same_session_updates_list_even_when_revisions_collide(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                for count in [1, 4, 2] {
                    let mut view = state("a");
                    view.transcript.snapshot(ConversationSnapshot {
                        seq: count,
                        first_seq: 1,
                        items: (0..count)
                            .map(|_| Item {
                                kind: "assistant_text".into(),
                                text: "Fresh snapshot".into(),
                                ..Default::default()
                            })
                            .collect(),
                    });
                    assert_eq!(view.transcript.revision, 1);
                    this.update_view(Arc::new(view), window, cx);
                    assert_eq!(this.list.item_count(), count as usize);
                }
            })
        });
    }

    #[gpui::test]
    fn reading_position_survives_switch_and_marks_new_activity(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut a = state("a");
                a.transcript.snapshot(ConversationSnapshot {
                    seq: 50,
                    first_seq: 1,
                    items: (0..50)
                        .map(|i| Item {
                            kind: "user_message".into(),
                            text: format!("Message {i}"),
                            ..Default::default()
                        })
                        .collect(),
                });
                this.update_view(Arc::new(a.clone()), window, cx);
                this.follow = false;
                this.list.scroll_to(ListOffset {
                    item_ix: 12,
                    offset_in_item: px(7.),
                });
                this.capture_reading(window, cx);
                this.update_view(Arc::new(state("b")), window, cx);
                // Empty attach snapshots must not erase a saved bookmark.
                let mut loading = state("a");
                loading.loading = true;
                this.update_view(Arc::new(loading), window, cx);
                a.transcript.delta(
                    wks_native::model::Delta {
                        seq: 51,
                        items: vec![Item {
                            kind: "assistant_text".into(),
                            text: "New activity".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    true,
                );
                this.update_view(Arc::new(a), window, cx);
                assert!(!this.follow);
                assert_eq!(this.list.logical_scroll_top().item_ix, 12);
                assert_eq!(f32::from(this.list.logical_scroll_top().offset_in_item), 7.);
                assert_eq!(this.chat.unread, Some(50));
            });
        });
    }

    #[gpui::test]
    fn response_actions_preserve_drafts_and_validate_worker_ownership(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("My draft", window, cx));
                this.card_action(
                    "a",
                    &wks_native::transcript::CardAction::FillComposer {
                        label: "Continue".into(),
                        text: "Proposed follow-up".into(),
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    this.composer.read(cx).value().as_ref(),
                    "My draft\nProposed follow-up"
                );
                this.card_action(
                    "a",
                    &wks_native::transcript::CardAction::OpenWorker {
                        label: "Unrelated".into(),
                        session_id: "b".into(),
                    },
                    window,
                    cx,
                );
                assert!(this.local_notice.contains("not a live worker"));
            });
        });
        assert!(
            commands.try_recv().is_err(),
            "prefills do not send and unrelated workers do not open"
        );
    }

    #[gpui::test]
    fn an_explicit_target_never_sends_to_the_default_session(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.open_session(Some("b".into()));
                this.composer.update(cx, |input, cx| {
                    input.set_value("targeted message", window, cx)
                });
                this.update_view(Arc::new(state("a")), window, cx);
                this.send(&SendMessage, window, cx);
                assert!(this.view.selected.is_none());
            })
        });
        assert!(matches!(commands.try_recv().unwrap(),Command::Select(id) if id == "b"));
        assert!(commands.try_recv().is_err());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("b")), window, cx);
                this.composer.update(cx, |input, cx| {
                    input.set_value("still targeted", window, cx)
                });
                let mut disappeared = state("a");
                disappeared.sessions = Arc::new(vec![disappeared.sessions[0].clone()]);
                this.update_view(Arc::new(disappeared), window, cx);
                this.send(&SendMessage, window, cx);
                assert!(!this.view.connected);
                assert_eq!(this.view.selected.as_deref(), Some("b"));
            })
        });
        assert!(commands.try_recv().is_err());
    }
    #[gpui::test]
    fn model_picker_keyboard_selection_and_provider_reset(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                this.project
                    .update(cx, |input, cx| input.set_value("/work/project", window, cx));
                let mut loaded = state("a");
                loaded.catalog = wks_native::launch::Catalog {
                    key: CatalogKey {
                        provider: "claude".into(),
                        cwd: String::new(),
                    },
                    models: vec![ModelChoice {
                        id: "opus".into(),
                        label: "Opus".into(),
                        windows: vec![200000, 1000000],
                    }],
                    ..Default::default()
                };
                this.update_view(Arc::new(loaded), window, cx);
                this.model_picker
                    .update(cx, |picker, cx| picker.focus(window, cx));
            });
        });
        visual.simulate_keystrokes("enter down enter");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.model_choice, "opus");
            assert_eq!(this.context_window, Some(200000));
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.context_window = Some(1000000);
                this.permission = Permission::Plan;
                this.create(cx);
                this.spawn_pending = false;
                this.choose_provider("codex", window, cx);
                assert!(this.model_choice.is_empty());
                assert_eq!(this.context_window, None);
                assert_eq!(this.permission, Permission::Ask);
                assert!(this.catalog_models.is_empty());
                this.model_choice = "__custom".into();
                this.create(cx);
                assert!(this.spawn_error.contains("Enter a custom model"));
            });
        });
        let requests: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::Create(request) => Some(request),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 1);
        let params = requests[0].params().unwrap();
        assert_eq!(params["model"], "opus");
        assert_eq!(params["contextWindow"], 1000000);
        assert_eq!(params["permissionMode"], "plan");
        assert_eq!(params["skipPermissions"], false);
    }
    #[gpui::test]
    fn secondary_views_preserve_drafts_and_only_request_reads(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("unsent work", window, cx));
                this.open_feature(Screen::Changes, window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "unsent work");
                this.open_feature(Screen::Session, window, cx);
                this.show_screen(Screen::Conversation, window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "unsent work");
            })
        });
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::Request(wks_native::features::Request::Changes { .. })
        ));
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn attachment_receipts_preserve_new_drafts_and_other_sessions(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.extras.attachments.insert(
                    "a".into(),
                    vec![("screen.png".into(), "/remote/screen.png".into())],
                );
                this.composer
                    .update(cx, |input, cx| input.set_value("inspect this", window, cx));
                this.send(&SendMessage, window, cx);
                this.update_view(Arc::new(state("b")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("other draft", window, cx));
                let mut next = state("b");
                next.receipt = Some(Receipt {
                    number: 1,
                    session: "a".into(),
                    action: Action::Send("[Image: /remote/screen.png]\ninspect this".into()),
                    error: None,
                });
                this.update_view(Arc::new(next), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "other draft");
                assert!(!this.drafts.contains_key("a"));
                assert!(this.extras.attachments["a"].is_empty());
            })
        });
        assert!(
            matches!(commands.try_recv().unwrap(), Command::Act { session, action:Action::Send(text) } if session == "a" && text.contains("[Image: /remote/screen.png]"))
        );
    }

    #[gpui::test]
    fn failed_attachment_send_keeps_original_draft_and_file(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.extras.attachments.insert(
                    "a".into(),
                    vec![("screen.png".into(), "/remote/screen.png".into())],
                );
                this.composer
                    .update(cx, |input, cx| input.set_value("inspect this", window, cx));
                this.send(&SendMessage, window, cx);
                let mut next = state("a");
                next.receipt = Some(Receipt {
                    number: 1,
                    session: "a".into(),
                    action: Action::Send("[Image: /remote/screen.png]\ninspect this".into()),
                    error: Some("Disconnected; outcome unknown".into()),
                });
                this.update_view(Arc::new(next), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "inspect this");
                assert_eq!(this.extras.attachments["a"].len(), 1);
            })
        });
    }

    #[gpui::test]
    fn archived_sessions_are_filtered_per_connection(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.settings
                    .archived
                    .insert("test".into(), vec!["a".into()]);
                assert_eq!(this.visible_sessions(cx), vec![1]);
                this.project_scope = "another-hub".into();
                assert_eq!(this.visible_sessions(cx), vec![0, 1]);
            })
        });
    }
    #[gpui::test]
    fn setup_round_trip_preserves_resume_and_model_picker_tracks_actual_choice(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                let old = Session {
                    id: "past".into(),
                    provider: "codex".into(),
                    cwd: "/project".into(),
                    state: "stopped".into(),
                    ..Default::default()
                };
                this.resume_session(&old, window, cx);
                this.open_feature(Screen::Setup, window, cx);
                this.back_from_feature(window, cx);
                assert!(this.new_session);
                assert_eq!(this.extras.resume.as_deref(), Some("past"));
                this.create(cx);
                this.spawn_pending = false;
                this.new_session = false;
                this.open_feature(Screen::Model, window, cx);
                assert_eq!(
                    this.model_picker
                        .read(cx)
                        .selected_value()
                        .map(String::as_str),
                    Some("__custom")
                );
            })
        });
        let mut resumed = false;
        while let Ok(command) = commands.try_recv() {
            if let Command::Create(request) = command {
                assert_eq!(request.resume_session_id.as_deref(), Some("past"));
                resumed = true;
            }
        }
        assert!(resumed);
    }
    #[gpui::test]
    fn question_choices_preserve_punctuation_and_custom_answers(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| workspace.update(cx, |this, cx| {
            let mut next = state("a");
            Arc::make_mut(&mut next.sessions)[0].questions = Some(serde_json::json!([{"question":"Which?", "options":[{"label":"Yes, please"},{"label":"No"}],"multiSelect":true}]));
            this.update_view(Arc::new(next), window, cx);
            this.extras.selected_options[0].insert(0);
            assert_eq!(this.question_answers(cx), vec!["Yes, please"]);
            this.extras.selected_options[0].remove(&0);
            assert_eq!(this.question_answers(cx), vec![""]);
            this.extras.answers[0].update(cx, |input, cx| input.set_value("Another approach", window, cx));
            assert_eq!(this.question_answers(cx), vec!["Another approach"]);
        }));
    }
    #[gpui::test]
    fn unsupported_provider_is_never_silently_resumed_as_claude(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let old = Session {
                    id: "other".into(),
                    provider: "pi".into(),
                    state: "stopped".into(),
                    ..Default::default()
                };
                this.resume_session(&old, window, cx);
                assert!(!this.new_session);
                assert!(this.extras.notice.contains("cannot be resumed"));
            })
        });
        assert!(commands.try_recv().is_err());
    }
    #[gpui::test]
    fn model_and_resume_keep_the_requested_context_pair(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut next = state("a");
            let session = &mut Arc::make_mut(&mut next.sessions)[0];
            session.merge(&serde_json::json!({"provider":"claude","requestedSelection":{"model":"opus","contextWindow":1000000}}));
            let session = session.clone();
            this.update_view(Arc::new(next), window, cx);
            this.open_feature(Screen::Model, window, cx);
            assert_eq!(this.context_window, Some(1000000));
            assert_eq!(this.model.read(cx).value().as_ref(), "opus");
            this.resume_session(&session, window, cx);
            assert_eq!(this.context_window, Some(1000000));
            assert_eq!(this.model.read(cx).value().as_ref(), "opus");
            this.resume_session(&Session { provider:"claude".into(), id:"old".into(), cwd:"/project".into(), ..Default::default() }, window, cx);
            assert!(this.model_choice.is_empty());
            assert_eq!(this.context_window, None);
        }));
    }
    #[gpui::test]
    fn normal_text_paste_still_reaches_the_composer(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                cx.write_to_clipboard(ClipboardItem::new_string("ordinary paste".into()));
                this.composer
                    .update(cx, |input, cx| input.focus(window, cx));
            })
        });
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-v"
        } else {
            "ctrl-v"
        });
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "ordinary paste")
        });
        assert!(commands.try_recv().is_err());
    }
    #[gpui::test]
    fn terminal_bus_request_is_visible_unsupported_and_never_creates_hidden_work(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window,cx|workspace.update(cx,|this,cx|{
            let data=serde_json::json!({"cwd":"/project","command":"echo requested","label":"Checks","parentSessionId":"manager"});
            let(intent,payload)=wks_native::ui_requests::parse("facade.openTerminal",&data).unwrap().unwrap();
            let mut next=state("a");next.ui_requests.push(wks_native::ui_requests::Request{number:1,intent,payload});
            this.update_view(Arc::new(next),window,cx);
            assert!(this.ui_bus.notice.contains("Terminal panes are unavailable"));
            assert_eq!(this.screen,Screen::Conversation);
        }));
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ConsumeUiRequest(1)
        ));
        assert!(commands.try_recv().is_err());
    }
    #[gpui::test]
    fn spawn_dialog_bus_request_only_prefills_and_decision_actions_are_refused(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut next = state("a");
                next.ui_requests.push(wks_native::ui_requests::Request {
                    number: 1,
                    intent: wks_native::ui_requests::Intent::OpenSpawnDialog {
                        cwd: "/requested/project".into(),
                    },
                    payload: serde_json::json!({"cwd":"/requested/project"}),
                });
                this.update_view(Arc::new(next), window, cx);
                assert!(this.new_session);
                assert_eq!(this.project.read(cx).value().as_ref(), "/requested/project");
                let mut next = state("a");
                next.ui_requests.push(wks_native::ui_requests::Request {
                    number: 2,
                    intent: wks_native::ui_requests::Intent::RunAction {
                        action: "fleet-approve-yes".into(),
                        digit: None,
                    },
                    payload: serde_json::json!({"action":"fleet-approve-yes"}),
                });
                this.update_view(Arc::new(next), window, cx);
                assert!(this.ui_bus.notice.contains("scoped controls"));
            })
        });
        while let Ok(command) = commands.try_recv() {
            assert!(matches!(
                command,
                Command::ConsumeUiRequest(_) | Command::LoadModels { .. }
            ));
        }
    }
    #[gpui::test]
    fn focus_bus_request_preserves_pinned_window_and_reviews_keep_explicit_cwd(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.requested_session = Some("a".into());
                let mut next = state("a");
                next.ui_requests.push(wks_native::ui_requests::Request {
                    number: 1,
                    intent: wks_native::ui_requests::Intent::FocusAgent("b".into()),
                    payload: serde_json::json!({"sessionId":"b"}),
                });
                this.update_view(Arc::new(next), window, cx);
                assert!(this.ui_bus.notice.contains("pinned"));
                this.requested_session = None;
                let mut next = state("a");
                next.ui_requests.push(wks_native::ui_requests::Request {
                    number: 2,
                    intent: wks_native::ui_requests::Intent::OpenPane {
                        pane_type: "review".into(),
                        cwd: "/different/project".into(),
                        url: String::new(),
                    },
                    payload: serde_json::json!({"paneType":"review","cwd":"/different/project"}),
                });
                this.update_view(Arc::new(next), window, cx);
                assert_eq!(this.screen, Screen::Changes);
            })
        });
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ConsumeUiRequest(1)
        ));
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ConsumeUiRequest(2)
        ));
        assert!(
            matches!(commands.try_recv().unwrap(),Command::Request(wks_native::features::Request::Changes{cwd}) if cwd=="/different/project")
        );
        assert!(commands.try_recv().is_err());
    }
    #[gpui::test]
    fn power_pause_waits_for_an_explicit_connection_control(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.connected = false;
                next.power_paused = true;
                next.power_pause_generation = 42;
                next.can_resume_power_pause = true;
                next.notice = "Server requested a reconnect pause.".into();
                this.update_view(Arc::new(next), window, cx);
                assert!(this.view.power_paused);
            })
        });
        assert!(commands.try_recv().is_err());
        visual.simulate_keystrokes("ctrl-r");
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ResumePowerPause(42)
        ));
        assert!(commands.try_recv().is_err());
    }
}
