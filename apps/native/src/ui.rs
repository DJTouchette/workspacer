mod archive;
mod attachments;
mod bus_commands;
mod children;
mod chrome;
mod conversation;
mod default_shell;
mod explorer;
mod features;
mod file_viewer;
mod fleet_card;
mod gauge;
mod handoff;
mod island;
mod jobs;
mod launch;
mod markdown;
mod motion;
mod navigation;
mod projects;
mod questions;
mod recent;
mod remote;
mod review;
mod scroll;
mod settings;
mod sidebar;
mod smooth_scroll;
mod states;
mod syntax;
mod terminal;
mod titles;
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

/// Fade height above the conversation dock; within its 12px of transcript padding.
const DOCK_FADE: f32 = 12.;
/// Height the dock's cards keep however tall the composer grows, so an
/// approval or question set stays visible and scrollable beside a big draft.
const DOCK_CARDS_FLOOR: f32 = 72.;

/// Bundled provider marks and extra Lucide icons layered over
/// gpui-component's icon set.
pub struct Assets;

/// Embeds each bundled icon under its asset path.
macro_rules! bundled_icons {
    ($($path:literal),* $(,)?) => {
        fn bundled_icon(path: &str) -> Option<&'static [u8]> {
            match path {
                $($path => Some(include_bytes!(concat!("../assets/icons/", $path))),)*
                _ => None,
            }
        }
    };
}

bundled_icons!(
    "brand/claude.svg",
    "brand/openai.svg",
    "lucide/file-diff.svg",
    "lucide/message-square-plus.svg",
    "lucide/user-round.svg",
    "lucide/reply.svg",
    "lucide/megaphone.svg",
);

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match bundled_icon(path) {
            Some(bytes) => Ok(Some(std::borrow::Cow::Borrowed(bytes))),
            None => gpui_component_assets::Assets.load(path),
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
        if appearance.is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
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
    theme.colors.switch_thumb = rgb(if appearance.is_dark() {
        p.text
    } else {
        p.surface
    })
    .into();
    theme.colors.popover = rgb(p.surface).into();
    theme.colors.popover_foreground = rgb(p.text).into();
    // Text selection is translucent and painted beneath the glyphs (see the
    // vendored Inline patch), so selected text keeps its syntax colors.
    theme.colors.selection = gpui::rgba(p.selection).into();
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
        ComposerEnter,
        SubmitAnswers,
        AnswerEnter,
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
        ShowJobs,
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
        ViewerSave,
        OpenEditor,
        ToggleTerminal,
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
        KeyBinding::new("g j", ShowJobs, Some("VimNormal")),
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
        // Inside the question picker Ctrl/Cmd+Enter sends its answers, never
        // the composer draft. Same depth as the Input binding above, so these
        // must come after it to win.
        KeyBinding::new("ctrl-enter", SubmitAnswers, Some("QuestionPicker")),
        KeyBinding::new("cmd-enter", SubmitAnswers, Some("QuestionPicker")),
        KeyBinding::new("ctrl-enter", SubmitAnswers, Some("QuestionPicker > Input")),
        KeyBinding::new("cmd-enter", SubmitAnswers, Some("QuestionPicker > Input")),
        // Enter in a typed answer: send or move on, never a newline.
        KeyBinding::new("enter", AnswerEnter, Some("QuestionPicker > Input")),
        // Settings → Keyboard → Send with Enter: only the chat composer carries
        // the ComposerEnter context, so every other field keeps its Enter.
        KeyBinding::new("enter", ComposerEnter, Some("ComposerEnter > Input")),
        KeyBinding::new(
            "shift-enter",
            gpui_component::input::Enter { secondary: false },
            Some("ComposerEnter > Input"),
        ),
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
        KeyBinding::new("ctrl-s", ViewerSave, Some("FileViewer")),
        KeyBinding::new("cmd-s", ViewerSave, Some("FileViewer")),
        // The editor on the selected session's folder, and its terminal.
        KeyBinding::new("ctrl-shift-e", OpenEditor, Some("Workspace")),
        KeyBinding::new("cmd-shift-e", OpenEditor, Some("Workspace")),
        KeyBinding::new("g f", OpenEditor, Some("VimNormal")),
        KeyBinding::new("ctrl-`", ToggleTerminal, Some("Workspace")),
        KeyBinding::new("cmd-`", ToggleTerminal, Some("Workspace")),
        KeyBinding::new("g t", ToggleTerminal, Some("VimNormal")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-shift-q", Quit, None),
    ]);
}

