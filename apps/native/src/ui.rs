mod features;
mod launch;
mod navigation;
mod transcript;
use gpui::{
    App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    ListAlignment, ListOffset, ListScrollEvent, ListState, Render, SharedString, Stateful, Task,
    Window, actions, div, list, prelude::*, px, rgb, uniform_list,
};
use gpui_component::{
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

const CHAT_WIDTH: f32 = 900.;

pub fn configure_theme(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
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
    theme.colors.input = rgb(p.muted).into();
    theme.colors.primary = rgb(p.primary).into();
    theme.colors.primary_foreground = rgb(p.on_primary).into();
    theme.colors.caret = rgb(p.accent).into();
    theme.colors.link = rgb(p.accent).into();
    theme.colors.muted = rgb(p.surface).into();
    theme.colors.muted_foreground = rgb(p.muted).into();
    theme.colors.selection = rgb(p.selected).into();
    theme.font_size = px(14.);
    theme.radius = px(8.);
}

fn mono_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
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
    if session.approval.is_some() {
        return ("Needs approval", p.warning);
    }
    if session.questions.is_some() {
        return ("Needs your input", p.warning);
    }
    match session.state.as_str() {
        "working" | "thinking" | "streaming" => ("Working", p.busy),
        "input" | "idle" => ("Ready", p.success),
        "stopped" | "ended" => ("Ended", p.muted),
        "" => ("Session", p.muted),
        state => (state, p.muted),
    }
}

