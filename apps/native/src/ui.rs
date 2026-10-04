mod bus_commands;
mod children;
mod chrome;
mod features;
mod file_viewer;
mod launch;
mod markdown;
mod navigation;
mod projects;
mod remote;
mod scroll;
mod settings;
mod sidebar;
mod smooth_scroll;
mod states;
mod syntax;
mod tools;
mod transcript;
mod typography;
mod updater;
mod usage;
mod window_destroy;
mod work;
use chrome::ControlTextStyle;
pub(crate) use chrome::custom_caption;
use gpui::{
    Animation, AnimationExt, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, KeyBinding, ListAlignment, ListOffset, ListScrollEvent, ListState, Render,
    SharedString, Stateful, Task, Window, actions, canvas, div, list, prelude::*, rgb,
    uniform_list,
};

/// Interface size multiplier on top of the OS display scale (which GPUI
/// already applies): 1.0 = 100%. Every literal size in the UI goes through
/// [`px`], and gpui-component's rem-based widgets follow the theme font size,
/// which is set through `px` too, so one factor zooms the whole app.
#[cfg_attr(all(test, feature = "ui-tests"), allow(dead_code))]
static ZOOM_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

// GPUI tests run concurrently, one per thread. A process-wide zoom set by
// one test would resize every other test's layout mid-assertion, so UI-test
// builds keep it per thread (each test's whole GPUI app lives on one).
#[cfg(all(test, feature = "ui-tests"))]
thread_local! {
    static TEST_ZOOM: std::cell::Cell<f32> = const { std::cell::Cell::new(1.) };
}

