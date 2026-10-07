//! Workspace UI tests on the GPUI test platform, one file per area. Fixtures
//! and helpers shared by more than one area live here.
use super::*;
use gpui::{TestAppContext, VisualTestContext, size};
use gpui_component::Root;
use wks_native::{
    controller::Receipt,
    features::Request,
    model::{ConversationSnapshot, Item, Session, Transcript},
};

mod archiving;
mod background_tasks;
mod bus_requests;
mod chat;
mod child_agents;
mod composer;
mod connection;
mod controls;
mod history_screen;
mod job_list;
mod new_agent;
mod preferences;
mod project_registry;
mod question_sets;
mod scrolling;
mod session_list;
mod switching;
mod tables;
mod title;
mod viewer;
mod window;

/// A demo workspace in a 1000×700 window, with the commands it issues and
/// the sender that feeds it views.
pub(super) fn fixture(
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
        let view = cx.new(|cx| {
            let mut workspace = Workspace::new(controller, true, window, cx);
            // The test platform draws no animation frames: chrome
            // springs would rest mid-flight. Motion has its own tests.
            workspace.settings.reduce_motion = true;
            workspace
        });
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
                Request::Projects | Request::InspectProject { .. } | Request::BrowseFolders { .. }
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

fn pane_of(
    workspace: &Entity<Workspace>,
    visual: &VisualTestContext,
) -> Entity<file_viewer::PreviewPane> {
    workspace.read_with(visual, |this, _| {
        this.file_viewer().cloned().expect("viewer open")
    })
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

/// Whether the last frame painted an accent border around `bounds`.
fn accent_border(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    bounds: gpui::Bounds<gpui::Pixels>,
) -> bool {
    let accent: gpui::Hsla =
        rgb(workspace.read_with(visual, |this, _| this.appearance.palette().accent)).into();
    let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() < px(0.5);
    visual.update(|window, _| {
        window.rendered_borders().into_iter().any(|(quad, color)| {
            color == accent
                && near(quad.left(), bounds.left())
                && near(quad.top(), bounds.top())
                && near(quad.size.width, bounds.size.width)
                && near(quad.size.height, bounds.size.height)
        })
    })
}

/// Bounds of `selector`, or a panic naming it.
fn bounds_of(visual: &mut VisualTestContext, selector: &str) -> gpui::Bounds<gpui::Pixels> {
    // `debug_bounds` wants a `'static` selector; tests leak a few.
    let key: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    visual
        .debug_bounds(key)
        .unwrap_or_else(|| panic!("{selector} is not rendered"))
}

/// Every effect that is not a passive read, for "archive sent only an
/// archive" checks.
fn archive_effects(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter(|c| {
            !project_read(c)
                && !matches!(
                    c,
                    Command::Terminal(wks_native::terminal::Command::Resize { .. })
                )
        })
        .collect()
}

/// Session "a" waiting on a mixed set: single choice with descriptions,
/// multiple choice, and free text.
fn question_state() -> View {
    let mut next = state("a");
    let session = &mut Arc::make_mut(&mut next.sessions)[0];
    session.state = "question".into();
    session.questions = Some(serde_json::json!([
        {"header":"Approach","question":"Which migration strategy?","multiSelect":false,"options":[
            {"label":"Online backfill","description":"Copy rows in batches while the hub keeps serving."},
            {"label":"Stop-the-world","description":"Pause writers and migrate in one transaction."},
            {"label":"Skip for now"}]},
        {"header":"Checks","question":"Which checks should run?","multiSelect":true,"options":[
            {"label":"cargo test"},{"label":"Clippy, strict"},{"label":"rustfmt --check"}]},
        {"header":"Reviewer","question":"Anything the reviewer should know?","options":[]}
    ]));
    next
}

/// A full key press: test keystrokes are key-down only, and GPUI
/// activates a focused control on key-up.
fn press(visual: &mut VisualTestContext, key: &str) {
    visual.simulate_keystrokes(key);
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(key).unwrap(),
    });
    visual.run_until_parked();
}

/// Put any request state into the view, as the controller does.
fn request_state(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    request: wks_native::features::Request,
    number: u64,
    value: serde_json::Value,
) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                request.key(),
                wks_native::features::RequestState {
                    request,
                    number,
                    loading: false,
                    error: None,
                    value: Arc::new(value),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

/// Rests the pointer on the title capsule and lets its secondary
/// actions finish showing (the test platform draws no animation frames).
fn reveal_title(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    let bar = visual.debug_bounds("title-bar").unwrap();
    visual.simulate_mouse_move(bar.center(), None, gpui::Modifiers::default());
    visual.run_until_parked();
    settle_title(workspace, visual);
}

/// Finishes the capsule's reveal wherever it is headed.
fn settle_title(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settle_title_reveal();
            cx.notify();
        })
    });
    visual.run_until_parked();
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();
}

/// Commands, without the terminal resizes painting produces.
fn effects(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter(|c| {
            !matches!(
                c,
                Command::Terminal(wks_native::terminal::Command::Resize { .. })
            )
        })
        .collect()
}

/// The 2026-10-04 fleet: a Codex manager in the repo checkout and Claude
/// workers in their own worktrees, as `sessions.snapshots` reports them.
fn fleet_sessions() -> Vec<Session> {
    let worker = |id: &str, label: &str, tree: &str| Session {
        id: id.into(),
        label: label.into(),
        provider: "claude".into(),
        model: "claude-opus-4-5".into(),
        parent_session_id: "manager".into(),
        cwd: format!("/home/u/.workspacer/worktrees/workspacer/{tree}"),
        state: "responding".into(),
        ..Default::default()
    };
    vec![
        worker(
            "worker-3",
            "Native themes · visuals · usage",
            "native-themes-visuals-usage",
        ),
        worker(
            "worker-2",
            "Native controls · projects · Codex",
            "native-controls-projects-codex",
        ),
        worker(
            "worker-1",
            "Native editor · terminal · Git",
            "native-editor-terminal-git",
        ),
        Session {
            id: "unrelated".into(),
            label: "Other project".into(),
            cwd: "/home/u/Work/other".into(),
            state: "input".into(),
            ..Default::default()
        },
        Session {
            id: "manager".into(),
            label: "Fleet manager".into(),
            provider: "codex".into(),
            cwd: "/home/u/Work/worky/workspacer".into(),
            state: "input".into(),
            ..Default::default()
        },
    ]
}