pub struct Workspace {
    ui_bus: bus_commands::UiState,
    extras: features::Extras,
    explorer: explorer::Explorer,
    review: review::ReviewUi,
    terminal: terminal::TerminalUi,
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
    /// The dock's composer layer, measured: the cards above it get the rest
    /// of the dock's share of the window.
    composer_layer_height: gpui::Pixels,
    /// The title capsule's measured width inside its border, without what
    /// its actions add: its notice island wraps inside it.
    title_base_width: gpui::Pixels,
    /// Hover, focus and tap state behind the capsule's secondary actions.
    title_reveal: island::TitleReveal,
    /// How the capsule's notice island grows, settles and lets go.
    island_motion: island::IslandMotion,
    /// How the capsule's context hairline fills and its glow warms.
    gauge_motion: gauge::GaugeMotion,
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
        // Quit is application-wide (including editor/terminal popouts), but
        // shares the main window's unsaved-document guard. Defer to avoid
        // borrowing a window already in the key-dispatch stack.
        let quitting = cx.entity().downgrade();
        let main = window.window_handle();
        App::on_action(cx, move |_: &Quit, cx| {
            let quitting = quitting.clone();
            cx.defer(move |cx| {
                if let Some(workspace) = quitting.upgrade() {
                    let _ = main.update(cx, |_, window, cx| {
                        if workspace.update(cx, |ws, cx| ws.confirm_window_close(window, cx)) {
                            cx.quit();
                        }
                    });
                }
            });
        });
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
        focus_watch.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.viewer_modal(window) && !this.viewer_has_focus(window, cx) {
                    this.hold_viewer_focus(window, cx);
                    cx.stop_propagation();
                } else if this.terminal_keystroke(&event.keystroke, window, cx) {
                    // A focused terminal takes keys ahead of every binding.
                    cx.stop_propagation();
                }
            });
        }));
        focus_watch.push(cx.on_release(|this, cx| {
            // A popped-out file viewer or terminal never outlives its workspace.
            this.close_popout_window(cx);
            this.close_terminal_popout(cx);
            if let Some(path) = &this.settings_path
                && let Err(error) = this.settings.save(path)
            {
                eprintln!("Could not save native settings: {error}");
            }
        }));
        window.focus(&focus);
        Self {
            extras,
            explorer: Default::default(),
            review: Default::default(),
            terminal: Default::default(),
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
            composer_layer_height: Default::default(),
            title_base_width: Default::default(),
            title_reveal: island::TitleReveal::new(cx),
            island_motion: Default::default(),
            gauge_motion: Default::default(),
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
        self.sync_explorer(&view, cx);
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
                    let _ = self.controller.command(Command::Request(
                        wks_native::features::Request::TouchProject {
                            path: launched,
                            at: timing::now_ms(),
                        },
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
            self.note_model_receipt(receipt);
            if let Some((signature, answers)) = &self.extras.answer_submission {
                let ours = matches!(&receipt.action, Action::Answers(sent) if sent == answers)
                    && view.selected.as_ref() == Some(&receipt.session);
                if ours {
                    let same_question = *signature == self.extras.question_signature;
                    self.extras.answers_sent = receipt.error.is_none() && same_question;
                    self.extras.answer_error =
                        same_question.then(|| receipt.error.clone()).flatten();
                    self.extras.answer_submission = None;
                } else if !view.busy {
                    // Another control won admission before our queued command;
                    // the controller is idle, so permit an explicit retry.
                    self.extras.answer_submission = None;
                }
            }
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
                // A viewed child's rows are not the parent's turns.
                let transcript = (!view.loading
                    && view.child.is_none()
                    && view.selected.as_ref() == Some(&session.id))
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
            .filter(|_| view.child.is_none())
            .and_then(|id| self.turn_clocks.get(id))
            .map(|clock| clock.message_labels(&view.transcript))
            .unwrap_or_default();
        self.sync_projects(&view);
        self.view = view;
        self.lift_child_clears(cx);
        self.ensure_project_icons(cx);
        self.resume_explorer(cx);
        self.sync_terminals(window, cx);
        self.sync_review(cx);
        self.hand_off_update(window, cx);
        self.land_on_latest();
        if self.new_session || matches!(self.screen, Screen::Model | Screen::Handoff) {
            self.sync_models(window, cx);
            if reconnected {
                self.load_models(true, cx);
            }
        }
        if self.screen == Screen::Settings && !self.new_session {
            self.sync_title_picker(window, cx);
            if reconnected {
                self.load_titles(cx);
                self.load_terminal_shell(cx);
            }
        }
        // Project names and icons appear beside every session, so the shared
        // registry is read on every (re)connection, not only on Projects.
        if reconnected {
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
        let attachments = self
            .extras
            .attachments
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.extras.sent_drafts.insert(id.clone(), draft);
        self.extras.sent_attachments.insert(id, attachments);
        self.act(Action::Send(text), cx);
    }

    /// Enter in the composer when Enter sends. An IME composition keeps the
    /// key (it commits the composition), and so does any state where the
    /// draft cannot be sent: nothing is inserted, nothing is lost.
    fn composer_enter(&mut self, _: &ComposerEnter, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self
            .composer
            .update(cx, |input, cx| {
                gpui::EntityInputHandler::marked_text_range(input, window, cx)
            })
            .is_some_and(|range| !range.is_empty());
        if composing || !self.settings.enter_sends {
            cx.propagate();
            return;
        }
        self.send(&SendMessage, window, cx);
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
        if self.screen == Screen::Recent && !self.new_session {
            self.move_recent_cursor(step, cx);
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

impl Workspace {
    /// Input's auto-grow minimum belongs to its text element. Adjust that row
    /// budget itself, so caret scrolling and painting use the real viewport
    /// instead of overflowing an externally capped outer box.
    fn fit_composer_rows(&self, window: &Window, cx: &mut Context<Self>) {
        let height = window.viewport_size().height;
        let share = if unzoom(height) < 300. { 0.10 } else { 0.14 };
        let rows = (f32::from(height) * share / (f32::from(window.rem_size()) * 1.25))
            .floor()
            .clamp(1., 4.) as usize;
        self.composer
            .update(cx, |input, cx| input.set_auto_grow_rows(1, rows, cx));
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_composer_rows(window, cx);
        let narrow = window.viewport_size().width < px(900.);
        let compact = window.viewport_size().height < px(620.);
        // Inset so the panel floats with the same rounding as cards and the composer.
        let sidebar = div()
            .h_full()
            .flex_shrink_0()
            .py_2()
            .pl_2()
            .child(self.render_sidebar(narrow, compact, window, cx));
        let content = if self.new_session {
            self.render_new_session(window, cx).into_any_element()
        } else {
            match self.screen {
                Screen::Changes => self.render_review(window, cx).into_any_element(),
                Screen::Recent
                | Screen::Jobs
                | Screen::History
                | Screen::Session
                | Screen::Setup
                | Screen::Model
                | Screen::Handoff => self.render_feature(window, cx).into_any_element(),
                Screen::Projects => self.render_projects(window, cx).into_any_element(),
                Screen::Settings => self.render_settings(window, cx).into_any_element(),
                Screen::Conversation => self
                    .render_conversation(narrow, compact, window, cx)
                    .into_any_element(),
            }
        };
        self.shell(window, cx)
            .child(sidebar)
            .child(content)
            .children(self.render_docked_viewer(window, cx))
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