fn session_badge(session: &Session, p: Palette) -> Div {
    let (label, color) = session_status(session, p);
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(status_dot(color))
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
    extras: features::Extras,
    chat: transcript::ChatUi,
    screen: Screen,
    settings: Settings,
    settings_path: Option<std::path::PathBuf>,
    project_scope: String,
    settings_error: String,
    project_filter: Option<String>,
    project_cursor: usize,
    project_path: Entity<InputState>,
    search: Entity<InputState>,
    navigation_selected: Option<String>,
    sidebar_scroll: gpui::UniformListScrollHandle,
    projects_scroll: gpui::UniformListScrollHandle,
    _focus_watch: Vec<gpui::Subscription>,
    appearance: Appearance,
    theme_error: String,
    controller: Controller,
    view: Arc<View>,
    composer: Entity<InputState>,
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
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(3)
                .placeholder("What would you like to work on?")
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
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions or projects…"));
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
            screen: Screen::Conversation,
            settings: Settings::default(),
            settings_path: None,
            project_scope: "test".into(),
            settings_error: String::new(),
            project_filter: None,
            project_cursor: 0,
            project_path,
            search,
            navigation_selected: None,
            sidebar_scroll: gpui::UniformListScrollHandle::new(),
            projects_scroll: gpui::UniformListScrollHandle::new(),
            _focus_watch: focus_watch,
            appearance: Appearance::default(),
            theme_error: String::new(),
            controller,
            view: Arc::new(View::default()),
            composer,
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
                    ..(*self.view).clone()
                });
                cx.notify();
                return;
            }
        }
        self.local_notice.clear();
        if self.view.selected != view.selected {
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
            let old = &self.view.transcript.rows;
            let new = &view.transcript.rows;
            let first = old
                .iter()
                .zip(new)
                .position(|(a, b)| !Arc::ptr_eq(a, b))
                .unwrap_or(old.len().min(new.len()));
            // Invalidate only changed measurements. Retain the scroll anchor
            // while reading history; bottom alignment follows when at the tail.
            // Revisions restart after a reconnect/reselection. Row identities
            // and count remain authoritative even if two revisions collide.
            let changed = first < old.len() || first < new.len();
            if changed {
                self.list.splice(first..old.len(), new.len() - first);
                if !self.follow {
                    self.chat.restored = false;
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
        self.view = view;
        self.restore_reading(window, cx);
        if self.new_session || self.screen == Screen::Model {
            self.sync_models(window, cx);
            if reconnected {
                self.load_models(true, cx);
            }
        }
        cx.notify();
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
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
        if self.demo || self.requested_session.is_some() {
            return;
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
        let p = self.appearance.palette();
        let id = id.into();
        let label = label.into();
        let primary = matches!(
            id.as_ref(),
            "send" | "create-session" | "new-session" | "welcome-new"
        );
        div()
            .id(id.clone())
            .px_3()
            .py_2()
            .font_weight(FontWeight::MEDIUM)
            .rounded_md()
            .bg(rgb(p.surface))
            .text_color(rgb(if enabled { p.text } else { p.disabled }))
            .text_size(px(12.))
            .when(enabled, |d| {
                d.cursor_pointer().hover(|s| {
                    s.bg(if primary {
                        gpui::Hsla::from(rgb(p.primary)).opacity(0.85)
                    } else {
                        rgb(p.selected).into()
                    })
                })
            })
            .when(
                enabled
                    && matches!(
                        id.as_ref(),
                        "send" | "create-session" | "new-session" | "welcome-new"
                    ),
                |d| d.bg(rgb(p.primary)).text_color(rgb(p.on_primary)),
            )
            .child(label)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.appearance.palette();
        let compact = window.viewport_size().height < px(620.);
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
        let title = selected
            .as_ref()
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Your sessions".into());
        let notice = if !self.local_notice.is_empty() {
            self.local_notice.clone()
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
        .min_h_0();

        let visible_sessions = self.visible_sessions(cx);
        let sidebar = div()
            .w(px(264.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .border_r_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_5()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .bg(rgb(p.selected))
                            .child(brand_mark(22., p)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .text_size(px(14.))
                                    .font_family(mono_font())
                                    .font_weight(FontWeight::BOLD)
                                    .child("work")
                                    .child(div().text_color(rgb(p.accent)).child("{spacer}")),
                            )
                            .child(div().text_size(px(10.)).text_color(rgb(p.muted)).child(
                                if self.demo {
                                    "NATIVE · DEMO"
                                } else {
                                    "YOUR AGENT WORKSPACE"
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pb_2()
                    .flex()
                    .gap_1()
                    .child(
                        self.button("nav-projects", "Projects", true)
                            .flex_1()
                            .when(self.screen == Screen::Projects, |d| d.bg(rgb(p.selected)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_screen(Screen::Projects, window, cx)
                            })),
                    )
                    .child(
                        self.button("nav-settings", "Settings", true)
                            .flex_1()
                            .when(self.screen == Screen::Settings, |d| d.bg(rgb(p.selected)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_screen(Screen::Settings, window, cx)
                            })),
                    ),
            )
            .child(
                div().px_3().pb_2().child(
                    self.button("nav-history", "Session history", true)
                        .w_full()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Recent, window, cx)
                        })),
                ),
            )
            .child(div().px_3().pb_3().child(Input::new(&self.search)))
            .child(
                div().px_3().pb_4().child(
                    self.button(
                        "new-session",
                        "New session",
                        !self.demo && self.requested_session.is_none(),
                    )
                    .w_full()
                    .flex()
                    .justify_between()
                    .child(keycap(
                        if cfg!(target_os = "macos") {
                            "⌘ N"
                        } else {
                            "Ctrl N"
                        },
                        p,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx))),
                ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .flex()
                    .justify_between()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child("SESSIONS")
                    .child(visible_sessions.len().to_string()),
            )
            .when_some(self.project_filter.as_ref(), |d, path| {
                d.child(
                    div()
                        .px_3()
                        .pb_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(p.accent))
                                .child(path.clone()),
                        )
                        .child(
                            self.button("all-projects", "All", true)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.project_filter = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                uniform_list(
                    "sessions",
                    visible_sessions.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                let session = &this.view.sessions[visible_sessions[ix]];
                                let id = session.id.clone();
                                let active = this
                                    .navigation_selected
                                    .as_ref()
                                    .or(this.view.selected.as_ref())
                                    == Some(&id);
                                div().h(px(80.)).px_2().pb_1().child(
                                    div()
                                        .id(ix)
                                        .h_full()
                                        .px_3()
                                        .py_2()
                                        .rounded_lg()
                                        .cursor_pointer()
                                        .overflow_hidden()
                                        .when(active, |d| d.bg(rgb(p.selected)))
                                        .hover(|style| style.bg(rgb(p.selected)))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div().w(px(3.)).h(px(14.)).rounded_full().bg(
                                                        rgb(if active {
                                                            p.accent
                                                        } else {
                                                            p.border
                                                        }),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .truncate()
                                                        .child(this.session_title(session)),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .pl_3()
                                                .text_size(px(11.))
                                                .text_color(rgb(p.muted))
                                                .truncate()
                                                .child(session.cwd.clone()),
                                        )
                                        .child(div().pl_3().mt_1().child(session_badge(session, p)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.new_session = false;
                                            this.screen = Screen::Conversation;
                                            this.command(Command::Select(id.clone()), cx)
                                        })),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.sidebar_scroll.clone())
                .flex_1()
                .min_h_0(),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(keycap(
                        if !self.settings.vim_navigation {
                            "SHORTCUTS"
                        } else if self.focus.is_focused(window) {
                            "NORMAL"
                        } else {
                            "INSERT"
                        },
                        p,
                    ))
                    .child(div().text_size(px(10.)).text_color(rgb(p.muted)).child(
                        if !self.settings.vim_navigation {
                            "Ctrl P · Ctrl ,"
                        } else if self.focus.is_focused(window) {
                            "i edit · g p projects"
                        } else {
                            "Esc to navigate"
                        },
                    )),
            )
            .child(
                div()
                    .p_4()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_dot(if self.view.connected {
                        p.success
                    } else {
                        p.warning
                    }))
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(if self.view.connected {
                        "Connected to hub"
                    } else {
                        "Reconnecting…"
                    }),
            );

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
                                        .child("Choose an agent, point it at a project, and make something great."),
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

        self.shell(window, cx)
            .child(sidebar)
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().bg(rgb(p.chat))
                .child(div().px_5().py(px(if compact { 10. } else { 16. })).bg(rgb(p.base)).border_b_1().border_color(rgb(p.border)).flex().items_center().gap_3().justify_between()
                    .child(div().flex_1().min_w_0().when(!compact, |d| d.child(overline("CONVERSATION", p))).child(div().truncate().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title)).child(div().truncate().text_size(px(11.)).text_color(rgb(p.muted)).child(selected.as_ref().map(|s| s.cwd.clone()).unwrap_or_else(|| "Your workspace, ready when you are".into()))))
                    .when_some(selected.as_ref(), |d, session| d.child(session_badge(session, p)))
                    .child(self.button("refresh", "Refresh", true).on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))))
                .child(div().px_5().py_2().flex().flex_wrap().gap_2()
                    .child(self.button("open-changes", "Changes", selected.is_some()).when(selected.is_some(), |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Changes, window, cx)))))
                    .child(self.button("open-history", "History", selected.is_some()).when(selected.is_some(), |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::History, window, cx)))))
                    .child(self.button("open-session", "Session…", selected.is_some()).when(selected.is_some(), |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Session, window, cx)))))
                    .child(self.button("open-model", "Model…", enabled && self.supported_session()).when(enabled && self.supported_session(), |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Model, window, cx))))))
                .when(!self.extras.notice.is_empty(), |d| d.child(div().px_5().text_color(rgb(p.warning)).child(self.extras.notice.clone())))
                .when(!notice.is_empty(), |d| d.child(div().px_5().py_2().text_size(px(12.)).text_color(rgb(p.warning)).child(notice)))
                .when(self.view.transcript.omitted, |d| d.child(div().px_5().text_size(px(11.)).text_color(rgb(p.muted)).child("Showing recent messages. Open History to browse older retained messages.")))
                .when(self.view.loading, |d| d.child(div().px_5().text_color(rgb(p.muted)).child("Loading conversation…")))
                .when(!self.view.loading && self.view.transcript.rows.is_empty(), |d| d.child(
                    div().flex_1().min_h_0().flex().flex_col().items_center().justify_center().px_5().gap_4()
                        .child(div().size(px(80.)).rounded(px(20.)).bg(rgb(p.surface)).flex().items_center().justify_center().child(brand_mark(40., p)))
                        .child(div().text_center().text_size(px(28.)).font_weight(FontWeight::BOLD).child("A little space for big ideas."))
                        .child(div().max_w(px(380.)).text_center().text_size(px(14.)).text_color(rgb(p.muted)).child(
                            if self.view.sessions.is_empty() {
                                if self.view.connected { "Start a session to bring your next idea to life." }
                                else { "Connecting to your workspace. Your sessions will appear here when the hub is ready." }
                            } else { "Ask a question, explore your code, or describe what you want to build." }))
                        .when(self.view.sessions.is_empty(), |d| d.child(self.button("welcome-setup", "Set up an agent", self.view.connected).when(self.view.connected, |d| d.on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Setup, window, cx))))))
                        .when(self.view.sessions.is_empty(), |d| d.child(self.button("welcome-new", "Start a session", self.view.connected && !self.demo && self.requested_session.is_none())
                            .when(self.view.connected && !self.demo && self.requested_session.is_none(), |d| d.on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx))))))
                ))
                .when(!self.view.transcript.rows.is_empty() || self.view.loading, |d| d.child(transcript))
                .when_some(self.chat.unread, |d, ix| d.child(self.button("new-activity", "New activity · jump to first unread", true).on_click(cx.listener(move |this, _, _, cx| {
                    this.follow = false;
                    this.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) }); cx.notify();
                }))))
                .child(self.render_pending(window, cx))
                .when(!self.follow, |d| d.child(self.button("latest", "Jump to latest", true).on_click(cx.listener(|this, _, window, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset { item_ix: this.view.transcript.rows.len(), offset_in_item: px(0.) }); this.chat.unread = None; this.capture_reading(window, cx); cx.notify();
                }))))
                .when_some(selected.as_ref().and_then(|s| s.approval.as_ref()), |d, approval| {
                    let label = approval.get("toolName").or_else(|| approval.get("tool")).and_then(serde_json::Value::as_str).unwrap_or("Tool");
                    let summary = approval.pointer("/toolInput/command").or_else(|| approval.pointer("/toolInput/file_path")).and_then(serde_json::Value::as_str).unwrap_or("").lines().next().unwrap_or("").to_owned();
                    let details = serde_json::to_string_pretty(approval.get("toolInput").or_else(|| approval.get("raw")).unwrap_or(approval)).unwrap_or_default();
                    d.child(div().w_full().max_w(px(CHAT_WIDTH + 40.)).mx_auto().px_5().py_2().flex().flex_col().gap_2().text_size(px(12.))
                        .child(div().flex().items_center().gap_2().text_color(rgb(p.warning)).child(status_dot(p.warning)).child(format!("Permission needed · {label}")))
                        .when(compact, |d| d.child(div().flex().items_center().gap_2().child(div().flex_1().min_w_0().truncate().font_family(mono_font()).child(summary)).child(self.button("approval-toggle", if self.extras.approval_details { "Hide details" } else { "Details" }, true).on_click(cx.listener(|this, _, _, cx| { this.extras.approval_details = !this.extras.approval_details; cx.notify(); })))))
                        .when(!compact || self.extras.approval_details, |d| d.child(div().id("approval-details").max_h(px(if compact { 52. } else { 120. })).overflow_y_scroll().p_3().rounded_md().bg(rgb(p.surface)).font_family(mono_font()).text_color(rgb(p.muted)).child(details)))
                        .child(div().flex().gap_2()
                            .child(self.button("approve", "Allow once", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(true), cx)))))
                            .child(self.button("deny", "Deny", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(false), cx)))))))
                })
                .when_some(selected.as_ref().filter(|s| s.questions.is_some()), |d, session| d.child(self.render_questions(session, enabled, compact, cx)))
                .when(selected.is_some(), |d| d.child(div().w_full().max_w(px(CHAT_WIDTH + 40.)).mx_auto().px_5().pt_3().pb_4().flex().flex_col().gap_2()
                    .child(div().bg(rgb(p.surface)).rounded_lg().p_3().flex().flex_col().gap_2()
                        .when(!compact || self.view.selected.as_ref().and_then(|id| self.extras.attachments.get(id)).is_some_and(|v| !v.is_empty()), |d| d.child(div().flex().flex_wrap().gap_2()
                            .child(self.button("attach-file", if self.uploading() { "Attaching…" } else { "Attach…" }, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, window, cx| this.pick_attachment(window, cx)))))
                            .when(!compact, |d| d.child(self.button("paste-image", "Paste image", enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, _, cx| { if !this.paste_image(cx) { this.extras.notice = "No supported image on the clipboard.".into(); cx.notify(); } })))))
                            .children(self.view.selected.as_ref().and_then(|id| self.extras.attachments.get(id)).into_iter().flatten().enumerate().map(|(ix, (name, _))| {
                                div().id(("attachment", ix)).px_2().py_1().rounded_md().bg(rgb(p.selected)).child(name.clone())
                                    .child(self.button("remove-attachment", "Remove", !self.view.busy).when(!self.view.busy, |d| d.on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(id) = &this.view.selected && let Some(files) = this.extras.attachments.get_mut(id) && ix < files.len() { files.remove(ix); } cx.notify();
                                    }))))
                            }))))
                        .child(Input::new(&self.composer).appearance(false).h(px(if compact { 48. } else { 88. })).disabled(self.view.busy))
                        .child(div().flex().items_center().justify_between().gap_2()
                            .when(compact, |d| d.child(self.button("compact-attach", if self.uploading() { "Attaching…" } else { "Attach…" }, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, window, cx| this.pick_attachment(window, cx))))))
                            .when(!compact, |d| d.child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(if self.view.busy { "Sending your message…" } else if enabled { "Make it happen." } else { "Select an available session to compose" })))
                            .child(div().flex().gap_2()
                                .child(self.button("stop", "Interrupt", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Stop, cx)))))
                                .child(self.button("send", if self.view.busy {"Sending…"} else {"Send message"}, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx))))))))
                    .child(div().flex().items_center().justify_between().text_size(px(10.)).text_color(rgb(p.muted))
                        .child(div().flex().gap_1().items_center().child(keycap(if cfg!(target_os = "macos") { "⌘ Enter" } else { "Ctrl Enter" }, p)).child("to send"))
                        .child("Enter for a new line")))))
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
}