pub(crate) fn zoom() -> f32 {
    #[cfg(all(test, feature = "ui-tests"))]
    return TEST_ZOOM.get();
    #[cfg(not(all(test, feature = "ui-tests")))]
    f32::from_bits(ZOOM_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

pub(crate) fn set_zoom(zoom: f32) {
    let zoom = if zoom.is_finite() {
        zoom.clamp(0.5, 2.5)
    } else {
        1.
    };
    #[cfg(all(test, feature = "ui-tests"))]
    TEST_ZOOM.set(zoom);
    #[cfg(not(all(test, feature = "ui-tests")))]
    ZOOM_BITS.store(zoom.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Logical pixels at the current interface size. Shadows `gpui::px` for the
/// whole UI module tree (`use super::*`).
pub(crate) fn px(value: f32) -> gpui::Pixels {
    gpui::px(value * zoom())
}

/// Convert measured logical pixels (mouse, viewport) back to unzoomed units,
/// for values stored in settings.
pub(crate) fn unzoom(value: gpui::Pixels) -> f32 {
    f32::from(value) / zoom()
}
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
use wks_native::navigation::{Provider, Settings};
use wks_native::projects::KnownProject;
use wks_native::timing::{self, TurnClock};

const CHAT_WIDTH: f32 = 900.;

/// Bundled provider marks layered over gpui-component's icon set.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        use std::borrow::Cow;
        match path {
            "brand/claude.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/brand/claude.svg"
            )))),
            "brand/openai.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/brand/openai.svg"
            )))),
            _ => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
    }
}

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
    theme.highlight_theme = syntax::highlight_theme(appearance);
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
        .text_size(px(chrome::scale::OVERLINE))
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
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ViewerTab,
        ViewerToggleSource,
        ViewerFind,
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
        KeyBinding::new("ctrl-=", ZoomIn, Some("Workspace")),
        KeyBinding::new("ctrl-+", ZoomIn, Some("Workspace")),
        KeyBinding::new("ctrl--", ZoomOut, Some("Workspace")),
        KeyBinding::new("ctrl-0", ZoomReset, Some("Workspace")),
        KeyBinding::new("cmd-=", ZoomIn, Some("Workspace")),
        KeyBinding::new("cmd-+", ZoomIn, Some("Workspace")),
        KeyBinding::new("cmd--", ZoomOut, Some("Workspace")),
        KeyBinding::new("cmd-0", ZoomReset, Some("Workspace")),
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
        // The file viewer is modal: Tab must not walk focus out to the
        // covered composer or sidebar.
        KeyBinding::new("tab", ViewerTab, Some("FileViewer")),
        KeyBinding::new("shift-tab", ViewerTab, Some("FileViewer")),
        // Markdown documents: rendered preview ⇄ source, and Ctrl+F from
        // the preview searches the source (the editor binds its own Ctrl+F).
        KeyBinding::new("ctrl-shift-v", ViewerToggleSource, Some("FileViewer")),
        KeyBinding::new("cmd-shift-v", ViewerToggleSource, Some("FileViewer")),
        KeyBinding::new("ctrl-f", ViewerFind, Some("FileViewer")),
        KeyBinding::new("cmd-f", ViewerFind, Some("FileViewer")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-shift-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub struct Workspace {
    ui_bus: bus_commands::UiState,
    extras: features::Extras,
    remote: remote::RemoteUi,
    chat: transcript::ChatUi,
    child_ui: children::ChildUi,
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
    settings_search: Entity<InputState>,
    settings_section: settings::SettingsSection,
    usage_open: bool,
    smooth_scroll: smooth_scroll::SmoothScroll,
    navigation_selected: Option<String>,
    sidebar_collapsed: bool,
    sidebar_drag: Option<(gpui::Pixels, f32)>,
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
    launch_details_open: bool,
    provider: &'static str,
    /// Search field and path entry of the project chooser; the chosen
    /// directory itself is `projects.cwd`.
    project_query: Entity<InputState>,
    projects: projects::ProjectUi,
    project_list_scroll: gpui::UniformListScrollHandle,
    launch_error_scroll: gpui::ScrollHandle,
    /// The directory of the launch in flight, recorded as recently used on
    /// the hub once the launch is acknowledged.
    launched_cwd: String,
    label: Entity<InputState>,
    model: Entity<InputState>,
    model_picker: Entity<SelectState<SearchableVec<PickerItem>>>,
    model_picker_subscription: gpui::Subscription,
    model_choice: String,
    /// Reasoning effort for the next launch; empty = the model's default.
    effort: String,
    effort_picker: Entity<SelectState<SearchableVec<PickerItem>>>,
    effort_picker_subscription: gpui::Subscription,
    /// Levels/default/selection the effort menu was last built for.
    effort_key: String,
    context_window: Option<u64>,
    permission: Permission,
    catalog_models: Vec<ModelChoice>,
    prompt: Entity<InputState>,
    spawn_pending: bool,
    last_spawn_receipt: u64,
    spawn_error: String,
    focus: FocusHandle,
    /// Where keyboard focus goes back to when the file viewer closes.
    viewer_return: Option<FocusHandle>,
    /// This workspace's window, for viewer actions from a popped-out window.
    main_window: Option<gpui::AnyWindowHandle>,
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
        let project_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search projects or paste a folder path")
        });
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
        let settings_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        let mut focus_watch = vec![cx.subscribe(
            &settings_search,
            |_, _, event: &gpui_component::input::InputEvent, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        )];
        focus_watch.push(cx.subscribe(
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
        ));
        let model_picker_subscription = cx.subscribe_in(&model_picker, window, Self::on_model_pick);
        let effort_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(launch::effort_items(&[], None)),
                Some(gpui_component::IndexPath::new(0)),
                window,
                cx,
            )
        });
        let effort_picker_subscription =
            cx.subscribe_in(&effort_picker, window, Self::on_effort_pick);
        focus_watch.push(cx.subscribe_in(
            &project_query,
            window,
            |this, _, event: &gpui_component::input::InputEvent, window, cx| match event {
                gpui_component::input::InputEvent::Change => {
                    this.projects.cursor = 0;
                    this.project_list_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    cx.notify();
                }
                gpui_component::input::InputEvent::PressEnter { secondary: false }
                    if this.new_session =>
                {
                    this.confirm_project_cursor(window, cx)
                }
                _ => {}
            },
        ));
        let extras = features::Extras::new(window, cx);
        // Enter in Session details' name field saves, like its button.
        focus_watch.push(cx.subscribe_in(
            &extras.name,
            window,
            |this, _, event: &gpui_component::input::InputEvent, _, cx| {
                if matches!(
                    event,
                    gpui_component::input::InputEvent::PressEnter { secondary: false }
                ) && this.screen == Screen::Session
                {
                    this.save_session_name(cx);
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
        list.set_scroll_handler(cx.listener(|this, event: &ListScrollEvent, _, cx| {
            this.follow = !event.is_scrolled;
            cx.notify();
        }));
        let focus = cx.focus_handle();
        focus_watch.push(cx.on_focus(&focus, window, |_, _, cx| cx.notify()));
        focus_watch.push(cx.on_blur(&focus, window, |_, _, cx| cx.notify()));
        focus_watch.push(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        // A key aimed at something the file viewer covers (focus pulled
        // behind it) is dropped before any binding or text input sees it.
        let (this, handle) = (cx.entity().downgrade(), window.window_handle());
        focus_watch.push(cx.intercept_keystrokes(move |_, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.viewer_modal(window) && !this.viewer_has_focus(window, cx) {
                    this.hold_viewer_focus(window, cx);
                    cx.stop_propagation();
                }
            });
        }));
        focus_watch.push(cx.on_release(|this, cx| {
            // A popped-out file viewer never outlives its workspace.
            this.close_popout_window(cx);
            if let Some(path) = &this.settings_path
                && let Err(error) = this.settings.save(path)
            {
                eprintln!("Could not save native reading positions: {error}");
            }
        }));
        window.focus(&focus);
        Self {
            extras,
            remote: Default::default(),
            chat: transcript::ChatUi::default(),
            child_ui: children::ChildUi::default(),
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
            settings_search,
            settings_section: Default::default(),
            usage_open: false,
            smooth_scroll: Default::default(),
            navigation_selected: None,
            sidebar_collapsed: false,
            sidebar_drag: None,
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
            launch_details_open: false,
            provider: "claude",
            project_query,
            projects: Default::default(),
            project_list_scroll: gpui::UniformListScrollHandle::new(),
            launch_error_scroll: gpui::ScrollHandle::new(),
            launched_cwd: String::new(),
            label,
            model,
            model_picker,
            model_picker_subscription,
            model_choice: String::new(),
            effort: String::new(),
            effort_picker,
            effort_picker_subscription,
            effort_key: String::new(),
            context_window: None,
            permission: Permission::Ask,
            catalog_models: Vec::new(),
            prompt,
            spawn_pending: false,
            last_spawn_receipt: 0,
            spawn_error: String::new(),
            focus,
            viewer_return: None,
            main_window: Some(window.window_handle()),
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
        let children_changed = self.sync_children(&view, cx);
        self.receive_chat_requests(&view, cx);
        self.sync_features(&view, window, cx);
        if let Some(receipt) = &view.spawn_receipt
            && receipt.number > self.last_spawn_receipt
        {
            self.last_spawn_receipt = receipt.number;
            // Receipts reach every window on the connection. Only the window
            // whose own launch is pending records recency, and any receipt
            // retires its launched folder so a failed launch's path can never
            // ride along on another window's success.
            let launched = std::mem::take(&mut self.launched_cwd);
            let own_launch = std::mem::replace(&mut self.spawn_pending, false);
            self.spawn_error = receipt.error.clone().unwrap_or_default();
            // A new failure is read from its first line, not where the last
            // one was left scrolled.
            self.launch_error_scroll.set_offset(gpui::Point::default());
            if let Some(id) = &receipt.session {
                // Like opening a project on the desktop: the hub's registry
                // records it as recently used. Best effort; never blocks.
                if own_launch && !self.demo && wks_native::launch::absolute_directory(&launched) {
                    let at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    let _ = self.controller.command(Command::Request(
                        wks_native::features::Request::TouchProject { path: launched, at },
                    ));
                }
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
                self.hold_viewer_focus(window, cx);
                cx.notify();
                return;
            }
        }
        self.local_notice.clear();
        if self.view.selected != view.selected || self.view.child != view.child {
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
            if changed || children_changed {
                let restored = anchor.map(|anchor| scroll::remap_anchor(anchor, old, new));
                let first = if children_changed { 0 } else { first };
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
        self.sync_projects(&view);
        self.view = view;
        self.hand_off_update(cx);
        self.land_on_latest();
        if self.new_session || self.screen == Screen::Model {
            self.sync_models(window, cx);
            if reconnected {
                self.load_models(true, cx);
            }
        }
        if reconnected && (self.new_session || self.screen == Screen::Projects) {
            self.load_projects(cx);
            if self.new_session {
                self.refresh_project_inspection(cx);
            }
        }
        self.apply_ui_requests(window, cx);
        self.hold_viewer_focus(window, cx);
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
        // Never submit a draft the open viewer covers.
        if self.viewer_modal(window) {
            return;
        }
        if self.new_session {
            self.create(window, cx);
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
        let resumed = self.extras.resume.take().is_some();
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
        let fresh = !self.new_session;
        if fresh {
            self.launch_details_open = false;
            self.choose_provider(self.settings.default_provider.id(), window, cx);
            self.permission = self.settings.default_access(self.provider);
            self.model_choice.clear();
            self.context_window = None;
            self.effort.clear();
            self.model
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.reset_model_picker(window, cx);
            self.reconcile_effort(window, cx);
            self.spawn_error.clear();
            self.projects.notice.clear();
            self.projects.fallback = None;
            self.project_query
                .update(cx, |input, cx| input.set_value("", window, cx));
            // A resumed conversation's name and task are not this agent's.
            // A plain draft the user typed earlier is kept.
            if resumed {
                self.label
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.prompt
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            // Start where the user is looking: the filtered project, else the
            // open conversation's folder, else the last choice.
            let here = self.project_filter.clone().or_else(|| {
                self.view
                    .sessions
                    .iter()
                    .find(|s| Some(&s.id) == self.view.selected.as_ref())
                    .map(|s| s.cwd.clone())
                    .filter(|cwd| !cwd.is_empty())
            });
            let path = here.unwrap_or_else(|| self.projects.cwd.clone());
            if !self.seed_project(&path, cx) {
                self.refresh_project_inspection(cx);
            }
        } else if let Some(path) = self.project_filter.clone() {
            self.seed_project(&path, cx);
        }
        self.new_session = true;
        self.screen = Screen::Conversation;
        self.load_projects(cx);
        if self.projects.picker_open {
            self.project_query
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.prompt.update(cx, |input, cx| input.focus(window, cx));
        }
        self.sync_models(window, cx);
        self.load_models(false, cx);
        cx.notify();
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo
            || self.requested_session.is_some()
            || self.spawn_pending
            || self.view.creating
            || !self.view.connected
        {
            return;
        }
        self.adopt_typed_project(window, cx);
        self.spawn_error.clear();
        if self.projects.cwd.is_empty() {
            self.spawn_error = "Choose a project folder first.".into();
            self.open_project_picker(window, cx);
            return;
        }
        let request = NewSession {
            provider: self.provider.into(),
            cwd: self.projects.cwd.clone(),
            label: self.label.read(cx).value().to_string(),
            model: if self.model_choice == "__custom" {
                self.model.read(cx).value().to_string()
            } else {
                self.model_choice.clone()
            },
            context_window: self.context_window,
            permission: self.permission,
            effort: self.effort.clone(),
            message: self.prompt.read(cx).value().to_string(),
            resume_session_id: self.extras.resume.clone(),
        };
        if self.model_choice == "__custom" && request.model.trim().is_empty() {
            self.spawn_error = "Enter a custom model or choose Provider default.".into();
            self.model.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        let cwd = request.cwd.clone();
        match request
            .params()
            .and_then(|_| self.controller.command(Command::Create(request)))
        {
            Ok(()) => {
                self.spawn_pending = true;
                self.launched_cwd = cwd;
            }
            Err(error) => self.spawn_error = error.to_string(),
        }
        cx.notify();
    }

    fn move_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.screen == Screen::Settings && !self.new_session {
            self.step_settings_section(step, cx);
            return;
        }
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
        self.sidebar_scroll.scroll_to_item(
            self.sidebar_session_position(index, cx),
            gpui::ScrollStrategy::Center,
        );
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
                d.hover_text_style(|s| {
                    s.bg(rgb(if primary { p.primary_hover } else { p.selected }))
                        .text_color(rgb(if primary { p.on_primary } else { p.text }))
                })
                .active_text_style(|s| {
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

        // Inset so the panel floats with the same rounding as cards and the composer.
        let sidebar = div()
            .h_full()
            .flex_shrink_0()
            .py_2()
            .pl_2()
            .child(self.render_sidebar(narrow, compact, window, cx));

        if self.new_session {
            let content = self.render_new_session(window, cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .children(self.render_docked_viewer(window, cx))
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
                .children(self.render_docked_viewer(window, cx))
                .into_any_element();
        }
        if self.screen == Screen::Projects {
            let content = self.render_projects(window, cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .children(self.render_docked_viewer(window, cx))
                .into_any_element();
        }
        if self.screen == Screen::Settings {
            let content = self.render_settings(window, cx);
            return self
                .shell(window, cx)
                .child(sidebar)
                .child(content)
                .children(self.render_docked_viewer(window, cx))
                .into_any_element();
        }

        // Transparent fade rather than a ruled strip: history scrolls softly
        // under the floating title pill instead of colliding with a hard edge.
        let header = div().absolute().top_0().left_0().w_full().flex().justify_center()
            .bg(gpui::linear_gradient(180., gpui::linear_color_stop(rgb(p.chat), 0.6), gpui::linear_color_stop(gpui::Hsla::from(rgb(p.chat)).opacity(0.), 1.)))
            // Only the top chrome row drags, with later occluding title pills
            // and caption controls excluded by GPUI's hit test. Occluding the
            // drag surface itself keeps shell focus from cancelling OS moves.
            .when(chrome::custom_caption(), |d| d.child(chrome::drag_region(div())
                .debug_selector(|| "chat-drag-region".into())
                .absolute().top_0().left_0().w_full().h(px(56.))))
            .when(chrome::custom_caption(), |d| d.pr(px(chrome::CAPTION_WIDTH)))
            .child(chrome::chat_column().relative().pt_3().pb_5().flex().flex_col().items_center().gap_2()
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
                .child(match self.render_child_title_bar(cx) {
                    Some(bar) => bar,
                    None => self.render_title_bar(narrow, enabled, &title, selected.as_ref(), cx),
                })
                .when(!self.extras.notice.is_empty(), |d| d.child(div().px_5().text_color(rgb(p.warning)).child(self.extras.notice.clone())))
                .when(!notice.is_empty(), |d| d.child(div().occlude().py_1().text_size(px(11.)).text_color(rgb(p.warning)).child(notice)))
                .when(!self.view.connected && !self.view.transcript.rows.is_empty(), |d| d.child(self.render_connection_banner(cx)))
                .when(self.view.transcript.omitted && !self.view.transcript.has_older, |d| d.child(div().occlude().rounded_md().bg(rgb(p.surface)).px_3().text_size(px(11.)).text_color(rgb(p.muted)).child("Showing recent messages. Open History to browse older retained messages.")))
                .when(self.view.loading && !self.view.transcript.rows.is_empty(), |d| d.child(div().occlude().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(p.muted)).child(brand_spinner(12., p, "conversation-refresh")).child("Refreshing conversation…")))
                .when(self.view.connected && !self.view.loading && self.view.notice.starts_with("Conversation unavailable:"), |d| d.child(self.button("retry-conversation", "Retry conversation", true).debug_selector(|| "retry-conversation".into()).on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))))
);

        self.shell(window, cx)
            .child(sidebar)
            .child(div().relative().flex_1().min_w_0().h_full().flex().flex_col().bg(rgb(p.chat))
                .when(self.view.transcript.rows.is_empty() && (self.view.child.is_some() || self.child_ui.agents.unanchored.is_empty()), |d| d.child(self.render_empty_state(compact, window, cx)))
                .when(self.view.transcript.rows.is_empty() && self.view.child.is_none() && !self.child_ui.agents.unanchored.is_empty(), |d| d.child(self.render_child_only(window, cx)))
                .when(!self.view.transcript.rows.is_empty(), |d| d.child(transcript).child(self.wheel_smoother(cx)))
                .child(header)
                .when(!self.follow, |d| d.child(div().absolute().left_0().w_full().bottom(self.composer_dock_bounds.size.height + px(6.)).flex().justify_center().child(self.button("latest", "Jump to latest", true).shadow(chrome::floating_shadow(p)).debug_selector(|| "jump-latest".into()).mx_auto().rounded_full().bg(rgb(p.surface)).occlude().on_click(cx.listener(|this, _, _, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset { item_ix: this.view.transcript.rows.len(), offset_in_item: px(0.) }); cx.notify();
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
                .when(self.view.child.is_none(), |d| d.child(self.render_pending(window, cx)))
                .when_some(selected.as_ref().and_then(|s| s.approval.as_ref()), |d, approval| {
                    let label = approval.get("toolName").or_else(|| approval.get("tool")).and_then(serde_json::Value::as_str).unwrap_or("Tool");
                    let summary = approval.pointer("/toolInput/command").or_else(|| approval.pointer("/toolInput/file_path")).and_then(serde_json::Value::as_str).unwrap_or("").lines().next().unwrap_or("").to_owned();
                    let details = serde_json::to_string_pretty(approval.get("toolInput").or_else(|| approval.get("raw")).unwrap_or(approval)).unwrap_or_default();
                    d.child(div().occlude().w_full().p(px(if compact { 8. } else { 12. })).rounded(px(p.panel_radius)).shadow(chrome::floating_shadow(p)).bg(rgb(p.surface)).flex_shrink_0().flex().flex_col().gap_2().text_size(px(12.))
                        .child(div().flex().items_center().gap_2().text_color(rgb(p.warning)).child(status_dot(p.warning)).child(format!("Permission needed · {label}"))
                            .when(compact, |d| d.child(div().flex_1().min_w_0().truncate().text_color(rgb(p.text)).font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).child(summary.clone())).child(self.button("approval-toggle-compact", if self.extras.approval_details { "Hide" } else { "Details" }, true).on_click(cx.listener(|this, _, _, cx| { this.extras.approval_details = !this.extras.approval_details; cx.notify(); })))))
                        .when(!compact, |d| d.child(div().flex().items_center().gap_2().child(div().flex_1().min_w_0().truncate().font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).child(if summary.is_empty() { "Review request details".to_owned() } else { summary })).child(self.button("approval-toggle", if self.extras.approval_details { "Hide details" } else { "Details" }, true).on_click(cx.listener(|this, _, _, cx| { this.extras.approval_details = !this.extras.approval_details; cx.notify(); })))))
                        .when(self.extras.approval_details, |d| d.child(div().id("approval-details").max_h(px(if compact { 52. } else { 120. })).overflow_y_scroll().p_3().rounded(px(p.control_radius)).bg(rgb(p.code_block)).border_1().border_color(rgb(p.border)).font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).text_color(rgb(p.prose)).child(details)))
                        .child(div().flex().gap_2()
                            .child(self.primary_button("approve", "Allow once", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(true), cx)))))
                            .child(self.button("deny", "Deny", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(false), cx)))))))
                })
                .when_some(selected.as_ref().filter(|s| s.questions.is_some()), |d, session| d.child(self.render_questions(session, enabled, compact, cx)))
                .children(self.render_child_bar(cx))
                .when(selected.is_some() && self.view.child.is_none(), |d| d.child(div().w_full().flex_shrink_0().flex().flex_col().gap_2()
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
                                .child(self.icon_button("paste-image", "Paste an image from the clipboard", IconName::GalleryVerticalEnd, enabled && !self.uploading()).when(enabled && !self.uploading(), |d| d.on_click(cx.listener(|this, _, _, cx| { if !this.paste_image(cx) { this.extras.notice = "No supported image on the clipboard.".into(); cx.notify(); } }))))
                                .children(selected.as_ref().and_then(|s| chrome::context_meter(s, p))))
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
            .children(self.render_docked_viewer(window, cx))
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

    struct HoverControls(Entity<Workspace>);

    impl Render for HoverControls {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |workspace, cx| {
                div()
                    .text_size(px(20.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_4()
                    .child(
                        workspace
                            .button("regular", "Regular action", true)
                            .debug_selector(|| "regular".into())
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        workspace
                            .primary_button("primary", "Primary action", true)
                            .debug_selector(|| "primary".into())
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        workspace
                            .quiet_button("quiet", "Quiet action", IconName::Info, true)
                            .debug_selector(|| "quiet".into())
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        workspace
                            .icon_button("icon", "Icon action", IconName::Info, true)
                            .debug_selector(|| "icon".into())
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        workspace
                            .file_button(
                                "file",
                                wks_native::links::tool_file("/repo", "src/main.rs", None),
                                "src/main.rs",
                                cx,
                            )
                            .debug_selector(|| "file".into()),
                    )
            })
        }
    }

    #[gpui::test]
    fn button_hover_and_press_preserve_geometry(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (controller, _commands, _updates) = Controller::test_channels();
        let window = cx.add_window(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(controller, true, window, cx));
            workspace.update(cx, |workspace, _| workspace.view = Arc::new(state("a")));
            let controls = cx.new(|_| HoverControls(workspace));
            Root::new(controls, window, cx)
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        for selector in ["regular", "primary", "quiet", "icon", "file"] {
            visual.simulate_mouse_move(
                gpui::point(px(800.), px(600.)),
                None,
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
            let before = visual.debug_bounds(selector).unwrap();
            visual.simulate_mouse_move(before.center(), None, gpui::Modifiers::default());
            visual.run_until_parked();
            assert_eq!(
                before,
                visual.debug_bounds(selector).unwrap(),
                "{selector} hover changed geometry"
            );
            visual.simulate_mouse_down(
                before.center(),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
            assert_eq!(
                before,
                visual.debug_bounds(selector).unwrap(),
                "{selector} press changed geometry"
            );
            visual.simulate_mouse_up(
                gpui::point(px(800.), px(600.)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
        }
    }

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

    /// Reads the launch form and project list issue on their own; none of
    /// them launches, selects or writes anything.
    fn project_read(command: &Command) -> bool {
        use wks_native::features::Request;
        matches!(
            command,
            Command::LoadModels { .. }
                | Command::Request(
                    Request::Projects
                        | Request::InspectProject { .. }
                        | Request::BrowseFolders { .. }
                )
        )
    }

    /// The next command that is not one of those reads.
    fn next_effect(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Option<Command> {
        std::iter::from_fn(|| commands.try_recv().ok()).find(|c| !project_read(c))
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
    fn sidebar_nests_sessions_and_opens_native_child_from_another_parent(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _) = fixture(cx);
        visual.update(|window, cx| workspace.update(cx, |this, cx| {
            let mut next = state("b");
            let sessions = Arc::make_mut(&mut next.sessions);
            sessions[0].subagents = serde_json::json!([{
                "id": "codex-child", "description": "Native research", "status": "running", "model": "gpt-5"
            }]);
            sessions.insert(0, Session { id: "grandchild".into(), parent_session_id: "child".into(), ..Default::default() });
            sessions.insert(0, Session { id: "child".into(), parent_session_id: "a".into(), ..Default::default() });
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![2, 0, 1, 3]);
        }));
        visual.run_until_parked();
        let parent = visual.debug_bounds("sidebar-session-0").unwrap();
        let native = visual.debug_bounds("sidebar-provider-1").unwrap();
        let child = visual.debug_bounds("sidebar-session-2").unwrap();
        let grandchild = visual.debug_bounds("sidebar-session-3").unwrap();
        assert!(native.left() > parent.left());
        assert!(child.left() > parent.left());
        assert!(grandchild.left() > child.left());
        assert!(
            parent.top() < native.top()
                && native.top() < child.top()
                && child.top() < grandchild.top()
        );
        // uniform_list gives every row the session-card height; a taller
        // child row would paint over its neighbours.
        assert!(parent.bottom() <= native.top() && native.bottom() <= child.top());
        // Workspacer-spawned children are sessions: they archive like parents.
        assert!(visual.debug_bounds("sidebar-archive-2").is_some());
        assert!(visual.debug_bounds("sidebar-archive-3").is_some());
        // Child rows carry the same brand model badge as session cards.
        assert!(visual.debug_bounds("sidebar-child-model-1").is_some());
        // A provider-native child opens as its own chat; the controller
        // selects its parent first.
        visual.simulate_click(native.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(matches!(commands.try_recv().unwrap(),
            Command::ViewChild(Some(t)) if t.parent == "a" && t.agent == "codex-child"));
        assert!(commands.try_recv().is_err());

        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.sidebar_collapsed = true;
                cx.notify();
            })
        });
        visual.run_until_parked();
        let rail_child = visual.debug_bounds("sidebar-provider-1").unwrap();
        assert!(rail_child.right() <= px(56.));
        visual.simulate_click(rail_child.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(matches!(commands.try_recv().unwrap(),
            Command::ViewChild(Some(t)) if t.parent == "a" && t.agent == "codex-child"));
    }

    #[gpui::test]
    fn sidebar_resize_tracks_drag_and_persists_width(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _) = fixture(cx);
        let path = std::env::temp_dir().join(format!("native-sidebar-{}.json", std::process::id()));
        visual.update(|_, cx| {
            workspace.update(cx, |this, _| this.settings_path = Some(path.clone()))
        });
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            px(304.)
        );
        let start = visual.debug_bounds("sidebar-resize").unwrap().center();
        let end = start + gpui::point(px(70.), px(0.));
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();
        visual.simulate_mouse_move(
            end,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        visual.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            px(374.)
        );
        assert_eq!(Settings::load(&path).unwrap().sidebar_width, 374.);
        workspace.read_with(&visual, |this, _| assert!(this.sidebar_drag.is_none()));
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            px(288.)
        );
        visual.simulate_resize(size(px(1000.), px(700.)));
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            px(374.)
        );
        let _ = std::fs::remove_file(path);
    }

    #[gpui::test]
    fn sidebar_model_metadata_stays_inside_clickable_session_rows(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                let session = &mut Arc::make_mut(&mut view.sessions)[0];
                session.provider = "codex".into();
                session.model = "a-model-with-a-long-name-that-needs-to-fit-in-the-sidebar".into();
                session.cwd = "/work/project".into();
                this.update_view(Arc::new(view), window, cx);
            });
        });
        visual.run_until_parked();
        let row = visual.debug_bounds("sidebar-session-0").unwrap();
        let model = visual.debug_bounds("sidebar-model-0").unwrap();
        assert!(row.contains(&model.origin));
        assert!(row.contains(&model.bottom_right()));
        let second = visual.debug_bounds("sidebar-session-1").unwrap();
        visual.simulate_click(second.center(), gpui::Modifiers::default());
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    }

    #[gpui::test]
    fn chat_markdown_follows_the_appearance_syntax_theme(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        for appearance in Appearance::ALL {
            visual.update(|window, cx| {
                configure_theme(appearance, Some(window), cx);
                assert_eq!(
                    *gpui_component::Theme::global(cx).highlight_theme,
                    *syntax::highlight_theme(appearance)
                );
                workspace.update(cx, |this, cx| {
                    this.appearance = appearance;
                    let mut view = state("a");
                    view.transcript.snapshot(ConversationSnapshot {
                        seq: 1,
                        first_seq: 1,
                        items: vec![Item {
                            kind: "assistant_text".into(),
                            text: "## Plan\n\n**Bold** and *italic* with `src/main.rs`.\n\n\
                                   - one\n1. two\n\n---\n\n```rust\nfn main() {}\n```\n\n```\nplain\n```"
                                .into(),
                            ..Default::default()
                        }],
                    });
                    this.update_view(Arc::new(view), window, cx);
                })
            });
            visual.run_until_parked();
            assert!(visual.debug_bounds("markdown-inline-live:a:0-0").is_some());
        }
        assert_ne!(
            syntax::highlight_theme(Appearance::Dark),
            syntax::highlight_theme(Appearance::Nord)
        );
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
        let Command::Request(wks_native::features::Request::FilePreview { session, target }) =
            request
        else {
            panic!("file click must request a native preview");
        };
        assert_eq!(session, "a");
        assert_eq!(target.path, "/fixture/repo/docs/README.md");
        assert_eq!(target.line, Some(12));
        let preview = |loading: bool, error: Option<&str>, number: u64| {
            let (session, target, workspace) = (session.clone(), target.clone(), workspace.clone());
            let error = error.map(str::to_owned);
            move |window: &mut Window, cx: &mut App| {
                workspace.update(cx, |this, cx| {
                    let mut view = (*this.view).clone();
                    view.requests.insert("file-preview", wks_native::features::RequestState {
                        request: wks_native::features::Request::FilePreview { session, target },
                        number, loading, error,
                        value: Arc::new(serde_json::json!({"contents":(1..=40).map(|i| format!("line {i}\n")).collect::<String>(),"size":290})),
                    });
                    this.update_view(Arc::new(view), window, cx);
                })
            }
        };
        visual.update(preview(true, None, 1));
        visual.run_until_parked();
        assert!(
            visual.debug_bounds("file-viewer").is_some(),
            "loading opens the sheet"
        );
        assert!(visual.debug_bounds("file-viewer-text").is_none());
        visual.update(preview(false, None, 1));
        visual.run_until_parked();
        assert!(visual.debug_bounds("file-viewer-title").is_some());
        assert!(visual.debug_bounds("file-viewer-text").is_some());
        let (text, cursor) = workspace.read_with(&visual, |this, cx| {
            let pane = this.file_viewer().unwrap().read(cx);
            assert_eq!(
                pane.mode(),
                file_viewer::Mode::Source,
                "a line anchor opens a Markdown file's source at that line"
            );
            let editor = pane.editor().unwrap().read(cx);
            (editor.value().to_string(), editor.cursor_position())
        });
        assert!(text.starts_with("line 1\n"));
        assert_eq!(cursor.line, 11, "line anchors place the cursor on line 12");
        // Read-only: typing reaches no file and changes nothing.
        visual.simulate_input("typed");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(
                this.file_viewer()
                    .unwrap()
                    .read(cx)
                    .editor()
                    .unwrap()
                    .read(cx)
                    .value()
                    .to_string(),
                text
            );
        });
        // Wheel over the sheet never scrolls the conversation underneath.
        let top = |this: &Workspace| {
            let top = this.list.logical_scroll_top();
            (top.item_ix, top.offset_in_item)
        };
        let before = workspace.read_with(&visual, |this, _| top(this));
        let sheet = visual.debug_bounds("file-viewer-text").unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: sheet.center(),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., -3.)),
            ..Default::default()
        });
        visual.run_until_parked();
        assert_eq!(workspace.read_with(&visual, |this, _| top(this)), before);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.file_viewer().is_none(), "Esc closes the viewer")
        });
        // Later view updates never rebuild a dismissed viewer or take focus.
        visual.update(preview(false, None, 1));
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.file_viewer().is_none());
        });
        // A failed read stays visible as a message inside the sheet.
        visual.update(preview(
            false,
            Some("No file at this path on the session's machine."),
            2,
        ));
        visual.run_until_parked();
        assert!(visual.debug_bounds("file-viewer-error").is_some());
        settle(&mut visual);
        let close = visual.debug_bounds("file-viewer-close").unwrap();
        visual.simulate_click(close.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(
                this.file_viewer().is_none(),
                "close button closes the viewer"
            )
        });
    }

    #[gpui::test]
    fn chat_links_route_web_refused_and_image_targets(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 3,
                    first_seq: 1,
                    items: [
                        "[Docs site](https://example.com/docs)",
                        "[Mail the team](mailto:team@example.com)",
                        "![Screenshot](out/shot.png)",
                    ]
                    .into_iter()
                    .map(|text| Item {
                        kind: "assistant_text".into(),
                        text: text.into(),
                        ..Default::default()
                    })
                    .collect(),
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        // TextView parses Markdown behind a real-time debounce the test
        // executor does not drive, so retry each click within a bounded wait.
        fn click_until(
            visual: &mut VisualTestContext,
            row: &'static str,
            mut done: impl FnMut(&mut VisualTestContext) -> bool,
        ) {
            for _ in 0..100 {
                let bounds = visual.debug_bounds(row).expect("markdown row");
                visual.simulate_click(
                    bounds.origin + gpui::point(px(20.), bounds.size.height / 2.),
                    gpui::Modifiers::default(),
                );
                visual.run_until_parked();
                if done(visual) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("{row} never responded to a click");
        }
        let platform = cx.clone();
        click_until(&mut visual, "markdown-inline-live:a:0-0", |_| {
            platform.opened_url().is_some()
        });
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        assert!(commands.try_recv().is_err(), "web links never read files");
        let notice = workspace.clone();
        click_until(&mut visual, "markdown-inline-live:a:1-0", |visual| {
            notice.read_with(visual, |this, _| !this.extras.notice.is_empty())
        });
        workspace.read_with(&visual, |this, _| {
            assert!(
                this.extras.notice.contains("mailto"),
                "refusals are visible"
            )
        });
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        assert!(commands.try_recv().is_err());
        // The image is a label, not a client-side load, and opens the viewer.
        let mut request = None;
        click_until(&mut visual, "markdown-inline-live:a:2-0", |_| {
            request = commands.try_recv().ok();
            request.is_some()
        });
        let Some(Command::Request(wks_native::features::Request::FilePreview { session, target })) =
            request
        else {
            panic!("image click must request a native preview");
        };
        assert_eq!(target.path, "/fixture/repo/out/shot.png");
        assert_eq!(target.kind, wks_native::links::FileKind::Image);
        let png = {
            use base64::Engine;
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(64, 32)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
        };
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "file-preview",
                    wks_native::features::RequestState {
                        request: wks_native::features::Request::FilePreview { session, target },
                        number: 1,
                        loading: false,
                        error: None,
                        value: Arc::new(
                            serde_json::json!({"png":png,"width":64,"height":32,"size":120}),
                        ),
                    },
                );
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("file-viewer-image").is_some());
        // Backdrop click closes it.
        visual.simulate_click(gpui::point(px(4.), px(4.)), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
    }

    #[gpui::test]
    fn html_card_links_route_from_raw_fence_through_sanitizer(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        // Tall enough that every card is laid out at once.
        visual.simulate_resize(size(px(1000.), px(1600.)));
        let card = |body: &str| {
            let raw =
                serde_json::json!({"v":1,"title":"Card","fallback":"Fallback","bodyHtml":body});
            format!("```wks-html-card\n{raw}\n```")
        };
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 5,
                    first_seq: 1,
                    items: [
                        card("<p><a href='https://example.com/docs?a=1&amp;b=&quot;2&quot;' onclick='x()'>Web docs</a></p>"),
                        card("<p><a href='src/long.rs#L150'>HTML source</a></p>"),
                        card("<p><img src='out/shot.png' alt='HTML image' onerror='x()'></p>"),
                        card("<p><a href='javascript:alert(1)'>Script link</a> <a href='mailto:a@b.c'>Mail</a></p>"),
                        card("<p><img src='https://example.com/x.png' alt='Remote image'>Remote</p>"),
                    ]
                    .into_iter()
                    .map(|text| Item {
                        kind: "assistant_text".into(),
                        text,
                        ..Default::default()
                    })
                    .collect(),
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        // TextView parses HTML behind a real-time debounce; retry clicks.
        fn click_until(
            visual: &mut VisualTestContext,
            card: &'static str,
            mut done: impl FnMut(&mut VisualTestContext) -> bool,
        ) {
            for _ in 0..100 {
                // Empty until the debounced parse lands.
                let Some(bounds) = visual.debug_bounds(card) else {
                    visual.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                };
                visual.simulate_click(
                    bounds.origin + gpui::point(px(20.), px(8.)),
                    gpui::Modifiers::default(),
                );
                visual.run_until_parked();
                if done(visual) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("{card} never responded to a click");
        }
        let platform = cx.clone();
        click_until(&mut visual, "html-card-live:a:0-0", |_| {
            platform.opened_url().is_some()
        });
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.com/docs?a=1&b=%222%22"),
            "the web destination survives sanitizing and both HTML parses (then URL-normalized)"
        );
        assert!(commands.try_recv().is_err(), "web links never read files");
        let mut request = None;
        click_until(&mut visual, "html-card-live:a:1-0", |_| {
            request = commands.try_recv().ok();
            request.is_some()
        });
        let Some(Command::Request(wks_native::features::Request::FilePreview { session, target })) =
            request
        else {
            panic!("an HTML file link must request a native preview");
        };
        assert_eq!(session, "a");
        assert_eq!(target.path, "/fixture/repo/src/long.rs");
        assert_eq!(target.line, Some(150));
        let mut request = None;
        click_until(&mut visual, "html-card-live:a:2-0", |_| {
            request = commands.try_recv().ok();
            request.is_some()
        });
        let Some(Command::Request(wks_native::features::Request::FilePreview { target, .. })) =
            request
        else {
            panic!("an HTML image must request a native preview");
        };
        assert_eq!(target.path, "/fixture/repo/out/shot.png");
        assert_eq!(target.kind, wks_native::links::FileKind::Image);
        // Refused schemes and remote images were dropped by the sanitizer:
        // nothing opens, nothing is requested, and no router refusal ran.
        for card in ["html-card-live:a:3-0", "html-card-live:a:4-0"] {
            for _ in 0..25 {
                let Some(bounds) = visual.debug_bounds(card) else {
                    visual.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                };
                visual.simulate_click(
                    bounds.origin + gpui::point(px(20.), px(8.)),
                    gpui::Modifiers::default(),
                );
                visual.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.com/docs?a=1&b=%222%22")
        );
        assert!(commands.try_recv().is_err());
        workspace.read_with(&visual, |this, _| assert!(this.extras.notice.is_empty()));
    }

    #[gpui::test]
    fn file_viewer_contains_keys_over_a_nonempty_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("DRAFT_MUST_NOT_SEND");
        visual.run_until_parked();
        let png = {
            use base64::Engine;
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(32, 16)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
        };
        let open = |path: &str, number: u64, loading: bool, error: Option<&str>| {
            let wks_native::links::Link::File(target) = wks_native::links::classify("/repo", path)
            else {
                panic!("{path} is a file link");
            };
            let value = serde_json::json!({
                "contents": "one\ntwo\n", "size": 8, "png": png, "width": 32, "height": 16,
            });
            let (workspace, error) = (workspace.clone(), error.map(str::to_owned));
            move |window: &mut Window, cx: &mut App| {
                workspace.update(cx, |this, cx| {
                    let mut view = (*this.view).clone();
                    view.requests.insert(
                        "file-preview",
                        wks_native::features::RequestState {
                            request: wks_native::features::Request::FilePreview {
                                session: "a".into(),
                                target,
                            },
                            number,
                            loading,
                            error,
                            value: Arc::new(value),
                        },
                    );
                    this.update_view(Arc::new(view), window, cx);
                })
            }
        };
        let untouched = |visual: &mut VisualTestContext,
                         commands: &mut tokio::sync::mpsc::Receiver<Command>,
                         keys: &str| {
            workspace.read_with(visual, |this, cx| {
                assert!(
                    this.file_viewer().is_some(),
                    "{keys} must not close the viewer"
                );
                assert_eq!(
                    this.composer.read(cx).value().as_ref(),
                    "DRAFT_MUST_NOT_SEND",
                    "{keys} reached the covered composer"
                );
                assert_eq!(this.screen, Screen::Conversation, "{keys} navigated");
                assert!(!this.new_session, "{keys} started a session");
                assert_eq!(this.view.selected.as_deref(), Some("a"));
            });
            visual.update(|window, cx| {
                assert!(
                    workspace.read(cx).viewer_has_focus(window, cx),
                    "{keys} moved focus out of the viewer"
                )
            });
            assert!(
                commands.try_recv().is_err(),
                "{keys} reached the workspace under the viewer"
            );
        };
        let states: [(&str, u64, bool, Option<&str>, &str); 5] = [
            ("docs/a.md", 1, true, None, "file-viewer"),
            ("docs/a.rs", 2, false, None, "file-viewer-text"),
            ("docs/a.md", 3, false, None, "file-viewer-markdown"),
            ("shot.png", 4, false, None, "file-viewer-image"),
            (
                "gone.md",
                5,
                false,
                Some("No file at this path."),
                "file-viewer-error",
            ),
        ];
        for (path, number, loading, error, selector) in states {
            visual.update(open(path, number, loading, error));
            visual.run_until_parked();
            assert!(visual.debug_bounds(selector).is_some(), "{selector} shows");
            visual.update(|window, cx| {
                assert!(
                    workspace.read(cx).viewer_modal(window),
                    "a 1000px window shows the viewer as a modal sheet"
                )
            });
            for keys in [
                "ctrl-enter",
                "cmd-enter",
                "alt-down",
                "alt-up",
                "ctrl-n",
                "ctrl-p",
                "ctrl-,",
                "ctrl-l",
                "ctrl-r",
                "ctrl-0",
                "tab",
                "shift-tab",
                "enter",
                "x",
                "backspace",
            ] {
                visual.simulate_keystrokes(keys);
                visual.run_until_parked();
                untouched(&mut visual, &mut commands, keys);
            }
            visual.simulate_input("typed");
            visual.run_until_parked();
            untouched(&mut visual, &mut commands, "typing");
            if selector == "file-viewer-text" {
                // The viewer's own keys still work: select all and copy.
                visual.simulate_keystrokes("ctrl-a ctrl-c");
                visual.run_until_parked();
                let copied = visual.update(|_, cx| cx.read_from_clipboard());
                assert_eq!(
                    copied.and_then(|item| item.text()).as_deref(),
                    Some("one\ntwo\n")
                );
                untouched(&mut visual, &mut commands, "ctrl-a ctrl-c");
            }
            // Focus pulled behind the sheet (a late receipt, a UI request) is
            // taken back before a key can reach the composer.
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    this.composer.read(cx).focus_handle(cx).focus(window)
                })
            });
            visual.simulate_keystrokes("ctrl-enter");
            visual.simulate_input("z");
            visual.run_until_parked();
            untouched(&mut visual, &mut commands, "refocused ctrl-enter");
            // Dismiss and reopen the next state from the composer again.
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
        }
        // Closed, the same draft and shortcut work normally again.
        let focused = visual.update(|window, cx| {
            workspace
                .read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(focused, "closing returns focus to the composer");
        visual.simulate_keystrokes("ctrl-enter");
        let Command::Act {
            session,
            action: Action::Send(text),
        } = commands.try_recv().expect("send works after closing")
        else {
            panic!("wrong action");
        };
        assert_eq!(
            (session.as_str(), text.as_str()),
            ("a", "DRAFT_MUST_NOT_SEND")
        );
    }

    const GUIDE: &str = "[Library source](../src/lib.rs:3)\n\n# Guide\n\n\
        Intro with **bold**, *italic*, `code`, ~~gone~~ and a [web link](https://example.com/docs).\n\n\
        ## Usage\n\n- one\n- two\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo\n\n\
        > A quoted note.\n\n| Name | Value |\n| --- | --- |\n| a | 1 |\n\n\
        ```rust\nfn main() {}\n```\n\n![Diagram](img/diagram.png)\n\n---\n\n\
        ## Usage\n\nSecond usage.\n\n## Usage-1\n\nA literal heading that collides.\n\n\
        Filler 1.\n\nFiller 2.\n\nFiller 3.\n\nFiller 4.\n\nFiller 5.\n\nFiller 6.\n\n\
        Filler 7.\n\nFiller 8.\n\nFiller 9.\n\nFiller 10.\n\nFiller 11.\n\nFiller 12.\n\n\
        Filler 13.\n\nFiller 14.\n\nFiller 15.\n\nFiller 16.\n\nFiller 17.\n\nFiller 18.\n\n\
        Filler 19.\n\nFiller 20.\n\nFiller 21.\n\nFiller 22.\n\nFiller 23.\n\nFiller 24.\n";

    /// The viewer slides in over ~240ms of real time; wait it out before
    /// clicking its controls so hit targets are where their bounds say.
    fn settle(visual: &mut VisualTestContext) {
        std::thread::sleep(std::time::Duration::from_millis(260));
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
    }

    /// Put a `file-preview` request state into the view, as the controller
    /// does when a read starts or finishes.
    #[allow(clippy::too_many_arguments)]
    fn preview_state(
        workspace: &Entity<Workspace>,
        visual: &mut VisualTestContext,
        session: &str,
        target: wks_native::links::FileTarget,
        number: u64,
        loading: bool,
        error: Option<&str>,
        value: serde_json::Value,
    ) {
        let (session, error) = (session.to_owned(), error.map(str::to_owned));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "file-preview",
                    wks_native::features::RequestState {
                        request: wks_native::features::Request::FilePreview { session, target },
                        number,
                        loading,
                        error,
                        value: Arc::new(value),
                    },
                );
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
    }

    fn file_target(raw: &str) -> wks_native::links::FileTarget {
        match wks_native::links::classify("/", raw) {
            wks_native::links::Link::File(target) => target,
            other => panic!("{raw} is not a file: {other:?}"),
        }
    }

    fn preview_request(
        commands: &mut tokio::sync::mpsc::Receiver<Command>,
    ) -> Option<(String, wks_native::links::FileTarget)> {
        match commands.try_recv().ok()? {
            Command::Request(wks_native::features::Request::FilePreview { session, target }) => {
                Some((session, target))
            }
            _ => panic!("unexpected command instead of a file preview"),
        }
    }

    fn pane_of(
        workspace: &Entity<Workspace>,
        visual: &VisualTestContext,
    ) -> Entity<file_viewer::PreviewPane> {
        workspace.read_with(visual, |this, _| {
            this.file_viewer().cloned().expect("viewer open")
        })
    }

    #[gpui::test]
    fn markdown_files_render_with_a_source_toggle_and_document_links(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                // Document links must ignore the session's working folder.
                Arc::make_mut(&mut view.sessions)[0].cwd = "/work/elsewhere".into();
                this.update_view(Arc::new(view), window, cx);
            })
        });
        let guide = file_target("/repo/docs/Guide.MD");
        preview_state(
            &workspace,
            &mut visual,
            "a",
            guide.clone(),
            1,
            true,
            None,
            serde_json::json!({}),
        );
        assert!(visual.debug_bounds("file-viewer").is_some());
        preview_state(
            &workspace,
            &mut visual,
            "a",
            guide.clone(),
            1,
            false,
            None,
            serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
        );
        assert!(
            visual.debug_bounds("file-viewer-markdown").is_some(),
            "Markdown opens rendered"
        );
        assert!(visual.debug_bounds("file-viewer-text").is_none());
        assert!(visual.debug_bounds("file-viewer-mode-source").is_some());
        let pane = pane_of(&workspace, &visual);
        // The rendered document has the GFM structures, headings in order.
        visual.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.mode(), file_viewer::Mode::Preview);
            let document = pane.document().expect("rendered document");
            let kinds = document.block_kinds(cx);
            for kind in [
                "heading",
                "paragraph",
                "list",
                "ordered-list",
                "task-list",
                "blockquote",
                "table",
                "code",
                "divider",
            ] {
                assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
            }
            let headings: Vec<_> = document
                .headings(cx)
                .into_iter()
                .map(|(_, level, text)| (level, text))
                .collect();
            assert_eq!(
                headings,
                [
                    (1, "Guide".into()),
                    (2, "Usage".into()),
                    (2, "Usage".into()),
                    (2, "Usage-1".into())
                ]
            );
        });
        settle(&mut visual);
        // A real click on the document's first link goes through the
        // pane's router, against the document's folder on the same session.
        let mut request = None;
        for _ in 0..100 {
            let bounds = visual.debug_bounds("file-viewer-markdown").unwrap();
            visual.simulate_click(
                bounds.origin + gpui::point(px(30. + 24.), px(16. + 10.)),
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
            request = preview_request(&mut commands);
            if request.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (session, target) = request.expect("the document link opened");
        assert_eq!(session, "a");
        assert_eq!(
            (target.path.as_str(), target.line),
            ("/repo/src/lib.rs", Some(3))
        );
        let follow = |visual: &mut VisualTestContext, raw: &'static str| {
            visual
                .update(|window, cx| pane.update(cx, |pane, cx| pane.follow_link(raw, window, cx)));
            visual.run_until_parked();
        };
        // Images are labels that open the image viewer from the same folder.
        follow(&mut visual, "img/diagram.png");
        let (_, image) = preview_request(&mut commands).expect("image link reads the image");
        assert_eq!(image.path, "/repo/docs/img/diagram.png");
        assert_eq!(image.kind, wks_native::links::FileKind::Image);
        // Same-document anchors scroll; they never re-read the file.
        let top = |visual: &mut VisualTestContext| {
            visual.update(|_, cx| pane.read(cx).document().unwrap().scroll_top(cx).0)
        };
        let usage_again =
            visual.update(|_, cx| pane.read(cx).document().unwrap().headings(cx)[2].0);
        follow(&mut visual, "#usage-1");
        assert_eq!(
            top(&mut visual),
            usage_again,
            "#usage-1 is the second Usage"
        );
        // A literal "Usage-1" heading never shares the generated anchor.
        let literal = visual.update(|_, cx| pane.read(cx).document().unwrap().headings(cx)[3].0);
        assert!(literal > usage_again);
        follow(&mut visual, "#usage-1-1");
        assert_eq!(
            top(&mut visual),
            literal,
            "#usage-1-1 is the literal Usage-1"
        );
        visual.update(|_, cx| assert!(pane.read(cx).notice().is_none()));
        follow(&mut visual, "#usage-1");
        assert_eq!(top(&mut visual), usage_again);
        follow(&mut visual, "#");
        assert_eq!(top(&mut visual), 0);
        follow(&mut visual, "Guide.MD#usage-1");
        assert_eq!(top(&mut visual), usage_again);
        assert!(commands.try_recv().is_err(), "anchors stay in the document");
        follow(&mut visual, "#not-here");
        visual.update(|_, cx| assert!(pane.read(cx).notice().unwrap().contains("No heading")));
        // Unsafe schemes are refused visibly inside the viewer.
        follow(&mut visual, "javascript:alert(1)");
        visual.update(|_, cx| assert!(pane.read(cx).notice().unwrap().contains("javascript")));
        assert!(visual.debug_bounds("file-viewer-notice").is_some());
        assert!(cx.opened_url().is_none() && commands.try_recv().is_err());
        follow(&mut visual, "https://example.com/docs");
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        // A line of this same document opens its source there.
        follow(&mut visual, "Guide.MD:5");
        assert!(
            commands.try_recv().is_err(),
            "same-file lines do not re-read"
        );
        visual.update(|window, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.mode(), file_viewer::Mode::Source);
            let editor = pane.editor().unwrap().read(cx);
            assert_eq!(editor.cursor_position().line, 4);
            assert_eq!(editor.value().as_ref(), GUIDE, "the loaded file is kept");
            assert!(editor.focus_handle(cx).is_focused(window));
        });
        assert!(visual.debug_bounds("file-viewer-text").is_some());
        // Ctrl+Shift+V switches back without reading anything again, and
        // keeps keyboard focus inside the viewer.
        visual.simulate_keystrokes("ctrl-shift-v");
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview);
            assert!(workspace.read(cx).viewer_has_focus(window, cx));
        });
        assert!(visual.debug_bounds("file-viewer-markdown").is_some());
        // Keys scroll the rendered document.
        visual.simulate_keystrokes("end");
        visual.run_until_parked();
        let bottom = top(&mut visual);
        assert!(bottom > 0, "End scrolls to the bottom");
        // A round trip through the source keeps the reading position.
        visual.simulate_keystrokes("ctrl-shift-v");
        visual.run_until_parked();
        visual.simulate_keystrokes("ctrl-shift-v");
        visual.run_until_parked();
        assert_eq!(
            top(&mut visual),
            bottom,
            "Preview → Source → Preview keeps the scroll"
        );
        visual.simulate_keystrokes("home");
        visual.run_until_parked();
        assert_eq!(top(&mut visual), 0);
        // Ctrl+F from the preview searches the source.
        visual.simulate_keystrokes("ctrl-f");
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Source);
            assert!(workspace.read(cx).viewer_has_focus(window, cx));
        });
        // Clicking the Preview segment returns to the rendered document.
        let segment = visual.debug_bounds("file-viewer-mode-preview").unwrap();
        visual.simulate_click(segment.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview));
        assert!(commands.try_recv().is_err());
        // Following a file link records Back; the next file arrives in place.
        follow(&mut visual, "../src/lib.rs:3");
        let (_, lib) = preview_request(&mut commands).unwrap();
        preview_state(
            &workspace,
            &mut visual,
            "a",
            lib,
            2,
            false,
            None,
            serde_json::json!({"contents": "a\nb\nc\nd\n", "size": 8}),
        );
        assert!(visual.debug_bounds("file-viewer-text").is_some());
        visual.update(|_, cx| assert!(!pane.read(cx).previewable(), "source files have no toggle"));
        let back = visual
            .debug_bounds("file-viewer-back")
            .expect("Back after a link");
        visual.simulate_click(back.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (_, again) = preview_request(&mut commands).expect("Back re-reads the document");
        assert_eq!(again.path, "/repo/docs/Guide.MD");
    }

    #[gpui::test]
    fn one_ctrl_f_from_the_preview_opens_source_search(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |c, cx| c.set_value("KEEP_DRAFT", window, cx));
            })
        });
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/Guide.MD"),
            1,
            false,
            None,
            serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
        );
        settle(&mut visual);
        let pane = pane_of(&workspace, &visual);
        let query = |visual: &mut VisualTestContext| {
            visual.update(|_, cx| {
                let editor = pane.read(cx).editor().unwrap().read(cx);
                editor.search_query(cx).map(|q| q.to_string())
            })
        };
        visual.update(|window, cx| {
            assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview);
            assert!(workspace.read(cx).viewer_has_focus(window, cx));
        });
        assert_eq!(query(&mut visual), None);
        // One press: the source shows with its search open and focused.
        visual.simulate_keystrokes("ctrl-f");
        visual.run_until_parked();
        assert_eq!(query(&mut visual).as_deref(), Some(""));
        visual.update(|window, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.mode(), file_viewer::Mode::Source);
            assert!(
                !pane.editor().unwrap().focus_handle(cx).is_focused(window),
                "the query field, not the read-only source, has the keyboard"
            );
            assert!(workspace.read(cx).viewer_has_focus(window, cx));
        });
        // Typing goes into the query, and Enter finds the next match.
        visual.simulate_input("Second usage");
        visual.run_until_parked();
        assert_eq!(query(&mut visual).as_deref(), Some("Second usage"));
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert_eq!(query(&mut visual).as_deref(), Some("Second usage"));
        // Esc closes the search, back to the source; a second Esc the viewer.
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(query(&mut visual), None);
        visual.update(|window, cx| {
            assert!(
                pane.read(cx)
                    .editor()
                    .unwrap()
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert!(workspace.read(cx).file_viewer().is_some());
        });
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(this.file_viewer().is_none());
            assert_eq!(this.composer.read(cx).value().as_ref(), "KEEP_DRAFT");
        });
        // Nothing typed into the viewer reached the hub.
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn markdown_line_anchors_and_oversized_documents_open_as_source(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/README.markdown:3"),
            1,
            false,
            None,
            serde_json::json!({"contents": "# A\n\nb\nc\n", "size": 10}),
        );
        let pane = pane_of(&workspace, &visual);
        visual.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(
                pane.mode(),
                file_viewer::Mode::Source,
                "line anchors open source"
            );
            assert_eq!(pane.editor().unwrap().read(cx).cursor_position().line, 2);
        });
        // The toggle still offers the rendered document.
        assert!(visual.debug_bounds("file-viewer-mode-preview").is_some());
        visual.simulate_keystrokes("ctrl-shift-v");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview));
        // Too large to render on the UI thread: source, with the reason.
        let big = "x".repeat(wks_native::links::MAX_RENDERED_MARKDOWN_BYTES + 1);
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/BIG.md"),
            2,
            false,
            None,
            serde_json::json!({"contents": big, "size": big.len()}),
        );
        visual.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.mode(), file_viewer::Mode::Source);
            assert!(!pane.previewable());
            assert!(pane.notice().unwrap().contains("opens as source"));
        });
        visual.simulate_keystrokes("ctrl-shift-v");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Source));
        assert!(visual.debug_bounds("file-viewer-text").is_some());
    }

    #[gpui::test]
    fn viewer_ignores_stale_states_and_reports_interrupted_loads(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        let text = |s: &str| serde_json::json!({"contents": s, "size": s.len()});
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/r/new.rs"),
            5,
            false,
            None,
            text("new"),
        );
        // A late answer to an older request never replaces newer content.
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/r/old.rs"),
            4,
            false,
            None,
            text("old"),
        );
        let pane = pane_of(&workspace, &visual);
        visual.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.state().number, 5);
            assert_eq!(pane.editor().unwrap().read(cx).value().as_ref(), "new");
        });
        // A read in flight that the controller drops (session switch)
        // stops loading and says so instead of spinning forever.
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/r/next.md"),
            6,
            true,
            None,
            serde_json::json!({}),
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.remove("file-preview");
                view.selected = Some("b".into());
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let state = pane.read(cx).state();
            assert!(!state.loading);
            assert!(state.error.as_deref().unwrap().contains("Loading stopped"));
        });
        assert!(visual.debug_bounds("file-viewer-error").is_some());
        // Closing dismisses it for good: the same state never reopens it.
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/r/next.md"),
            6,
            false,
            None,
            text("late"),
        );
        workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
    }

    #[gpui::test]
    fn docked_viewer_keeps_the_chat_usable_and_routes_keys_by_focus(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(1400.), px(800.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("DOCKED_DRAFT");
        visual.run_until_parked();
        let opened_at = std::time::Instant::now();
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/guide.md"),
            1,
            false,
            None,
            serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
        );
        let panel = visual
            .debug_bounds("file-viewer-panel")
            .expect("docked beside the chat");
        let opening = opened_at.elapsed();
        assert!(
            visual.debug_bounds("file-viewer-backdrop").is_none(),
            "docked is not modal"
        );
        let composer = visual
            .debug_bounds("chat-composer")
            .expect("composer still shown");
        assert!(
            composer.right() <= panel.left() + px(1.),
            "the chat sits beside the viewer"
        );
        assert!(composer.size.width >= px(300.));
        // It slides in from the right: its first frame is still narrow, and
        // it reaches its full width once the slide is over.
        settle(&mut visual);
        let full = visual.debug_bounds("file-viewer-panel").unwrap();
        assert!(full.size.width > px(300.));
        assert_eq!(full.right(), panel.right(), "anchored to the right edge");
        assert!(
            panel.size.width < full.size.width - px(1.)
                || opening >= std::time::Duration::from_millis(240),
            "first frame {:?} of {:?} after {opening:?}",
            panel.size.width,
            full.size.width
        );
        visual.update(|window, cx| {
            let this = workspace.read(cx);
            assert!(!this.viewer_modal(window));
            assert!(
                this.composer.read(cx).focus_handle(cx).is_focused(window),
                "content arriving beside a draft never takes the keyboard"
            );
        });
        // Composer focused: sending is intentional and works.
        visual.simulate_keystrokes("ctrl-enter");
        visual.run_until_parked();
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Act { action: Action::Send(text), .. }) if text == "DOCKED_DRAFT"
        ));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                // The send is acknowledged; the docked viewer stays open.
                let mut view = (*this.view).clone();
                view.busy = false;
                this.update_view(Arc::new(view), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("SECOND_DRAFT", window, cx))
            })
        });
        // Focus in the viewer: workspace shortcuts and typing stay out of
        // the composer.
        settle(&mut visual);
        let document = visual.debug_bounds("file-viewer-markdown").unwrap();
        visual.simulate_click(
            document.origin + gpui::point(px(200.), document.size.height - px(20.)),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert!(
                workspace.read(cx).viewer_has_focus(window, cx),
                "clicking the viewer focuses it"
            )
        });
        for keys in [
            "ctrl-enter",
            "alt-down",
            "ctrl-n",
            "ctrl-l",
            "tab",
            "x",
            "enter",
            "backspace",
        ] {
            visual.simulate_keystrokes(keys);
            visual.run_until_parked();
            assert!(commands.try_recv().is_err(), "{keys} reached the workspace");
            workspace.read_with(&visual, |this, cx| {
                assert_eq!(
                    this.composer.read(cx).value().as_ref(),
                    "SECOND_DRAFT",
                    "{keys}"
                );
                assert!(this.file_viewer().is_some(), "{keys} closed the viewer");
            });
            visual.update(|window, cx| {
                assert!(
                    workspace.read(cx).viewer_has_focus(window, cx),
                    "{keys} left the viewer"
                )
            });
        }
        // Clicking back into the composer is enough to chat again; no
        // modal guard pulls focus back to the viewer.
        let composer = visual.debug_bounds("chat-composer").unwrap();
        // The composer's text field is its first row.
        visual.simulate_click(
            composer.origin + gpui::point(px(60.), px(22.)),
            gpui::Modifiers::default(),
        );
        visual.simulate_input("!");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(
                this.composer.read(cx).value().contains('!'),
                "typing reaches the clicked composer"
            );
            assert!(this.file_viewer().is_some());
        });
        // Esc inside the viewer closes it and returns focus.
        let document = visual.debug_bounds("file-viewer-markdown").unwrap();
        visual.simulate_click(
            document.origin + gpui::point(px(200.), document.size.height - px(20.)),
            gpui::Modifiers::default(),
        );
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
        assert!(
            visual.debug_bounds("file-viewer-panel").is_none() || {
                // debug_bounds can outlive an element by a frame; the pane is gone.
                true
            }
        );
        visual.update(|window, cx| {
            assert!(!workspace.read(cx).viewer_has_focus(window, cx));
        });
    }

    #[gpui::test]
    fn viewer_watch_terminal_states_and_closed_channel_recover(cx: &mut TestAppContext) {
        use window_destroy::{Event, Watch};
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(1400.), px(800.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        for (index, terminal) in [
            None,
            Some(Event::Unidentified),
            Some(Event::Ambiguous),
            Some(Event::Failed("connection lost".into())),
            Some(Event::Destroyed),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let number = index as u64 + 1;
            preview_state(
                &workspace,
                &mut visual,
                "a",
                file_target("/repo/guide.md"),
                number,
                false,
                None,
                serde_json::json!({"contents": "# Guide", "size": 7}),
            );
            settle(&mut visual);
            visual.update(|window, cx| workspace.update(cx, |this, cx| this.pop_out(window, cx)));
            visual.run_until_parked();
            let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
            let (sender, events) = async_channel::bounded(2);
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    this.watch_popout(handle.window_id(), Watch::Started(events), window, cx)
                })
            });
            if index == 5 {
                sender.try_send(Event::Attached).unwrap();
            }
            if let Some(event) = terminal {
                sender.try_send(event).unwrap();
            }
            // No fake remove_window: GPUI still believes this window is live.
            assert_eq!(cx.windows().len(), 2);
            drop(sender);
            visual.run_until_parked();
            assert_eq!(
                cx.windows().len(),
                1,
                "terminal state {index} removes logical window"
            );
            workspace.read_with(&visual, |this, cx| {
                assert!(this.viewer_popout().is_none());
                assert_eq!(this.file_viewer().unwrap().read(cx).state().number, number);
            });
            settle(&mut visual);
            assert!(visual.debug_bounds("file-viewer-panel").is_some());
        }
    }

    #[gpui::test]
    fn viewer_pops_out_into_its_own_window_and_docks_back(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(1400.), px(800.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("POPOUT_DRAFT");
        let guide = file_target("/repo/docs/guide.md");
        preview_state(
            &workspace,
            &mut visual,
            "a",
            guide.clone(),
            1,
            false,
            None,
            serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
        );
        let main_pane = pane_of(&workspace, &visual);
        visual.update(|window, cx| {
            main_pane.update(cx, |pane, cx| {
                pane.set_mode(file_viewer::Mode::Source, window, cx)
            })
        });
        settle(&mut visual);
        let popout = visual
            .debug_bounds("file-viewer-popout")
            .expect("pop out button");
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 2, "a second window opened");
        let (handle, pane) = workspace.read_with(&visual, |this, _| {
            assert!(
                this.file_viewer().is_none(),
                "the main window's viewer moved out"
            );
            this.viewer_popout().expect("popped out")
        });
        visual.run_until_parked();
        assert!(
            visual.debug_bounds("file-viewer-panel").is_none()
                || workspace.read_with(&visual, |this, _| this.file_viewer().is_none())
        );
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        window.run_until_parked();
        assert!(window.debug_bounds("file-viewer-dock").is_some());
        assert!(
            window.debug_bounds("file-viewer-text").is_some(),
            "mode carried over"
        );
        window.update(|window, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.mode(), file_viewer::Mode::Source);
            assert_eq!(pane.state().number, 1);
            assert!(
                pane.has_focus(window, cx),
                "the new window focuses its content"
            );
        });
        // Appearance changes redraw the separate window too.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.set_appearance(Appearance::Light, window, cx)
            })
        });
        window.run_until_parked();
        assert!(window.debug_bounds("file-viewer-text").is_some());
        // The window's keys are its own: nothing reaches the workspace.
        for keys in ["ctrl-enter", "ctrl-n", "escape", "x"] {
            window.simulate_keystrokes(keys);
            window.run_until_parked();
            assert!(commands.try_recv().is_err(), "{keys} reached the workspace");
        }
        assert_eq!(
            cx.windows().len(),
            2,
            "Esc does not close a separate window"
        );
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "POPOUT_DRAFT")
        });
        // Main-window chat keeps working while it is open.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.composer.read(cx).focus_handle(cx).focus(window)
            })
        });
        visual.simulate_keystrokes("ctrl-enter");
        visual.run_until_parked();
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Act { action: Action::Send(text), .. }) if text == "POPOUT_DRAFT"
        ));
        // New links land in the popped-out window; switching sessions keeps it.
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/next.md"),
            2,
            false,
            None,
            serde_json::json!({"contents": "# Next\n", "size": 7}),
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.remove("file-preview");
                view.selected = Some("b".into());
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        window.run_until_parked();
        window.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.state().number, 2);
            assert_eq!(pane.mode(), file_viewer::Mode::Preview);
        });
        workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
        // Links in the popped-out document still read from the owning
        // session's machine, even though another session is selected.
        window.update(|window, cx| {
            pane.update(cx, |pane, cx| pane.follow_link("img/a.png", window, cx))
        });
        let (session, target) = preview_request(&mut commands).expect("popout links read files");
        assert_eq!(
            (session.as_str(), target.path.as_str()),
            ("a", "/repo/docs/img/a.png")
        );
        // Dock: the window closes and the viewer returns beside the chat.
        let dock = window.debug_bounds("file-viewer-dock").unwrap();
        window.simulate_click(dock.center(), gpui::Modifiers::default());
        window.run_until_parked();
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1, "docking closes the window");
        let docked = pane_of(&workspace, &visual);
        visual.update(|_, cx| {
            let pane = docked.read(cx);
            assert_eq!(pane.state().number, 2);
            assert_eq!(pane.mode(), file_viewer::Mode::Preview);
        });
        assert!(visual.debug_bounds("file-viewer-panel").is_some());
        // The OS close button docks back too, rather than losing the file.
        settle(&mut visual);
        let popout = visual.debug_bounds("file-viewer-popout").unwrap();
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        assert!(window.simulate_close(), "the window may close");
        window.run_until_parked();
        visual.run_until_parked();
        // The test platform leaves closing to the caller; a real one closes it.
        window.update(|window, _| window.remove_window());
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        workspace.read_with(&visual, |this, cx| {
            assert!(this.viewer_popout().is_none());
            assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 2);
        });
        // A window destroyed without a close request (no should-close
        // callback) docks back on the next update instead of swallowing files.
        settle(&mut visual);
        let popout = visual.debug_bounds("file-viewer-popout").unwrap();
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        window.update(|window, _| window.remove_window());
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/after.md"),
            7,
            false,
            None,
            serde_json::json!({"contents": "# After\n", "size": 8}),
        );
        workspace.read_with(&visual, |this, cx| {
            assert!(this.viewer_popout().is_none());
            assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 7);
        });
        assert!(visual.debug_bounds("file-viewer-panel").is_some());
        // The X11 server destroying the window behind GPUI's back leaves it
        // registered (GPUI 0.2 ignores DestroyNotify), so the cx.windows()
        // check above cannot see it. The platform watch reports it instead:
        // the viewer docks back with its mode and the dead window goes.
        settle(&mut visual);
        let popout = visual.debug_bounds("file-viewer-popout").unwrap();
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (handle, popped) =
            workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        window.update(|window, cx| {
            popped.update(cx, |pane, cx| {
                pane.set_mode(file_viewer::Mode::Source, window, cx)
            })
        });
        assert_eq!(cx.windows().len(), 2, "still registered, as on X11");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.popout_destroyed(handle.window_id(), window, cx)
            })
        });
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1, "the dead window is dropped");
        workspace.read_with(&visual, |this, cx| {
            assert!(this.viewer_popout().is_none());
            let pane = this.file_viewer().unwrap().read(cx);
            assert_eq!(pane.state().number, 7);
            assert_eq!(pane.mode(), file_viewer::Mode::Source);
        });
        visual.update(|window, cx| assert!(workspace.read(cx).viewer_has_focus(window, cx)));
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/destroyed.md"),
            8,
            false,
            None,
            serde_json::json!({"contents": "# Destroyed\n", "size": 12}),
        );
        workspace.read_with(&visual, |this, cx| {
            assert!(this.viewer_popout().is_none());
            assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 8);
        });
        assert!(visual.debug_bounds("file-viewer-panel").is_some());
        // A destroy reported after the window closed normally changes nothing.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.popout_destroyed(handle.window_id(), window, cx)
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 8)
        });
        // Its ✕ closes the viewer entirely.
        settle(&mut visual);
        let popout = visual.debug_bounds("file-viewer-popout").unwrap();
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        window.run_until_parked();
        let close = window.debug_bounds("file-viewer-close").unwrap();
        window.simulate_click(close.center(), gpui::Modifiers::default());
        window.run_until_parked();
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        workspace.read_with(&visual, |this, _| {
            assert!(this.viewer_popout().is_none() && this.file_viewer().is_none())
        });
        // Releasing the workspace closes a popped-out window with it.
        let popout_again = {
            preview_state(
                &workspace,
                &mut visual,
                "a",
                file_target("/repo/docs/last.md"),
                9,
                false,
                None,
                serde_json::json!({"contents": "# Last\n", "size": 7}),
            );
            settle(&mut visual);
            visual.debug_bounds("file-viewer-popout").unwrap()
        };
        visual.simulate_click(popout_again.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 2);
        visual.update(|_, cx| workspace.update(cx, |this, cx| this.close_popout_window(cx)));
        visual.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
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
        let sidebar = visual.debug_bounds("session-sidebar").unwrap();
        assert!(reset.left() >= sidebar.left() && reset.right() <= sidebar.right());
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
        assert!(matches!(next_effect(&mut commands).unwrap(), Command::Select(id) if id == "b"));
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.project_filter.as_deref(), Some("/two/app"));
            assert_eq!(this.visible_sessions(cx), vec![1]);
        });
        visual.simulate_keystrokes("n");
        workspace.read_with(&visual, |this, _| {
            assert!(this.new_session);
            assert_eq!(this.provider, "codex");
            assert_eq!(this.projects.cwd.as_str(), "/two/app");
        });
        visual.simulate_input("jkgn");
        workspace.read_with(&visual, |this, cx| {
            assert!(this.prompt.read(cx).value().ends_with("jkgn"))
        });
        assert!(
            next_effect(&mut commands).is_none(),
            "typing in form must not launch or select agents"
        );
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
    fn settings_categories_and_search_filter_preferences(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("g s");
        visual.run_until_parked();
        // One category at a time; Appearance first.
        assert!(visual.debug_bounds("setting-theme").is_some());
        assert!(visual.debug_bounds("setting-vim").is_none());
        // j / k step through categories in Normal mode.
        visual.simulate_keystrokes("j");
        visual.run_until_parked();
        assert!(visual.debug_bounds("setting-interface-font").is_some());
        assert!(visual.debug_bounds("setting-theme").is_none());
        let keyboard = visual.debug_bounds("settings-nav-Keyboard").unwrap();
        visual.simulate_click(keyboard.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("setting-vim").is_some());
        // `/` searches every category at once.
        visual.simulate_keystrokes("/");
        visual.simulate_input("clock");
        visual.run_until_parked();
        // Results span categories (debug bounds outlive removed rows, so
        // filtering itself is pinned by `settings_match`'s unit test).
        assert!(visual.debug_bounds("setting-clock").is_some());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.settings_search
                    .update(cx, |input, cx| input.set_value("font mono", window, cx));
                cx.notify();
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("setting-code-font").is_some());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.settings_search
                    .update(cx, |input, cx| input.set_value("zzzz", window, cx));
                cx.notify();
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("settings-no-results").is_some());
        // Picking a category clears the search.
        let chat = visual.debug_bounds("settings-nav-Chat").unwrap();
        visual.simulate_click(chat.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("setting-merge-turn").is_some());
        workspace.read_with(&visual, |this, cx| {
            assert!(this.settings_search.read(cx).value().is_empty())
        });
    }

    #[gpui::test]
    fn project_bookmark_can_be_saved_without_launching_an_agent(cx: &mut TestAppContext) {
        use wks_native::features::{Request, RequestState};
        use wks_native::projects::Patch;
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
            })
        });
        visual.simulate_keystrokes("g p i");
        visual.simulate_input("/work/jk-project/");
        visual.simulate_keystrokes("ctrl-enter");
        workspace.read_with(&visual, |this, cx| {
            assert!(this.project_path.read(cx).value().is_empty());
            assert!(
                this.settings.bookmarks("test").is_empty(),
                "the hub is asked first"
            );
        });
        let Some(Command::Request(Request::SaveProject { path, change })) =
            next_effect(&mut commands)
        else {
            panic!("pinning asks the hub's registry")
        };
        assert_eq!(
            (path.as_str(), &change),
            ("/work/jk-project", &Patch::Pin(true))
        );
        assert!(next_effect(&mut commands).is_none(), "nothing launches");
        let receipt = |number: u64, error: Option<&str>, value: serde_json::Value| {
            let (path, change, error) = (path.clone(), change.clone(), error.map(str::to_owned));
            let workspace = workspace.clone();
            move |window: &mut Window, cx: &mut App| {
                workspace.update(cx, |this, cx| {
                    let mut view = (*this.view).clone();
                    view.requests.insert(
                        "project-save",
                        RequestState {
                            number,
                            request: Request::SaveProject { path, change },
                            loading: false,
                            value: Arc::new(value),
                            error,
                        },
                    );
                    this.update_view(Arc::new(view), window, cx);
                })
            }
        };
        // A refused write is never shown as saved; the device keeps it on request.
        visual.update(receipt(
            7,
            Some("operator scope required"),
            serde_json::Value::Null,
        ));
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.projects.notice.contains("operator scope required"));
            assert_eq!(this.projects.fallback.as_deref(), Some("/work/jk-project"));
            assert!(
                !this
                    .known_projects()
                    .iter()
                    .any(|p| p.path == "/work/jk-project")
            );
        });
        let keep = visual.debug_bounds("keep-on-device-projects").unwrap();
        visual.simulate_click(keep.center(), gpui::Modifiers::default());
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.settings.bookmarks("test"), ["/work/jk-project"]);
            assert!(
                this.known_projects()
                    .iter()
                    .any(|p| p.path == "/work/jk-project")
            );
        });
        // A verified hub save becomes the registry the list is drawn from.
        visual.update(receipt(
            8,
            None,
            serde_json::json!({"revision":1,"projects":{"/work/hub-only":{"favourite":true}},"favourites":[],"recent":[],"configured":[]}),
        ));
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.projects.notice, "Pinned.");
            let first = &this.known_projects()[0];
            assert_eq!(
                (first.path.as_str(), first.favourite),
                ("/work/hub-only", true)
            );
        });
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
        workspace.read_with(&visual, |this, _| {
            assert!(this.new_session);
            assert!(this.requested_session.is_none());
            assert_eq!(this.projects.cwd.as_str(), "/work/project");
        });
        while let Ok(command) = commands.try_recv() {
            assert!(
                matches!(command, Command::Refresh) || project_read(&command),
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
        // The acknowledged launch marks its project recently used on the hub,
        // exactly once, and does nothing else.
        let Ok(Command::Request(wks_native::features::Request::TouchProject { path, at })) =
            commands.try_recv()
        else {
            panic!("an acknowledged launch records its project")
        };
        assert_eq!(path, "/work/project");
        assert!(at > 0);
        assert!(commands.try_recv().is_err());
        // A failed launch keeps nothing: a later receipt for a launch this
        // window did not start (another window's) records no project.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.spawn_pending = true;
                this.launched_cwd = "/work/failed".into();
                let mut failed = state("b");
                failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 2,
                    session: None,
                    error: Some("provider executable not found".into()),
                    unsent_message: None,
                });
                this.update_view(Arc::new(failed), window, cx);
                let mut other = state("c");
                other.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 3,
                    session: Some("c".into()),
                    error: None,
                    unsent_message: None,
                });
                this.update_view(Arc::new(other), window, cx);
            })
        });
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).all(|c| !matches!(
                c,
                Command::Request(wks_native::features::Request::TouchProject { .. })
            )),
            "another window's launch never records this window's failed folder"
        );
    }

    #[gpui::test]
    fn guided_launch_keeps_options_and_start_action_accessible(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(1000.), px(1100.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut view = state("a");
                Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
                Arc::make_mut(&mut view.sessions)[1].cwd = "/work/beta".into();
                this.update_view(Arc::new(view), window, cx);
                this.show_new_session(window, cx);
                this.prompt
                    .update(cx, |input, cx| input.set_value("Fix the tests", window, cx));
            });
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("launch-details").is_none());
        let customize = visual.debug_bounds("launch-customize").unwrap();
        visual.simulate_click(customize.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("launch-details").is_some());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.label
                    .update(cx, |input, cx| input.set_value("Test repair", window, cx));
                this.permission = Permission::Plan;
            });
        });
        visual.simulate_click(customize.center(), gpui::Modifiers::default());
        let provider = visual.debug_bounds("launch-provider-codex").unwrap();
        visual.simulate_click(provider.center(), gpui::Modifiers::default());
        // The open conversation's folder is preselected; Change swaps it.
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.projects.cwd, "/work/alpha")
        });
        visual.run_until_parked();
        let change = visual.debug_bounds("launch-project-change").unwrap();
        visual.simulate_click(change.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let project = visual.debug_bounds("launch-project-row-1").unwrap();
        visual.simulate_click(project.center(), gpui::Modifiers::default());
        workspace.read_with(&visual, |this, cx| {
            assert!(!this.launch_details_open);
            assert_eq!(this.permission, Permission::Ask);
            assert_eq!(this.provider, "codex");
            assert_eq!(this.projects.cwd.as_str(), "/work/beta");
            assert_eq!(this.label.read(cx).value().as_str(), "Test repair");
            assert_eq!(this.prompt.read(cx).value().as_str(), "Fix the tests");
        });
        assert!(next_effect(&mut commands).is_none());
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.run_until_parked();
        let start = visual.debug_bounds("launch-start").unwrap();
        assert!(start.bottom() <= px(480.) && start.right() <= px(720.));
        assert!(start.top() >= px(0.));
        visual.simulate_click(start.center(), gpui::Modifiers::default());
        let Command::Create(request) = commands.try_recv().unwrap() else {
            panic!("expected launch")
        };
        assert_eq!(request.provider, "codex");
        assert_eq!(request.cwd, "/work/beta");
        assert_eq!(request.label, "Test repair");
        assert_eq!(request.message, "Fix the tests");
        assert!(commands.try_recv().is_err());
    }

    /// The hub's outcome-unknown launch failure at the smallest supported
    /// window: the recovery step is a whole, unclipped line above a visible
    /// Start action, and every word of the hub's text can be scrolled to.
    #[gpui::test]
    fn uncertain_launch_error_keeps_recovery_and_full_text_reachable(cx: &mut TestAppContext) {
        const ERROR: &str = "launch admission may have executed for session 480e8332-3423-44e7-9c18-2b7e274ea3f9: daemon returned 503 Service Unavailable: execution engine unavailable: provider executable not found or not executable; execution outcome is unknown; inspect its outcome before retrying; cleanup: daemon returned 404 Not Found: no wrapper attached";
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                this.projects.cwd = "/work/api".into();
                let mut failed = state("a");
                failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 1,
                    session: None,
                    error: Some(ERROR.into()),
                    unsent_message: None,
                });
                this.update_view(Arc::new(failed), window, cx);
            });
        });
        visual.run_until_parked();
        let window = gpui::Bounds::new(gpui::point(px(0.), px(0.)), size(px(720.), px(480.)));
        let inside = |b: gpui::Bounds<gpui::Pixels>| {
            b.top() >= window.top()
                && b.bottom() <= window.bottom()
                && b.left() >= window.left()
                && b.right() <= window.right()
        };
        let card = visual.debug_bounds("launch-error").unwrap();
        let headline = visual.debug_bounds("launch-error-headline").unwrap();
        let guidance = visual.debug_bounds("launch-error-guidance").unwrap();
        let start = visual.debug_bounds("launch-start").unwrap();
        for (name, b) in [
            ("card", card),
            ("headline", headline),
            ("guidance", guidance),
            ("start", start),
        ] {
            assert!(inside(b), "{name} {b:?} leaves the window");
        }
        // The guidance may wrap; its own box holds every line, and the card
        // encloses it rather than clipping it.
        assert!(guidance.bottom() <= card.bottom() && card.bottom() <= start.top());
        // The hub's wording is longer than its region; scrolling reaches the end.
        let region = visual.debug_bounds("launch-error-details").unwrap();
        let text = visual.debug_bounds("launch-error-text").unwrap();
        assert!(inside(region), "{region:?}");
        // Every wrapped line of the whole string is laid out: the box is as
        // tall as GPUI's own unclamped wrap of it at that width.
        let (lines, line_height) = visual.update(|window, _| {
            let mut style = window.text_style();
            style.font_size = px(12.).into();
            let shaped = window
                .text_system()
                .shape_text(
                    ERROR.into(),
                    px(12.),
                    &[style.to_run(ERROR.len())],
                    Some(text.size.width),
                    None,
                )
                .unwrap();
            let lines: usize = shaped.iter().map(|l| l.wrap_boundaries().len() + 1).sum();
            (lines, style.line_height_in_pixels(window.rem_size()))
        });
        assert!(
            lines > 3,
            "a long error wraps past any small clamp: {lines}"
        );
        assert!(
            text.size.height >= line_height * (lines as f32 - 0.5),
            "{text:?} shows fewer than {lines} lines of {line_height:?}"
        );
        assert!(
            text.size.height > region.size.height,
            "{text:?} in {region:?}"
        );
        assert!(
            text.bottom() > region.bottom(),
            "the end starts out of view"
        );
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: region.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-400.))),
            ..Default::default()
        });
        visual.run_until_parked();
        let scrolled = visual.debug_bounds("launch-error-text").unwrap();
        assert!(
            scrolled.bottom() <= region.bottom() + px(1.) && scrolled.bottom() > region.top(),
            "end of {scrolled:?} reachable in {region:?}"
        );
        assert_eq!(visual.debug_bounds("launch-start").unwrap(), start);
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.spawn_error, ERROR, "nothing is truncated in state");
            assert!(wks_native::launch::uncertain_outcome(&this.spawn_error));
        });
        // Still allowed after checking: one click is one launch.
        visual.simulate_click(start.center(), gpui::Modifiers::default());
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("expected one create command")
        };
        assert_eq!(request.cwd, "/work/api");
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
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("expected one create command")
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
                assert_eq!(this.projects.cwd.as_str(), "/work/project");
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
        visual.simulate_keystrokes("tab tab tab");
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
        // Projects, history, toggle, search, then the first session.
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
    fn consecutive_tools_group_expand_and_keep_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 3,
            first_seq: 1,
            items: (0..3)
                .map(|i| Item {
                    kind: "tool_use".into(),
                    id: format!("call-{i}"),
                    name: "Read".into(),
                    input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
                    ..Default::default()
                })
                .collect(),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("keep my draft", window, cx));
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("tool-activity-group").is_some());
        // Work cards open with one-line steps; the header collapses them.
        assert!(visual.debug_bounds("tool-toggle-0").is_some());
        let open = visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height;
        let toggle = visual.debug_bounds("work-card-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height
                < open
        );
        let toggle = visual.debug_bounds("work-card-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let tool = visual.debug_bounds("tool-toggle-0").unwrap();
        visual.simulate_click(tool.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "keep my draft");
            assert_eq!(this.tool_expansion.get("call:call-0"), Some(&true));
        });
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn composer_shows_context_meter_once_the_runtime_reports(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("context-meter").is_none());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
                    "statusLine": {"contextUsedPct": 73.0, "contextWindowSize": 200000}
                }));
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
        let meter = visual.debug_bounds("context-meter").unwrap();
        let composer = visual.debug_bounds("chat-composer").unwrap();
        assert!(composer.contains(&meter.center()));
    }

    #[gpui::test]
    fn sidebar_shows_measured_usage_windows_per_account(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-usage").is_none());
        let now = chrono::Utc::now().timestamp();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.usage = Some(Arc::new(
                    serde_json::json!({"providers":[{"provider":"claude","accounts":[
                        {"label":"default","is_default":true,"windows":{"five_hour":{
                            "used_percent":{"state":"ok","value":42.0},"resets_at":now + 3600}}}
                    ]}]}),
                ));
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
        let usage = visual.debug_bounds("sidebar-usage").unwrap();
        assert!(usage.size.height > px(0.));
        // Hover shows every window as a card.
        visual.simulate_mouse_move(usage.center(), None, gpui::Modifiers::default());
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(800));
        visual.run_until_parked();
        assert!(visual.debug_bounds("usage-hover-card").is_some());
        // Click opens the detail modal and asks for a fresh reading.
        visual.simulate_click(usage.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.usage_open));
        assert!(matches!(commands.try_recv(), Ok(Command::RefreshUsage)));
        // Clicks inside the card do not dismiss it; the close button does.
        let modal = visual.debug_bounds("usage-modal").unwrap();
        visual.simulate_click(modal.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.usage_open));
        let close = visual.debug_bounds("usage-close").unwrap();
        visual.simulate_click(close.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(!this.usage_open));
        // Esc closes it too.
        visual.simulate_click(usage.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("escape");
        workspace.read_with(&visual, |this, _| assert!(!this.usage_open));
    }

    struct CaptionPreview;

    impl CaptionPreview {
        fn new() -> Self {
            chrome::FORCE_CAPTION.set(true);
            Self
        }
    }

    impl Drop for CaptionPreview {
        fn drop(&mut self) {
            chrome::FORCE_CAPTION.set(false);
        }
    }

    fn caption_mouse_down(
        visual: &mut VisualTestContext,
        position: gpui::Point<gpui::Pixels>,
    ) -> (bool, bool) {
        chrome::DRAG_HIT.set(false);
        visual.simulate_mouse_down(
            position,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        let default_prevented = visual.update(|window, _| window.default_prevented());
        (chrome::DRAG_HIT.get(), default_prevented)
    }

    #[gpui::test]
    fn app_drawn_caption_drag_surfaces_survive_layouts_and_exclude_controls(
        cx: &mut TestAppContext,
    ) {
        let _caption = CaptionPreview::new();
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
            })
        });
        for (width, sidebar_width) in [
            (720., 200.),
            (720., 304.),
            (720., 520.),
            (1000., 304.),
            (1600., 520.),
        ] {
            visual.update(|_, cx| {
                workspace.update(cx, |this, cx| {
                    this.settings.sidebar_width = sidebar_width;
                    cx.notify();
                })
            });
            visual.simulate_resize(size(px(width), px(700.)));
            for collapsed in [false, true] {
                for screen in [
                    Screen::Conversation,
                    Screen::Projects,
                    Screen::Settings,
                    Screen::Recent,
                    Screen::Changes,
                    Screen::History,
                    Screen::Session,
                    Screen::Setup,
                    Screen::Model,
                ] {
                    // Also exercise New Session, which takes a separate render branch.
                    for new_session in [false, true] {
                        visual.update(|_, cx| {
                            workspace.update(cx, |this, cx| {
                                this.sidebar_collapsed = collapsed;
                                this.screen = screen;
                                this.new_session = new_session;
                                cx.notify();
                            })
                        });
                        visual.run_until_parked();
                        let drag = visual.debug_bounds("sidebar-drag-region").unwrap();
                        assert!(
                            drag.size.width >= px(40.) && drag.size.height >= px(32.),
                            "positive, practical sidebar drag bounds: {drag:?}"
                        );
                        let grab = if collapsed {
                            drag.center()
                        } else {
                            let actions = visual.debug_bounds("new-session-button").unwrap();
                            assert!(
                                actions.left() - drag.left() >= px(40.),
                                "at least 40px remains beside the controls at minimum width"
                            );
                            for point in [
                                gpui::point(drag.left() + px(6.), drag.top() + px(6.)),
                                gpui::point(drag.left() + px(38.), drag.bottom() - px(6.)),
                            ] {
                                let (hit, prevented) = caption_mouse_down(&mut visual, point);
                                assert!(hit && !prevented, "row padding is usable chrome");
                            }
                            gpui::point(drag.left() + px(24.), drag.center().y)
                        };
                        let (hit, default_prevented) = caption_mouse_down(&mut visual, grab);
                        assert!(
                            hit,
                            "sidebar grab reachable: {width} {collapsed} {screen:?}"
                        );
                        assert!(
                            !default_prevented,
                            "Windows must be allowed to start the native move"
                        );
                        for selector in [
                            "sidebar-toggle",
                            "new-session-button",
                            "caption-minimize",
                            "caption-maximize",
                            "caption-close",
                        ] {
                            let control = visual.debug_bounds(selector).unwrap();
                            assert!(control.size.width > px(0.) && control.size.height > px(0.));
                            assert!(
                                !caption_mouse_down(&mut visual, control.center()).0,
                                "{selector} must exclude native Drag"
                            );
                        }
                        let caption = visual.debug_bounds("window-caption").unwrap();
                        assert_eq!(caption.top(), px(0.));
                        assert_eq!(caption.right(), px(width));
                        assert_eq!(
                            visual.debug_bounds("caption-close").unwrap().right(),
                            px(width)
                        );
                        if screen == Screen::Conversation && !new_session {
                            let pill = visual.debug_bounds("title-bar").unwrap();
                            assert!(pill.right() <= caption.left(), "title pill clears caption");
                            assert!(
                                !caption_mouse_down(&mut visual, pill.center()).0,
                                "the occluding pill excludes its underlying drag hitbox"
                            );
                            let header = visual.debug_bounds("chat-drag-region").unwrap();
                            let (hit, default_prevented) = caption_mouse_down(
                                &mut visual,
                                gpui::point(header.left() + px(8.), header.top() + px(8.)),
                            );
                            assert!(hit && !default_prevented);
                            assert!(
                                !caption_mouse_down(
                                    &mut visual,
                                    gpui::point(header.center().x, header.bottom() + px(20.))
                                )
                                .0,
                                "content below the chrome must not drag"
                            );
                        }
                    }
                }
            }
        }
        // The toggle still performs its normal action with the native chrome present.
        let toggle = visual.debug_bounds("sidebar-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(!this.sidebar_collapsed));
    }

    #[gpui::test]
    fn secondary_pages_keep_actions_clear_of_the_caption_and_drag_from_the_top(
        cx: &mut TestAppContext,
    ) {
        let _caption = CaptionPreview::new();
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
            })
        });
        for width in [720., 1200.] {
            visual.simulate_resize(size(px(width), px(700.)));
            for (screen, new_session) in [
                (Screen::Projects, false),
                (Screen::Settings, false),
                (Screen::Recent, false),
                (Screen::Changes, false),
                (Screen::History, false),
                (Screen::Session, false),
                (Screen::Setup, false),
                (Screen::Model, false),
                (Screen::Conversation, true),
            ] {
                visual.update(|_, cx| {
                    workspace.update(cx, |this, cx| {
                        this.screen = screen;
                        this.new_session = new_session;
                        cx.notify();
                    })
                });
                visual.run_until_parked();
                let caption = visual.debug_bounds("window-caption").unwrap();
                let title = visual.debug_bounds("page-title").unwrap();
                assert!(
                    title.top() >= caption.bottom(),
                    "{screen:?} title starts below the caption strip at {width}"
                );
                for selector in ["page-actions", "feature-back"] {
                    if let Some(control) = visual.debug_bounds(selector) {
                        assert!(
                            !control.intersects(&caption),
                            "{selector} on {screen:?} sits under the caption at {width}"
                        );
                    }
                }
                let strip = visual.debug_bounds("page-drag-region").unwrap();
                assert!(
                    strip.right() <= caption.left(),
                    "strip stops at the caption"
                );
                let (hit, prevented) = caption_mouse_down(
                    &mut visual,
                    gpui::point(strip.center().x, strip.top() + px(8.)),
                );
                assert!(hit && !prevented, "{screen:?} top strip drags the window");
                assert!(
                    !caption_mouse_down(&mut visual, title.center()).0,
                    "page content below the strip never drags"
                );
            }
        }
    }

    #[gpui::test]
    fn file_viewer_controls_stay_clear_of_the_caption(cx: &mut TestAppContext) {
        let _caption = CaptionPreview::new();
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        // Docked beside the chat, then the modal sheet of a narrow window.
        for (width, number) in [(1400., 1), (720., 2)] {
            visual.simulate_resize(size(px(width), px(800.)));
            preview_state(
                &workspace,
                &mut visual,
                "a",
                file_target("/repo/docs/guide.md"),
                number,
                false,
                None,
                serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
            );
            settle(&mut visual);
            let caption = visual.debug_bounds("window-caption").unwrap();
            let close = visual.debug_bounds("file-viewer-close").unwrap();
            assert!(
                close.top() >= caption.bottom(),
                "viewer close {close:?} under the caption {caption:?} at {width}"
            );
            if width < 1000. {
                let backdrop = visual.debug_bounds("file-viewer-backdrop").unwrap();
                assert!(
                    backdrop.top() >= caption.bottom(),
                    "the sheet leaves the window buttons usable"
                );
            }
        }
    }

    #[gpui::test]
    fn narrow_settings_rail_keeps_every_category_reachable(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.settings.sidebar_width = 304.;
                this.show_screen(Screen::Settings, window, cx);
            })
        });
        visual.simulate_resize(size(px(760.), px(600.)));
        visual.run_until_parked();
        let rail = visual.debug_bounds("settings-rail").unwrap();
        assert_eq!(rail.size.width, px(40.), "icon rail beside a wide sidebar");
        let keyboard = visual.debug_bounds("settings-nav-Keyboard").unwrap();
        assert!(keyboard.size.width > px(0.) && rail.contains(&keyboard.center()));
        visual.simulate_click(keyboard.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.settings_section, settings::SettingsSection::Keyboard)
        });
        visual.simulate_resize(size(px(1400.), px(800.)));
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("settings-rail").unwrap().size.width,
            px(200.),
            "labelled rail when there is room"
        );
    }

    #[gpui::test]
    fn enter_saves_the_session_name_on_this_device(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.open_feature(Screen::Session, window, cx);
            })
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.extras
                    .name
                    .update(cx, |input, cx| input.set_value("", window, cx))
            })
        });
        visual.run_until_parked();
        visual.simulate_input("Release prep");
        visual.run_until_parked();
        // A single-line Input propagates Enter after emitting PressEnter; the
        // test platform then types the unhandled key as "\n", which a real
        // platform does not. Dispatch the Input's own Enter action instead.
        visual.dispatch_action(gpui_component::input::Enter { secondary: false });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            let saved = this
                .settings
                .names
                .get(&this.project_scope)
                .and_then(|names| names.get("a"));
            assert_eq!(saved.map(String::as_str), Some("Release prep"));
            assert_eq!(this.extras.notice, "Name saved");
            assert_eq!(this.screen, Screen::Session, "saving keeps the page open");
        });
    }

    #[gpui::test]
    fn ending_a_session_asks_first_and_cancel_sends_nothing(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.open_feature(Screen::Session, window, cx);
            })
        });
        visual.run_until_parked();
        while commands.try_recv().is_ok() {}
        let click = |visual: &mut VisualTestContext, selector: &'static str| {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is rendered"));
            visual.simulate_click(bounds.center(), gpui::Modifiers::default());
            visual.run_until_parked();
        };
        let confirming = |visual: &mut VisualTestContext| {
            workspace.read_with(visual, |this, _| this.extras.confirm_end.clone())
        };
        assert_eq!(confirming(&mut visual), None);
        click(&mut visual, "end-session");
        assert_eq!(confirming(&mut visual).as_deref(), Some("a"));
        assert!(visual.debug_bounds("confirm-end-panel").is_some());
        click(&mut visual, "cancel-end");
        assert_eq!(confirming(&mut visual), None);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .all(|c| !matches!(c, Command::Act { .. })),
            "cancelling never reaches the agent"
        );
        click(&mut visual, "end-session");
        click(&mut visual, "confirm-end");
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                c,
                Command::Act { ref session, action: Action::Terminate } if session == "a"
            )),
            "confirming ends exactly the selected session"
        );
    }

    #[gpui::test]
    fn original_non_occluding_drag_is_cancelled_by_shell_focus(cx: &mut TestAppContext) {
        // Reproduce cd7a5028's event path without requiring a Windows window.
        // Native WM_NCLBUTTONDOWN checks this exact DispatchEventResult before
        // delegating HTCAPTION to DefWindowProc.
        struct DragFixture {
            focus: FocusHandle,
            occlude: bool,
        }
        impl Render for DragFixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().track_focus(&self.focus).child(
                    div()
                        .w(px(200.))
                        .h(px(52.))
                        .window_control_area(gpui::WindowControlArea::Drag)
                        .when(self.occlude, |d| d.occlude()),
                )
            }
        }
        let mut view = None;
        let window = cx.add_window(|_, cx| {
            view = Some(cx.entity());
            DragFixture {
                focus: cx.focus_handle(),
                occlude: false,
            }
        });
        let view = view.unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let point = gpui::point(px(24.), px(24.));
        let (_, before_prevented) = caption_mouse_down(&mut visual, point);
        assert!(before_prevented, "original shell focus cancels native move");
        visual.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.occlude = true;
                cx.notify();
            })
        });
        visual.run_until_parked();
        let (_, default_prevented) = caption_mouse_down(&mut visual, point);
        assert!(
            !default_prevented,
            "occluding drag hitbox excludes the focusable shell"
        );
    }

    #[gpui::test]
    fn viewed_subagent_reads_as_its_own_chat_without_a_composer(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                Arc::make_mut(&mut next.sessions)[0].subagents = serde_json::json!([{
                    "id": "task-1", "description": "Audit the parser", "status": "running", "model": "claude-sonnet-4-6"
                }]);
                next.child = Some(wks_native::controller::ChildTarget {
                    parent: "a".into(),
                    agent: "task-1".into(),
                });
                next.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: "Reading the parser now.".into(),
                        ..Default::default()
                    }],
                });
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("child-title-bar").is_some());
        assert!(visual.debug_bounds("child-read-only-bar").is_some());
        assert!(
            visual.debug_bounds("chat-composer").is_none(),
            "subagents take no input"
        );
        let back = visual.debug_bounds("child-back-parent").unwrap();
        visual.simulate_click(back.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::ViewChild(None)
        ));
    }

    #[gpui::test]
    fn subagent_overview_holds_its_place_and_collapses_when_all_finish(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        let say = |text: &str, at: &str| Item {
            kind: "assistant_text".into(),
            text: text.into(),
            timestamp: Some(format!("2026-10-02T10:00:{at}Z")),
            ..Default::default()
        };
        let ask = |text: &str, at: &str| Item {
            kind: "user_message".into(),
            text: text.into(),
            timestamp: Some(format!("2026-10-02T10:00:{at}Z")),
            ..Default::default()
        };
        let render = |items: Vec<Item>, status: &str, visual: &mut gpui::VisualTestContext| {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    let mut next = state("a");
                    Arc::make_mut(&mut next.sessions)[0].subagents = serde_json::json!([
                        {"id":"t1","description":"Audit the parser","status":status,"startedAt":"2026-10-02T10:00:15Z"},
                        {"id":"t2","description":"Check the tests","status":status,"startedAt":"2026-10-02T10:00:16Z"}
                    ]);
                    next.transcript.snapshot(ConversationSnapshot {
                        seq: items.len() as u64,
                        first_seq: 1,
                        items,
                    });
                    this.update_view(Arc::new(next), window, cx)
                })
            });
            visual.run_until_parked();
        };
        render(
            vec![
                ask("Audit everything", "10"),
                say("Spawning two agents.", "12"),
            ],
            "running",
            &mut visual,
        );
        // Both started after row 1 and before anything later: one card there.
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.child_ui.overview.keys().copied().collect::<Vec<_>>(),
                [1]
            );
            assert_eq!(this.child_ui.overview[&1].len(), 2);
        });
        let running = visual
            .debug_bounds("subagent-overview")
            .unwrap()
            .size
            .height;
        // New messages land below the card instead of under it.
        render(
            vec![
                ask("Audit everything", "10"),
                say("Spawning two agents.", "12"),
                say("Still waiting on both.", "40"),
            ],
            "running",
            &mut visual,
        );
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.child_ui.overview.keys().copied().collect::<Vec<_>>(),
                [1]
            );
        });
        // All finished: a collapsed summary.
        render(
            vec![
                ask("Audit everything", "10"),
                say("Spawning two agents.", "12"),
                say("Still waiting on both.", "40"),
            ],
            "completed",
            &mut visual,
        );
        let finished = visual
            .debug_bounds("subagent-overview")
            .unwrap()
            .size
            .height;
        assert!(finished < running, "the finished overview collapses");
        // It can be reopened.
        let toggle = visual.debug_bounds("subagent-overview-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("subagent-overview")
                .unwrap()
                .size
                .height
                > finished
        );
    }

    #[gpui::test]
    fn finished_native_subagents_leave_the_sidebar_once_the_parent_turn_ends(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        let rows = |parent_state: &str,
                    child_status: &str,
                    visual: &mut gpui::VisualTestContext| {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    let mut next = state("b");
                    let parent = &mut Arc::make_mut(&mut next.sessions)[0];
                    parent.state = parent_state.into();
                    parent.subagents = serde_json::json!([{"id":"t1","description":"Audit","status":child_status}]);
                    this.update_view(Arc::new(next), window, cx);
                    this.sidebar_rows(cx).len()
                })
            })
        };
        // Sessions a and b, plus the subagent row while it matters.
        assert_eq!(rows("input", "running", &mut visual), 3, "running: shown");
        assert_eq!(
            rows("responding", "complete", &mut visual),
            3,
            "parent mid-turn: shown"
        );
        assert_eq!(
            rows("input", "complete", &mut visual),
            2,
            "done and turn over: gone"
        );
        // Viewing it keeps it, so the highlight has somewhere to live.
        let kept = visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = (*this.view).clone();
                next.child = Some(wks_native::controller::ChildTarget {
                    parent: "a".into(),
                    agent: "t1".into(),
                });
                this.update_view(Arc::new(next), window, cx);
                this.sidebar_rows(cx).len()
            })
        });
        assert_eq!(kept, 3);
    }

    #[gpui::test]
    fn interface_size_zooms_layout_and_widgets_together(cx: &mut TestAppContext) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                set_zoom(1.);
            }
        }
        let _reset = Reset;
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.run_until_parked();
        let before = visual.debug_bounds("session-sidebar").unwrap().size.width;
        // Ctrl + steps to the next offered size.
        visual.simulate_keystrokes("ctrl-=");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.settings.interface_scale, 110);
            // gpui-component sizes widgets from the theme font size (rem).
            assert_eq!(
                gpui_component::Theme::global(cx).font_size,
                gpui::px(15. * 1.1)
            );
        });
        let after = visual.debug_bounds("session-sidebar").unwrap().size.width;
        assert!((f32::from(after) - f32::from(before) * 1.1).abs() < 1.);
        // The stored width stays in unzoomed units.
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.settings.sidebar_width, 304.)
        });
        visual.simulate_keystrokes("ctrl-0");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.settings.interface_scale, 100)
        });
        assert_eq!(
            visual.debug_bounds("session-sidebar").unwrap().size.width,
            before
        );
    }

    #[gpui::test]
    fn available_update_shows_a_pill_and_installs_from_about(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let asset = serde_json::json!({"name":"Workspacer-Native-Rust-Preview-Setup-0.170.0-nightly.1-x64.exe",
            "version":"0.170.0-nightly.1","size":10,"url":"https://github.com/DJTouchette/workspacer/releases/download/nightly/x.exe"});
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.requests.insert(
                    "updates",
                    wks_native::features::RequestState {
                        number: 1,
                        request: wks_native::features::Request::Updates,
                        loading: false,
                        value: Arc::new(serde_json::json!({
                            "channel":"nightly","installed":"0.169.0-nightly.1",
                            "latest":"0.170.0-nightly.1","update_available":true,
                            "installable":true,"asset":asset.clone()
                        })),
                        error: None,
                    },
                );
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
        let pill = visual.debug_bounds("update-pill").unwrap();
        visual.simulate_click(pill.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Settings);
            assert_eq!(this.settings_section, settings::SettingsSection::About);
        });
        let install = visual.debug_bounds("install-update").unwrap();
        visual.simulate_click(install.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let requested = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
            Command::Request(wks_native::features::Request::DownloadUpdate { asset }) => {
                Some(asset)
            }
            _ => None,
        });
        assert_eq!(requested, Some(asset));
    }

    #[gpui::test]
    fn remote_settings_pair_and_revoke_through_owner_requests(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        // Tall enough that the pairing list is on screen without scrolling.
        visual.simulate_resize(size(px(1000.), px(1400.)));
        let remote = |value: serde_json::Value| wks_native::features::RequestState {
            number: 1,
            request: wks_native::features::Request::Remote,
            loading: false,
            value: Arc::new(value),
            error: None,
        };
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.show_screen(Screen::Settings, window, cx);
                this.enter_settings_section(settings::SettingsSection::Remote, cx);
                let mut next = state("a");
                next.requests.insert(
                    "remote",
                    remote(serde_json::json!({
                        "tailscale":{"available":true,"magicName":"node.tailnet.ts.net",
                            "serveActive":true,"canServe":true},
                        "pairing":{"scope":"operator","canManageTokens":true},
                        "tokens":[{"token":"t-view","scope":"view",
                            "label":"Remote Control: view","created":"2026-10-02T10:00:00Z"}]
                    })),
                );
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
        let sent = |commands: &mut tokio::sync::mpsc::Receiver<Command>| {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|c| match c {
                    Command::Request(request) => Some(request),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(
            sent(&mut commands)
                .iter()
                .any(|r| matches!(r, wks_native::features::Request::Remote)),
            "opening Remote reads live Tailscale state"
        );
        // Triage is the default and has no pairing yet.
        assert!(visual.debug_bounds("pairing-qr").is_none());
        let create = visual.debug_bounds("pairing-create").unwrap();
        visual.simulate_click(create.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(sent(&mut commands).iter().any(|r| matches!(r,
            wks_native::features::Request::RemoteAction(wks_native::remote::Action::Pair(scope)) if scope == "triage")));

        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.remote.scope = "view";
                cx.notify();
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("pairing-qr").is_some());
        let revoke = visual.debug_bounds("pairing-revoke-0").unwrap();
        visual.simulate_click(revoke.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.remote.confirm_revoke.as_deref(), Some("t-view"))
        });
        assert!(
            sent(&mut commands).is_empty(),
            "the first click only asks for confirmation"
        );
        let revoke = visual.debug_bounds("pairing-revoke-0").unwrap();
        visual.simulate_click(revoke.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(sent(&mut commands).iter().any(|r| matches!(r,
            wks_native::features::Request::RemoteAction(wks_native::remote::Action::Revoke(token)) if token == "t-view")));
    }

    #[gpui::test]
    fn merged_turn_card_carries_interior_notes_only(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        let mut view = state("a");
        let call = |i: usize| Item {
            kind: "tool_use".into(),
            id: format!("call-{i}"),
            name: "Read".into(),
            input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
            ..Default::default()
        };
        let note = |text: &str| Item {
            kind: "assistant_text".into(),
            text: text.into(),
            ..Default::default()
        };
        view.transcript.snapshot(ConversationSnapshot {
            seq: 4,
            first_seq: 1,
            items: vec![
                call(0),
                note("Now the second file."),
                call(2),
                note("All done."),
            ],
        });
        let view = Arc::new(view);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(view.clone(), window, cx))
        });
        visual.run_until_parked();
        // Off (the default): prose splits the calls into separate cards.
        assert!(visual.debug_bounds("work-note-1").is_none());
        assert!(visual.debug_bounds("work-card-toggle").is_none());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.settings.merge_turn_tools = true;
                let count = this.view.transcript.rows.len();
                this.list.splice(0..count, count);
                this.update_view(view.clone(), window, cx);
                cx.notify();
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("work-card-toggle").is_some());
        assert!(visual.debug_bounds("work-note-1").is_some());
        // The closing answer stays an ordinary message below the card.
        assert!(visual.debug_bounds("work-note-3").is_none());
    }

    #[gpui::test]
    fn orchestration_calls_use_the_work_card_shell(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "tool_use".into(),
                id: "skill-0".into(),
                name: "Skill".into(),
                input: serde_json::json!({"skill":"review"}),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let card = visual.debug_bounds("orchestration-card").unwrap();
        let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
        assert!(visual.debug_bounds("tool-activity-group").is_none());
        // The header row spans the card, inset only by its border.
        assert!(toggle.size.width + px(4.) >= card.size.width);
        visual.simulate_click(toggle.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.tool_expansion.get("call:skill-0"), Some(&true));
        });
    }

    #[gpui::test]
    fn message_timestamps_are_below_user_assistant_and_tool_content(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        for role in ["user_message", "assistant_text", "tool_use"] {
            let mut view = state("a");
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: role.into(),
                    text: "A message to timestamp".into(),
                    id: "timestamped".into(),
                    name: "Read".into(),
                    input: serde_json::json!({"file_path":"src/main.rs"}),
                    ..Default::default()
                }],
            });
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    this.update_view(Arc::new(view), window, cx);
                })
            });
            visual.run_until_parked();
            let footer = visual.debug_bounds("message-timestamp-footer").unwrap();
            let row = visual.debug_bounds("last-transcript-row").unwrap();
            assert!(footer.top() > row.top() + row.size.height / 2.);
        }
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
    fn workspacer_spawn_receipt_opens_only_an_available_child(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: vec![
                Item {
                    kind: "tool_use".into(),
                    id: "spawn-child".into(),
                    name: "mcp__workspacer__spawn_agent".into(),
                    input: serde_json::json!({"message":"Review parsing"}),
                    ..Default::default()
                },
                Item {
                    kind: "tool_result".into(),
                    tool_use_id: "spawn-child".into(),
                    content: serde_json::json!({"sessionId":"b"}).to_string(),
                    ..Default::default()
                },
            ],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Parent draft", window, cx));
            })
        });
        visual.run_until_parked();
        let open = visual.debug_bounds("open-spawned-session").unwrap();
        visual.simulate_click(open.center(), gpui::Modifiers::default());
        assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
        assert!(commands.try_recv().is_err());
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "Parent draft")
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                Arc::make_mut(&mut view.sessions).retain(|s| s.id != "b");
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        let open = visual.debug_bounds("open-spawned-session").unwrap();
        visual.simulate_click(open.center(), gpui::Modifiers::default());
        assert!(commands.try_recv().is_err());
    }

    #[gpui::test]
    fn inline_children_update_and_open_parent_scoped_transcripts_without_losing_drafts(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let mut view = state("a");
        Arc::make_mut(&mut view.sessions)[0].merge(&serde_json::json!({"provider":"codex","subagents":[
            {"id":"native-one","toolUseId":"dispatch","type":"Explore","description":"Inspect parsing","status":"running","model":"runtime-model","tokens":0,"costUSD":0,"toolCalls":2,"startedAt":1000,"lastToolName":"Read"},
            {"id":"native-two","toolUseId":"dispatch","type":"Test","description":"Check aliases","status":"complete","startedAt":1000,"completedAt":4000}
        ]}));
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "tool_use".into(),
                id: "dispatch".into(),
                name: "Agent".into(),
                input: serde_json::json!({"prompt":"Inspect parsing"}),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Parent draft", window, cx));
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("child-agent-native-one").is_some());
        assert!(visual.debug_bounds("child-agent-native-two").is_some());
        assert!(visual.debug_bounds("native-spawn-icon").is_some());
        assert!(visual.debug_bounds("provider-child-icon").is_some());
        workspace.read_with(&visual, |this, _| {
            let child = &this.child_ui.agents.by_tool["dispatch"][0];
            assert_eq!(child.model, "runtime-model");
            assert_eq!(child.telemetry.tokens, Some(0));
            assert_eq!(child.telemetry.cost_usd, Some(0.));
            assert_eq!(
                this.child_ui.agents.by_tool["dispatch"][1].duration_ms(9000),
                Some(3000)
            );
        });
        let bounds = visual.debug_bounds("child-agent-native-one").unwrap();
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert!(
            matches!(commands.try_recv().unwrap(),Command::Request(wks_native::features::Request::SubagentHistory{session,agent}) if session=="a" && agent=="native-one")
        );
        assert!(commands.try_recv().is_err());
        visual.update(|window,cx|workspace.update(cx,|this,cx|{
            let mut view=(*this.view).clone();
            Arc::make_mut(&mut view.sessions)[0].merge(&serde_json::json!({"subagents":[
                {"id":"native-one","status":"complete","completedAt":5000,"tokens":2000},
                {"id":"native-two"}
            ]}));
            view.requests.insert("subagent-history",wks_native::features::RequestState{
                request:wks_native::features::Request::SubagentHistory{session:"a".into(),agent:"native-one".into()},number:1,loading:false,error:None,
                value:Arc::new(serde_json::json!({"rows":[{"key":1,"role":"Assistant","text":"The parser handles these aliases.\n```wks-result\n{\"ok\":true,\"caveats\":[]}\n```"}]}))
            });
            this.update_view(Arc::new(view),window,cx);
        }));
        visual.run_until_parked();
        assert!(visual.debug_bounds("child-transcript-panel").is_some());
        assert!(visual.debug_bounds("structured-result-card").is_some());
        workspace.read_with(&visual, |this, cx| {
            let child = &this.child_ui.agents.by_tool["dispatch"][0];
            assert!(!child.running());
            assert_eq!(child.telemetry.tokens, Some(2000));
            assert_eq!(child.duration_ms(9000), Some(4000));
            assert_eq!(this.composer.read(cx).value().as_ref(), "Parent draft");
        });
        let close = visual.debug_bounds("close-child-transcript").unwrap();
        visual.simulate_click(close.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.child_ui.closed,
                Some(1),
                "close={close:?}, header={:?}, dock={:?}",
                this.header_bounds,
                this.composer_dock_bounds
            )
        });
        // GPUI Frame::clear retains historical debug_bounds. A newly painted
        // closed marker proves the transition; absence of the old selector does not.
        assert!(visual.debug_bounds("closed-child-transcript").is_some());
        assert!(commands.try_recv().is_err());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut other = state("b");
                Arc::make_mut(&mut other.sessions)[1].merge(
                    &serde_json::json!({"subagents":[{"id":"native-one","status":"running"}]}),
                );
                other.requests = this.view.requests.clone();
                this.update_view(Arc::new(other), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(
            visual.debug_bounds("child-agent-native-one").is_some(),
            "unanchored children remain visible even before transcript arrives"
        );
        assert!(visual.debug_bounds("child-only-list").is_some());
        workspace.read_with(&visual, |this,_| {
            assert_eq!(this.view.selected.as_deref(),Some("b"));
            assert!(matches!(&this.view.requests["subagent-history"].request,wks_native::features::Request::SubagentHistory{session,..} if session!="b"));
            assert!(this.child_ui.agents.by_tool.is_empty());
        });
    }

    #[gpui::test]
    fn wheel_notches_glide_instead_of_jumping(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let items: Vec<Item> = (0..40)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nA paragraph to scroll past smoothly."),
                ..Default::default()
            })
            .collect();
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 40,
            first_seq: 1,
            items,
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let top = |visual: &mut gpui::VisualTestContext| {
            visual.debug_bounds("last-transcript-row").unwrap().top()
        };
        let start = top(&mut visual);
        let wheel = |visual: &mut gpui::VisualTestContext, lines: f32| {
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(px(600.), px(300.)),
                delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                ..Default::default()
            });
        };
        // One notch toward older messages: no instant jump…
        wheel(&mut visual, 3.);
        visual.run_until_parked();
        assert_eq!(
            top(&mut visual),
            start,
            "the list does not jump on the event"
        );
        // …a partial glide after a frame or two…
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(20));
        visual.run_until_parked();
        let partway = top(&mut visual) - start;
        assert!(
            partway > px(0.) && partway < px(96.),
            "partway: {partway:?}"
        );
        // …and it settles on exactly three lines.
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(600));
        visual.run_until_parked();
        assert!((f32::from(top(&mut visual) - start) - 96.).abs() < 0.01);
        workspace.read_with(&visual, |this, _| assert!(!this.follow, "reading history"));
        // Scrolling back down past the end re-pins the newest message.
        wheel(&mut visual, -30.);
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(800));
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.follow));
        // Touchpad pixel deltas stay immediate.
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(600.), px(300.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(50.))),
            ..Default::default()
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(!this.follow));
    }

    #[gpui::test]
    fn wheel_over_a_scrollable_panel_stays_with_the_panel(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut items: Vec<Item> = (0..30)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}"),
                ..Default::default()
            })
            .collect();
        items.push(Item {
            kind: "tool_use".into(),
            id: "fail-1".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command":"cargo test"}),
            ..Default::default()
        });
        items.push(Item {
            kind: "tool_result".into(),
            tool_use_id: "fail-1".into(),
            is_error: true,
            content: (0..200).map(|n| format!("line {n}\n")).collect(),
            ..Default::default()
        });
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: items.len() as u64,
            first_seq: 1,
            items,
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        // Failed calls open their details; the panel scrolls itself.
        let panel = visual.debug_bounds("tool-details").unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: panel.center(),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., 3.)),
            ..Default::default()
        });
        workspace.read_with(&visual, |this, _| assert!(!this.smooth_scroll.gliding()));
        // Outside it, the same notch glides the conversation.
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(
                workspace.read_with(&visual, |this, _| this.list.viewport_bounds().left()) + px(8.),
                panel.center().y,
            ),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., 3.)),
            ..Default::default()
        });
        workspace.read_with(&visual, |this, _| assert!(this.smooth_scroll.gliding()));
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(800));
        visual.run_until_parked();
    }

    #[gpui::test]
    fn markdown_table_cells_wrap_inside_their_column(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let long = "apps/native/src/ui/transcript.rs keeps growing with words that must wrap \
                    inside this column rather than run across the next one";
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: format!("| File | Notes | Status |\n|---|---|---|\n| {long} | short | ok |"),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        let long_cell = visual.debug_bounds("prose-table-cell-1-0").unwrap();
        let next = visual.debug_bounds("prose-table-cell-1-1").unwrap();
        let header = visual.debug_bounds("prose-table-cell-0-0").unwrap();
        assert!(
            long_cell.right() <= next.left() + px(0.5),
            "{long_cell:?} vs {next:?}"
        );
        assert!(
            long_cell.size.height > header.size.height * 2.,
            "the long cell wraps onto several lines: {long_cell:?}"
        );
    }

    /// Show `markdown` as the selected session's one assistant message.
    fn show_assistant_markdown(
        workspace: &Entity<Workspace>,
        visual: &mut VisualTestContext,
        markdown: &str,
    ) {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                // Later calls replace the message: a newer snapshot of the
                // same one-item conversation.
                let mut view = if this.view.selected.is_some() {
                    (*this.view).clone()
                } else {
                    state("a")
                };
                let seq = view.transcript.seq.unwrap_or(0) + 1;
                view.transcript.snapshot(ConversationSnapshot {
                    seq,
                    first_seq: 1,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: markdown.into(),
                        ..Default::default()
                    }],
                });
                this.update_view(Arc::new(view), window, cx)
            })
        });
        visual.run_until_parked();
        // Text views re-parse changed Markdown after a 200ms real-time
        // debounce (a smol timer, not the test clock).
        for _ in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    }

    /// Bounds of `selector`, or a panic naming it.
    fn bounds_of(visual: &mut VisualTestContext, selector: &str) -> gpui::Bounds<gpui::Pixels> {
        // `debug_bounds` wants a `'static` selector; tests leak a few.
        let key: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        visual
            .debug_bounds(key)
            .unwrap_or_else(|| panic!("{selector} is not rendered"))
    }

    /// A table's frame selector: tables are keyed by their first cell's
    /// source offset (`text` must start that cell).
    fn table_frame(markdown: &str, first_cell: &str) -> (String, String) {
        let key = markdown.find(first_cell).expect("first cell in the source");
        (
            format!("prose-table-frame-{key}"),
            format!("prose-table-content-{key}"),
        )
    }

    /// Every column of the (only) table in view can be brought fully inside
    /// its frame: wide tables scroll sideways rather than clip. Scrolls
    /// across in half-viewport steps and requires each header cell to be
    /// seen whole inside the viewport; adjacent painted cells never overlap.
    /// Returns whether the table had to scroll, leaving it back at the start.
    fn assert_every_column_reachable(
        visual: &mut VisualTestContext,
        (frame, content): &(String, String),
        cols: usize,
        window_width: gpui::Pixels,
    ) -> bool {
        let tolerance = gpui::px(0.5);
        let view = bounds_of(visual, frame);
        assert!(
            view.right() <= window_width + tolerance,
            "the table's viewport {view:?} stays inside the {window_width:?} window"
        );
        let content_start = bounds_of(visual, content);
        assert!(
            (content_start.left() - view.left()).abs() <= gpui::px(1.5),
            "the table starts unscrolled: {content_start:?} vs {view:?}"
        );
        let scrolls = content_start.right() > view.right() + gpui::px(1.5);
        assert_eq!(
            visual.debug_bounds("prose-table-scrollbar").is_some(),
            scrolls,
            "a scrollbar shows exactly when columns lie beyond the viewport \
             ({content_start:?} in {view:?})"
        );
        let mut seen = vec![false; cols];
        let mut header_heights = vec![None; cols];
        for _ in 0..60 {
            let mut painted = Vec::new();
            for (col, seen) in seen.iter_mut().enumerate() {
                let key: &'static str =
                    Box::leak(format!("prose-table-cell-0-{col}").into_boxed_str());
                if let Some(cell) = visual.debug_bounds(key) {
                    *seen |= cell.left() >= view.left() - tolerance
                        && cell.right() <= view.right() + tolerance;
                    painted.push((col, cell));
                    header_heights[col] = Some(cell.size.height);
                }
            }
            for pair in painted.windows(2) {
                let ((a_col, a), (b_col, b)) = (pair[0], pair[1]);
                if b_col == a_col + 1 {
                    assert!(a.right() <= b.left() + tolerance, "{a:?} overlaps {b:?}");
                }
            }
            if bounds_of(visual, content).right() <= view.right() + gpui::px(1.5) {
                break;
            }
            // Sideways (trackpad or Shift+wheel) scrolling, over the header.
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(view.center().x, view.top() + gpui::px(12.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(-view.size.width / 2., gpui::px(0.))),
                ..Default::default()
            });
            visual.run_until_parked();
        }
        assert!(
            seen.iter().all(|s| *s),
            "every column is reachable whole inside {view:?}: {seen:?}"
        );
        // These tests' headers are short words: one line each, never broken.
        let heights: Vec<_> = header_heights.into_iter().flatten().collect();
        let low = heights.iter().copied().fold(heights[0], |a, b| a.min(b));
        assert!(
            heights.iter().all(|h| *h <= low + tolerance),
            "a header wrapped onto another line: {heights:?}"
        );
        let end = bounds_of(visual, content);
        assert!(
            (end.right() - view.right()).abs() <= gpui::px(1.5),
            "scrolling stops at the table's end: {end:?} vs {view:?}"
        );
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(view.center().x, view.top() + gpui::px(12.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(100_000.), gpui::px(0.))),
            ..Default::default()
        });
        visual.run_until_parked();
        assert_eq!(bounds_of(visual, content).left(), content_start.left());
        scrolls
    }

    const FOUR_COLUMNS: &str = "| Component | Implementation | Validation | Observation |\n\
        |---|---|---|---|\n| renderer | incremental | regression | consistent |";

    #[gpui::test]
    fn narrow_chat_table_scrolls_to_every_column(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(720.), px(480.)));
        show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
        // Geometry alone: sideways scrolling over the table brings its last
        // column whole into the chat column (it used to be clipped at
        // x≈666–778, past the 720px window, with no way to reach it).
        let chat = bounds_of(&mut visual, "chat-composer");
        let first = bounds_of(&mut visual, "prose-table-cell-0-0");
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-5000.), px(0.))),
            ..Default::default()
        });
        visual.run_until_parked();
        let last = bounds_of(&mut visual, "prose-table-cell-0-3");
        assert!(
            last.left() >= chat.left() && last.right() <= chat.right().min(px(720.)),
            "the last column {last:?} is reachable inside the chat column {chat:?}"
        );
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(5000.), px(0.))),
            ..Default::default()
        });
        visual.run_until_parked();
        let table = table_frame(FOUR_COLUMNS, "Component");
        let (frame, content) = table.clone();
        let view = bounds_of(&mut visual, &frame);
        assert!(
            view.left() >= chat.left() - px(1.) && view.right() <= chat.right() + px(1.),
            "the table stays in the chat column: {view:?} vs {chat:?}"
        );
        assert!(
            assert_every_column_reachable(&mut visual, &table, 4, px(720.)),
            "four whole-word columns do not fit beside the sidebar at 720px"
        );
        // A plain vertical wheel over the table scrolls the chat, not the table.
        let start = bounds_of(&mut visual, &content).left();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: view.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-40.))),
            ..Default::default()
        });
        visual.run_until_parked();
        assert_eq!(bounds_of(&mut visual, &content).left(), start);
        // The keyboard: click (or Tab to) the table, then Left/Right.
        let view = bounds_of(&mut visual, &frame);
        visual.simulate_click(view.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("right");
        visual.run_until_parked();
        let moved = bounds_of(&mut visual, &content).left();
        assert!(
            moved < start,
            "Right scrolls the table: {moved:?} vs {start:?}"
        );
        for _ in 0..20 {
            visual.simulate_keystrokes("right");
        }
        visual.run_until_parked();
        let view = bounds_of(&mut visual, &frame);
        let last = bounds_of(&mut visual, "prose-table-cell-0-3");
        assert!(
            last.right() <= view.right() + px(0.5),
            "{last:?} vs {view:?}"
        );
        let content_bounds = bounds_of(&mut visual, &content);
        assert!(
            (content_bounds.right() - view.right()).abs() <= px(1.5),
            "Right stops at the end: {content_bounds:?} vs {view:?}"
        );
        for _ in 0..20 {
            visual.simulate_keystrokes("left");
        }
        visual.run_until_parked();
        assert_eq!(bounds_of(&mut visual, &content).left(), start);
        // Tab leaves the table and Shift+Tab comes back to it.
        visual.simulate_keystrokes("tab right");
        visual.run_until_parked();
        assert_eq!(
            bounds_of(&mut visual, &content).left(),
            start,
            "after Tab, Right belongs to another control"
        );
        visual.simulate_keystrokes("shift-tab right");
        visual.run_until_parked();
        assert!(
            bounds_of(&mut visual, &content).left() < start,
            "the table is a keyboard stop"
        );
        visual.simulate_keystrokes("left left left left left left");
        visual.run_until_parked();
        // The mouse: drag the scrollbar's thumb to the right.
        let bar = bounds_of(&mut visual, "prose-table-scrollbar");
        let grab = gpui::point(bar.left() + px(20.), bar.center().y);
        visual.simulate_mouse_down(grab, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(
            grab + gpui::point(px(300.), px(0.)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        visual.simulate_mouse_up(
            grab + gpui::point(px(300.), px(0.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        let view = bounds_of(&mut visual, &frame);
        let dragged = bounds_of(&mut visual, &content);
        assert!(
            (dragged.right() - view.right()).abs() <= px(1.5),
            "dragging the thumb to the end shows the last column: {dragged:?} vs {view:?}"
        );
    }

    #[gpui::test]
    fn tables_that_fit_fill_the_width_without_scrolling(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let md = "| Name | Status |\n|---|---|\n| alpha | ok |";
        show_assistant_markdown(&workspace, &mut visual, md);
        let table = table_frame(md, "Name");
        let frame = table.0.clone();
        assert!(!assert_every_column_reachable(
            &mut visual,
            &table,
            2,
            px(1000.)
        ));
        let view = bounds_of(&mut visual, &frame);
        let last = bounds_of(&mut visual, "prose-table-cell-0-1");
        assert!(
            (last.right() - view.right()).abs() <= px(1.5),
            "columns fill the table: {last:?} vs {view:?}"
        );
        // Not a scroll region, so not a keyboard stop either.
        visual.simulate_click(view.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("right");
        visual.run_until_parked();
        assert_eq!(bounds_of(&mut visual, &frame), view);
        // The four-column table fits the default window without scrolling.
        show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
        let table = table_frame(FOUR_COLUMNS, "Component");
        assert!(!assert_every_column_reachable(
            &mut visual,
            &table,
            4,
            px(1000.)
        ));
    }

    #[gpui::test]
    fn wide_tables_of_every_shape_keep_all_columns_reachable(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(720.), px(480.)));
        let url = "https://example.com/a/very/long/unbroken/path/to/a/resource?with=query";
        let token = "Supercalifragilisticexpialidociousness";
        for cols in [2usize, 3, 5, 6, 8] {
            let header = (0..cols)
                .map(|c| ["Status", "Owner", "Notes", "Severity", "Area"][c % 5].to_owned())
                .collect::<Vec<_>>();
            let aligns = (0..cols)
                .map(|c| [":---", ":---:", "---:"][c % 3])
                .collect::<Vec<_>>();
            let body = (0..cols)
                .map(|c| match c % 4 {
                    0 => url.to_owned(),
                    1 => "`render_prose_table` keeps `min_w`".to_owned(),
                    2 => token.to_owned(),
                    _ => "short words that wrap inside the column".to_owned(),
                })
                .collect::<Vec<_>>();
            let md = format!(
                "Before the table.\n\n| {} |\n|{}|\n| {} |\n| {} |\n\nAfter the table.",
                header.join(" | "),
                aligns.join("|"),
                body.join(" | "),
                body.iter().rev().cloned().collect::<Vec<_>>().join(" | "),
            );
            show_assistant_markdown(&workspace, &mut visual, &md);
            // Tall tables open scrolled to their end; bring the header in.
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(px(500.), px(250.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(2000.))),
                ..Default::default()
            });
            visual.run_until_parked();
            visual
                .executor()
                .advance_clock(std::time::Duration::from_millis(800));
            visual.run_until_parked();
            let table = table_frame(&md, header[0].as_str());
            let frame = table.0.clone();
            assert_every_column_reachable(&mut visual, &table, cols, px(720.));
            // Rows stack inside the frame, and the table sits in the message
            // flow: it neither overlaps nor collapses into its neighbours.
            let view = bounds_of(&mut visual, &frame);
            let header_row = bounds_of(&mut visual, "prose-table-cell-0-0");
            let last_row = bounds_of(&mut visual, "prose-table-cell-2-0");
            assert!(header_row.top() >= view.top() && last_row.bottom() <= view.bottom() + px(0.5));
            assert!(last_row.top() >= header_row.bottom() - px(0.5));
            for col in 0..cols {
                let a = bounds_of(&mut visual, &format!("prose-table-cell-1-{col}"));
                assert_eq!(
                    a.top(),
                    bounds_of(&mut visual, "prose-table-cell-1-0").top()
                );
                assert!(
                    a.size.height > px(0.),
                    "{cols} columns: cell {col} is empty"
                );
            }
        }
    }

    #[gpui::test]
    fn two_wide_tables_scroll_independently(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.simulate_resize(size(px(720.), px(480.)));
        let second = FOUR_COLUMNS.replace("Component", "Subsystem");
        let md = format!("{FOUR_COLUMNS}\n\nBetween.\n\n{second}");
        show_assistant_markdown(&workspace, &mut visual, &md);
        let (frame_a, content_a) = table_frame(&md, "Component");
        let (frame_b, content_b) = table_frame(&md, "Subsystem");
        let (a, b) = (
            bounds_of(&mut visual, &frame_a),
            bounds_of(&mut visual, &frame_b),
        );
        assert!(a.bottom() <= b.top(), "{a:?} above {b:?}");
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: b.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-5000.), px(0.))),
            ..Default::default()
        });
        visual.run_until_parked();
        assert!(bounds_of(&mut visual, &content_b).left() < b.left() - px(1.));
        assert_eq!(bounds_of(&mut visual, &content_a).left(), a.left() + px(1.));
    }

    #[gpui::test]
    fn larger_interface_sizes_keep_table_columns_reachable(cx: &mut TestAppContext) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                set_zoom(1.);
            }
        }
        let _reset = Reset;
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
        let table = table_frame(FOUR_COLUMNS, "Component");
        for _ in 0..6 {
            visual.simulate_keystrokes("ctrl-=");
        }
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(
                this.settings.interface_scale >= 150,
                "{}",
                this.settings.interface_scale
            )
        });
        assert!(assert_every_column_reachable(
            &mut visual,
            &table,
            4,
            px(1000.)
        ));
    }

    #[gpui::test]
    fn markdown_preview_tables_reach_every_column_docked_sheet_and_window(cx: &mut TestAppContext) {
        let doc = format!("# Plan\n\n{FOUR_COLUMNS}\n\nAfter.\n");
        let table = table_frame(&doc, "Component");
        let frame = table.0.clone();
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        // A narrow window: the viewer is a sheet over the chat.
        visual.simulate_resize(size(gpui::px(720.), gpui::px(480.)));
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/plan.md"),
            1,
            false,
            None,
            serde_json::json!({"contents": doc, "size": doc.len()}),
        );
        settle(&mut visual);
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(
                this.file_viewer().unwrap().read(cx).mode(),
                file_viewer::Mode::Preview
            )
        });
        assert_every_column_reachable(&mut visual, &table, 4, gpui::px(720.));
        // Docked beside the chat in a wide window.
        visual.simulate_resize(size(gpui::px(1400.), gpui::px(800.)));
        settle(&mut visual);
        let panel = bounds_of(&mut visual, "file-viewer-panel");
        let view = bounds_of(&mut visual, &frame);
        assert!(view.left() >= panel.left() && view.right() <= panel.right() + gpui::px(0.5));
        assert_every_column_reachable(&mut visual, &table, 4, gpui::px(1400.));
        // Popped out into a narrow window of its own.
        let popout = bounds_of(&mut visual, "file-viewer-popout");
        visual.simulate_click(popout.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        let (handle, _pane) =
            workspace.read_with(&visual, |this, _| this.viewer_popout().expect("popped out"));
        let mut window = VisualTestContext::from_window(handle.into(), cx);
        window.simulate_resize(size(gpui::px(420.), gpui::px(480.)));
        window.run_until_parked();
        assert!(
            assert_every_column_reachable(&mut window, &table, 4, gpui::px(420.)),
            "a narrow separate window scrolls the table"
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
    fn switching_back_lands_on_the_latest_message(cx: &mut TestAppContext) {
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
                assert!(this.follow);
                // Scroll up, leave, and come back after new activity.
                this.follow = false;
                this.list.scroll_to(ListOffset {
                    item_ix: 12,
                    offset_in_item: px(7.),
                });
                this.update_view(Arc::new(state("b")), window, cx);
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
                assert!(this.follow);
            });
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(this.follow);
            assert_ne!(this.list.logical_scroll_top().item_ix, 12);
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
    fn default_access_applies_on_new_launch_and_provider_switch(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.settings.default_claude_access = Permission::FullAccess;
                this.settings.default_codex_access = Permission::Ask;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                assert_eq!(this.permission, Permission::FullAccess);
                this.choose_provider("codex", window, cx);
                assert_eq!(this.permission, Permission::Ask);
                this.permission = Permission::FullAccess;
                this.new_session = false;
                this.settings.default_provider = Provider::Codex;
                this.show_new_session(window, cx);
                assert_eq!(
                    this.permission,
                    Permission::Ask,
                    "a new launch reapplies the saved default even for the same provider"
                );
                this.choose_provider("claude", window, cx);
                assert_eq!(this.permission, Permission::FullAccess);
                this.select_project("/work/project", window, cx);
                this.create(window, cx);
            });
        });
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("expected create")
        };
        assert_eq!(request.permission, Permission::FullAccess);
        assert_eq!(
            request.params().unwrap()["permissionMode"],
            "bypassPermissions"
        );
    }

    #[gpui::test]
    fn model_menu_shows_catalog_when_custom_is_selected(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut loaded = state("a");
                loaded.catalog = wks_native::launch::Catalog {
                    key: CatalogKey {
                        provider: "claude".into(),
                        cwd: String::new(),
                    },
                    models: ["opus", "sonnet", "haiku"]
                        .into_iter()
                        .map(|id| ModelChoice {
                            id: id.into(),
                            label: id.into(),
                            windows: vec![],
                            is_default: false,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                };
                this.update_view(Arc::new(loaded), window, cx);
                this.show_new_session(window, cx);
                this.model_choice = "__custom".into();
                this.model_picker.update(cx, |picker, cx| {
                    picker.set_selected_value(&String::from("__custom"), window, cx);
                    picker.focus(window, cx);
                });
            });
        });
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        for selector in [
            "model-option-opus",
            "model-option-sonnet",
            "model-option-haiku",
        ] {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "selected Custom must not scroll {selector} out of a menu that can fit all entries"
            );
        }
        visual.simulate_input("custom");
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        workspace.read_with(&visual, |this, _| assert_eq!(this.model_choice, "__custom"));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.show_screen(Screen::Conversation, window, cx);
                this.show_new_session(window, cx);
                this.model_picker
                    .update(cx, |picker, cx| picker.focus(window, cx));
            });
        });
        visual.simulate_keystrokes("enter down enter");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.model_choice, "opus",
                "a new picker must clear the previous Custom-only search"
            )
        });
    }

    #[gpui::test]
    fn fresh_agent_does_not_inherit_custom_model_from_session_controls(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                this.open_feature(Screen::Model, window, cx);
                assert_eq!(this.model_choice, "__custom");
                this.show_new_session(window, cx);
                assert!(this.model_choice.is_empty(), "new agent should start with Provider default, rather than the inspected session's Custom entry");
                assert_eq!(this.model_picker.read(cx).selected_value().map(String::as_str), Some(""));
            });
        });
    }

    #[gpui::test]
    fn opening_new_agent_uses_an_already_loaded_catalog(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut loaded = state("a");
                loaded.catalog = wks_native::launch::Catalog {
                    key: CatalogKey {
                        provider: "claude".into(),
                        cwd: String::new(),
                    },
                    models: vec![ModelChoice {
                        id: "opus".into(),
                        label: "Opus".into(),
                        windows: vec![200000],
                        is_default: false,
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                this.update_view(Arc::new(loaded), window, cx);
                this.show_new_session(window, cx);
                assert_eq!(
                    this.catalog_models.len(),
                    1,
                    "a cached catalog must populate without waiting for another update"
                );
            });
        });
    }

    #[gpui::test]
    fn open_model_dropdown_renders_arriving_catalog(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        visual.simulate_resize(size(px(1000.), px(700.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                this.model_picker
                    .update(cx, |picker, cx| picker.focus(window, cx));
            });
        });
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert!(visual.debug_bounds("model-option-").is_some());
        assert!(visual.debug_bounds("model-option-opus").is_none());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut loaded = state("a");
                loaded.catalog = wks_native::launch::Catalog {
                    key: CatalogKey {
                        provider: "claude".into(),
                        cwd: String::new(),
                    },
                    models: vec![ModelChoice {
                        id: "opus".into(),
                        label: "Opus".into(),
                        windows: vec![200000],
                        is_default: false,
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                this.update_view(Arc::new(loaded), window, cx);
            });
        });
        visual.run_until_parked();
        let option = visual
            .debug_bounds("model-option-opus")
            .expect("catalog arrival should redraw the open dropdown");
        assert!(option.size.width > px(0.) && option.size.height > px(0.));
        assert!(
            option.top() >= px(0.) && option.bottom() <= px(700.),
            "model option outside viewport: {option:?}"
        );
        visual.simulate_click(option.center(), gpui::Modifiers::default());
        workspace.read_with(&visual, |this, _| assert_eq!(this.model_choice, "opus"));
    }

    #[gpui::test]
    fn model_picker_keyboard_selection_and_provider_reset(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                this.select_project("/work/project", window, cx);
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
                        is_default: false,
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                this.update_view(Arc::new(loaded), window, cx);
                this.launch_details_open = true;
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
                this.create(window, cx);
                this.spawn_pending = false;
                this.choose_provider("codex", window, cx);
                assert!(this.model_choice.is_empty());
                assert_eq!(this.context_window, None);
                assert_eq!(this.permission, Permission::Ask);
                assert!(this.catalog_models.is_empty());
                this.model_choice = "__custom".into();
                this.create(window, cx);
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
    fn effort_sits_beside_the_model_and_follows_what_the_model_accepts(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let catalog = |view: &mut View| {
            view.catalog = wks_native::launch::Catalog {
                key: CatalogKey {
                    provider: "codex".into(),
                    cwd: "/work/project".into(),
                },
                models: wks_native::launch::parse_models(
                    "codex",
                    serde_json::json!([
                        {"id":"sol","label":"Sol","default":true,
                         "effortLevels":["low","medium","high","xhigh"],"defaultEffort":"medium"},
                        {"id":"mini","label":"Mini","effortLevels":["minimal","low"],"defaultEffort":"low"}
                    ]),
                )
                .unwrap(),
                ..Default::default()
            };
        };
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.settings.default_provider = Provider::Codex;
                this.update_view(Arc::new(state("a")), window, cx);
                this.show_new_session(window, cx);
                this.select_project("/work/project", window, cx);
                let mut loaded = state("a");
                catalog(&mut loaded);
                this.update_view(Arc::new(loaded), window, cx);
            })
        });
        visual.run_until_parked();
        // Visible without opening Options, on the model's row.
        let model = visual.debug_bounds("launch-model-picker").unwrap();
        let effort = visual.debug_bounds("launch-effort-picker").unwrap();
        assert!(visual.debug_bounds("launch-details").is_none());
        assert_eq!(model.top(), effort.top());
        assert!(effort.left() > model.right());
        let focus = |picker: fn(&Workspace) -> Entity<SelectState<SearchableVec<PickerItem>>>| {
            let workspace = workspace.clone();
            move |window: &mut Window, cx: &mut App| {
                let picker = picker(workspace.read(cx));
                picker.update(cx, |p, cx| p.focus(window, cx));
            }
        };
        // The menu lists exactly what Sol accepts.
        visual.update(focus(|this| this.effort_picker.clone()));
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert!(visual.debug_bounds("effort-option-xhigh").is_some());
        assert!(
            visual.debug_bounds("effort-option-max").is_none(),
            "Codex Sol has no max"
        );
        visual.simulate_keystrokes("down down down down enter");
        workspace.read_with(&visual, |this, _| assert_eq!(this.effort, "xhigh"));
        // A model that does not accept the level returns effort to Default.
        visual.update(focus(|this| this.model_picker.clone()));
        visual.simulate_keystrokes("enter down down enter");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.model_choice, "mini");
            assert!(
                this.effort.is_empty(),
                "stale xhigh must not be sent to Mini"
            );
            assert_eq!(this.effort_options().0, ["minimal", "low"]);
        });
        visual.update(focus(|this| this.effort_picker.clone()));
        visual.simulate_keystrokes("enter down enter");
        workspace.read_with(&visual, |this, _| assert_eq!(this.effort, "minimal"));
        visual.simulate_keystrokes("ctrl-enter");
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("expected launch")
        };
        let params = request.params().unwrap();
        assert_eq!(
            (params["model"].as_str(), params["effort"].as_str()),
            (Some("mini"), Some("minimal"))
        );
        // Claude has its own ladder; nothing chosen for Codex carries over.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.spawn_pending = false;
                this.choose_provider("claude", window, cx);
                assert!(this.effort.is_empty());
                assert_eq!(
                    this.effort_options().0,
                    wks_native::launch::CLAUDE_EFFORTS.map(String::from)
                );
                this.create(window, cx);
            })
        });
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("expected launch")
        };
        assert!(
            request.params().unwrap().get("effort").is_none(),
            "Default sends no effort"
        );
        // Still on screen at the minimum window size.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.spawn_pending = false;
                this.show_new_session(window, cx);
            })
        });
        visual.simulate_resize(size(px(720.), px(480.)));
        visual.run_until_parked();
        let effort = visual.debug_bounds("launch-effort-picker").unwrap();
        assert!(effort.right() <= px(720.));
    }

    /// A hub registry reply, as the controller would publish it.
    fn with_registry(view: &mut View, number: u64, mut registry: serde_json::Value) {
        registry["revision"] = serde_json::json!(number);
        view.requests.insert(
            "projects",
            wks_native::features::RequestState {
                number,
                request: wks_native::features::Request::Projects,
                loading: false,
                value: Arc::new(registry),
                error: None,
            },
        );
    }

    #[gpui::test]
    fn project_snapshot_revisions_beat_request_order_in_every_window(cx: &mut TestAppContext) {
        use wks_native::{
            features::{Request, RequestState},
            projects::Patch,
        };
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let (other, mut second, _commands2, _updates2) = fixture(cx);
        let snapshot = |revision, pinned| {
            Arc::new(serde_json::json!({
                "revision":revision,"projects":{"/work/web":{"favourite":pinned,"lastOpened":10}},
                "favourites":[],"recent":[],"configured":[]
            }))
        };
        let read = |value| RequestState {
            number: 9,
            request: Request::Projects,
            loading: false,
            value,
            error: None,
        };
        let save = |number, change, value| RequestState {
            number,
            request: Request::SaveProject {
                path: "/work/web".into(),
                change,
            },
            loading: false,
            value,
            error: None,
        };
        let mut next = state("a");
        // Newer request 9 returned pre-save state; acknowledged save 6 is
        // authoritative. This is the review's Pinned + empty-star regression.
        next.requests.insert("projects", read(snapshot(1, false)));
        next.requests
            .insert("project-save", save(6, Patch::Pin(true), snapshot(2, true)));
        for (entity, visual) in [(&workspace, &mut visual), (&other, &mut second)] {
            visual.update(|window, cx| {
                entity.update(cx, |this, cx| {
                    this.demo = false;
                    this.show_new_session(window, cx);
                    this.select_project("/work/web", window, cx);
                    this.update_view(Arc::new(next.clone()), window, cx);
                    assert!(this.known_project("/work/web").unwrap().favourite);
                    assert_eq!(this.projects.notice, "Pinned.");
                })
            });
        }
        visual.run_until_parked();
        while commands.try_recv().is_ok() {}
        let pin = visual.debug_bounds("launch-project-pin").unwrap();
        visual.simulate_click(pin.center(), gpui::Modifiers::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::Request(Request::SaveProject {
                    change: Patch::Pin(false),
                    ..
                })
            )),
            "the selected summary offers Unpin after the acknowledged pin"
        );
        // A late old read cannot replace that acknowledged pin either.
        next.requests.remove("project-save");
        next.requests.insert(
            "projects",
            RequestState {
                number: 12,
                ..read(snapshot(1, false))
            },
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx);
                assert!(this.known_project("/work/web").unwrap().favourite);
            })
        });
        // A later touch includes the unpin and must outlive an older save.
        let touched = snapshot(4, false);
        next.project_registry = Some(touched);
        next.requests.insert(
            "project-save",
            save(10, Patch::Pin(false), snapshot(3, false)),
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx);
                assert!(!this.known_project("/work/web").unwrap().favourite);
                assert_eq!(this.projects.notice, "Unpinned.");
                assert_eq!(this.projects.registry_revision, 4);
            })
        });
        next.requests.insert(
            "project-save",
            save(
                11,
                Patch::Remove,
                Arc::new(serde_json::json!({"revision":5,"projects":{}})),
            ),
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx);
                assert!(this.known_project("/work/web").is_none());
                assert_eq!(this.projects.notice, "Removed from projects.");
            })
        });
        next.requests.insert(
            "project-save",
            RequestState {
                error: Some("refused".into()),
                ..save(13, Patch::Pin(true), Arc::new(serde_json::Value::Null))
            },
        );
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(next.clone()), window, cx);
                assert!(this.projects.notice.contains("refused"));
                assert_eq!(this.projects.fallback.as_deref(), Some("/work/web"));
                assert!(this.known_project("/work/web").is_none());
            })
        });
    }

    #[gpui::test]
    fn project_chooser_is_keyboard_first_and_never_launches_without_a_folder(
        cx: &mut TestAppContext,
    ) {
        use wks_native::features::Request;
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut view = state("a");
                with_registry(
                    &mut view,
                    1,
                    serde_json::json!({"projects":{
                        "/work/api":{"label":"API Server","lastOpened":20},
                        "/work/web":{"favourite":true},
                        "/work/old":{"lastOpened":10}
                    },"favourites":[],"recent":[],"configured":[]}),
                );
                this.update_view(Arc::new(view), window, cx);
            })
        });
        // No folder yet: the form opens on the chooser with the search focused.
        visual.simulate_keystrokes("ctrl-n");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(this.projects.picker_open && this.projects.cwd.is_empty());
            let rows: Vec<_> = this
                .pick_rows(cx)
                .iter()
                .map(|r| match r {
                    projects::PickRow::Project(p) => p.path.clone(),
                    other => panic!("unexpected {other:?}"),
                })
                .collect();
            assert_eq!(
                rows,
                ["/work/web", "/work/api", "/work/old"],
                "pinned, then recent"
            );
        });
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|c| matches!(c, Command::Request(Request::Projects))),
            "opening the form reads the hub's registry"
        );
        // Launching without a folder explains itself and does not create.
        visual.simulate_keystrokes("ctrl-enter");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.spawn_error, "Choose a project folder first.");
            assert!(this.projects.picker_open);
        });
        assert!(next_effect(&mut commands).is_none());
        // Search by label, move, choose with Enter.
        visual.simulate_input("server");
        visual.simulate_keystrokes("enter");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.projects.cwd, "/work/api");
            assert!(!this.projects.picker_open);
            assert!(this.spawn_error.is_empty());
        });
        let inspected = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
            Command::Request(Request::InspectProject { path }) => Some(path),
            _ => None,
        });
        assert_eq!(
            inspected.as_deref(),
            Some("/work/api"),
            "the hub checks the chosen folder"
        );
        // Change, arrow down, Esc keeps the original choice.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.open_project_picker(window, cx))
        });
        visual.simulate_keystrokes("down");
        visual.simulate_keystrokes("escape");
        workspace.read_with(&visual, |this, _| {
            assert!(!this.projects.picker_open);
            assert_eq!(this.projects.cwd, "/work/api");
        });
        // A pasted path that is not a project: offered first, kept visible
        // as the current folder when the list reopens, and launched into.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.open_project_picker(window, cx))
        });
        visual.simulate_input("/srv/fresh/");
        workspace.read_with(&visual, |this, cx| {
            assert!(
                matches!(&this.pick_rows(cx)[0], projects::PickRow::Typed(p) if p == "/srv/fresh")
            );
        });
        visual.simulate_keystrokes("ctrl-enter");
        let Some(Command::Create(request)) = next_effect(&mut commands) else {
            panic!("ctrl-enter launches into the pasted folder")
        };
        assert_eq!(request.cwd, "/srv/fresh");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.spawn_pending = false;
                this.open_project_picker(window, cx);
                assert!(matches!(&this.pick_rows(cx)[0], projects::PickRow::Current(p) if p == "/srv/fresh"));
            })
        });
    }

    #[gpui::test]
    fn fresh_forms_drop_resumed_context_and_stale_folder_checks(cx: &mut TestAppContext) {
        use wks_native::features::{Request, RequestState};
        use wks_native::projects::Inspection;
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut view = state("a");
                Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
                Arc::make_mut(&mut view.sessions)[1].cwd = "/work/beta".into();
                this.update_view(Arc::new(view), window, cx);
                let old = Session {
                    id: "past".into(),
                    label: "Old work".into(),
                    provider: "claude".into(),
                    cwd: "/work/beta".into(),
                    state: "stopped".into(),
                    ..Default::default()
                };
                this.resume_session(&old, window, cx);
                assert_eq!(this.projects.cwd, "/work/beta");
                assert_eq!(this.label.read(cx).value().as_ref(), "Old work");
                this.prompt
                    .update(cx, |i, cx| i.set_value("continue please", window, cx));
                this.effort = "high".into();
                this.show_screen(Screen::Conversation, window, cx);
                // A fresh New Agent is not the resumed conversation.
                this.show_new_session(window, cx);
                assert!(this.extras.resume.is_none());
                assert!(this.label.read(cx).value().is_empty());
                assert!(this.prompt.read(cx).value().is_empty());
                assert!(this.effort.is_empty());
                assert_eq!(
                    this.projects.cwd, "/work/alpha",
                    "starts where the user is looking"
                );
                // A plain draft survives leaving and reopening the form.
                this.prompt
                    .update(cx, |i, cx| i.set_value("my draft", window, cx));
                this.show_screen(Screen::Conversation, window, cx);
                this.show_new_session(window, cx);
                assert_eq!(this.prompt.read(cx).value().as_ref(), "my draft");
                // A check for a folder that is no longer chosen is not shown.
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "project-inspect",
                    RequestState {
                        number: 50,
                        request: Request::InspectProject {
                            path: "/work/beta".into(),
                        },
                        loading: false,
                        value: Arc::new(serde_json::json!({"exists":false,"error":"gone"})),
                        error: None,
                    },
                );
                this.update_view(Arc::new(view), window, cx);
                assert!(this.inspection().is_none());
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "project-inspect",
                    RequestState {
                        number: 51,
                        request: Request::InspectProject {
                            path: "/work/alpha".into(),
                        },
                        loading: false,
                        value: Arc::new(serde_json::json!({"exists":false,"error":"gone"})),
                        error: None,
                    },
                );
                this.update_view(Arc::new(view), window, cx);
                assert_eq!(
                    this.inspection(),
                    Some(Ok(Inspection::Missing("gone".into())))
                );
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("launch-project-status").is_some());
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
    fn structured_results_render_in_assistant_and_completion_messages(cx: &mut TestAppContext) {
        let (workspace, mut visual, _, _updates) = fixture(cx);
        for text in [
            "Done\n```wks-result\n{\"merged\":true,\"caveats\":[\"Not checked\"],\"customField\":{\"count\":2}}\n```",
            "[fleet] Worker finished:\n- Builder (session:b, cwd /repo) — last reply: Done\n\nStructured result — Builder (session:b):\n{\"ok\":true}",
            "```wks-result\ninvalid JSON\n```",
        ] {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    let mut view = state("a");
                    view.transcript.snapshot(ConversationSnapshot {
                        seq: 1,
                        first_seq: 1,
                        items: vec![Item {
                            kind: if text.starts_with("[fleet]") {
                                "user_message"
                            } else {
                                "assistant_text"
                            }
                            .into(),
                            text: text.into(),
                            ..Default::default()
                        }],
                    });
                    this.update_view(Arc::new(view), window, cx);
                })
            });
            visual.run_until_parked();
            assert!(visual.debug_bounds("structured-result-card").is_some());
        }
    }

    #[gpui::test]
    fn sidebar_archive_hides_session_without_selecting_or_stopping_it(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands, _updates) = fixture(cx);
        let path = std::env::temp_dir().join(format!("native-archive-{}.json", std::process::id()));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.settings_path = Some(path.clone());
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            });
        });
        visual.run_until_parked();
        let archive = visual.debug_bounds("sidebar-archive-1").unwrap();
        visual.simulate_click(archive.center(), gpui::Modifiers::default());
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.visible_sessions(cx), vec![0]);
            assert_eq!(this.view.selected.as_deref(), Some("a"));
            assert_eq!(this.composer.read(cx).value().as_ref(), "keep draft");
        });
        assert!(commands.try_recv().is_err());
        assert_eq!(Settings::load(&path).unwrap().archived["test"], vec!["b"]);
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.toggle_archive("b", cx);
                assert_eq!(this.visible_sessions(cx), vec![0, 1]);
            })
        });
        let _ = std::fs::remove_file(path);
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
                this.create(window, cx);
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
                assert_eq!(this.projects.cwd.as_str(), "/requested/project");
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
            assert!(matches!(command, Command::ConsumeUiRequest(_)) || project_read(&command));
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
