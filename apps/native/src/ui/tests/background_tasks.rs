//! Background tasks: the title-bar chip, the task list, a following log,
//! agents opening as their own chat, and a confirmed stop.
use super::*;
use wks_native::features::RequestState;

fn task_rows() -> serde_json::Value {
    serde_json::json!([
        {"id":"sh1","taskType":"local_bash","status":"running","description":"npm run dev",
         "startedAt":1,"hasOutput":true,"pid":4242},
        {"id":"ag1","taskType":"local_agent","status":"running","description":"explore the repo",
         "startedAt":2,"subagentId":"ag1"},
        {"id":"old","taskType":"local_bash","status":"completed","description":"cargo build",
         "startedAt":1,"endedAt":5,"hasOutput":true}
    ])
}

/// Session `a` with the given daemon task fields.
fn with_tasks(fields: serde_json::Value) -> View {
    let mut view = state("a");
    Arc::make_mut(&mut view.sessions)[0].merge(&fields);
    view
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    let at = bounds_of(visual, selector).center();
    visual.simulate_click(at, gpui::Modifiers::default());
    visual.run_until_parked();
}

fn drain(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn log_request(commands: &[Command]) -> Option<Option<u64>> {
    commands.iter().find_map(|c| match c {
        Command::Request(Request::TaskOutput {
            session,
            task,
            offset,
        }) if session == "a" && task == "sh1" => Some(*offset),
        _ => None,
    })
}

/// The controller's answer to a log read, folded into the view.
fn log_answer(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    number: u64,
    offset: Option<u64>,
    value: serde_json::Value,
) {
    let mut view = (*workspace.read_with(visual, |this, _| this.view.clone())).clone();
    view.requests.insert(
        "task-output",
        RequestState {
            number,
            request: Request::TaskOutput {
                session: "a".into(),
                task: "sh1".into(),
                offset,
            },
            loading: false,
            value: Arc::new(value),
            error: None,
        },
    );
    show(workspace, visual, view);
}

#[gpui::test]
fn the_chip_counts_running_tasks_and_hides_without_any(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, state("a"));
    assert!(
        visual.debug_bounds("title-tasks").is_none(),
        "no work, no chip"
    );
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"background_tasks":2,"background_task_list":task_rows()})),
    );
    assert!(visual.debug_bounds("title-tasks").is_some());
    let running = workspace.read_with(&visual, |this, _| {
        wks_native::background_tasks::running(&this.view.sessions[0].tasks)
    });
    assert_eq!(running, 2, "the finished build does not count");
}

#[gpui::test]
fn a_pty_session_shows_its_count_and_says_why_there_is_no_list(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"backgroundTasks":3})),
    );
    click(&mut visual, "title-tasks");
    assert!(visual.debug_bounds("tasks-empty").is_some());
    assert!(visual.debug_bounds("task-row-sh1").is_none());
}

#[gpui::test]
fn a_shell_log_reads_the_tail_then_follows_from_where_it_stopped(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    // Wide enough that the panel docks beside the conversation.
    visual.simulate_resize(gpui::size(px(1400.), px(800.)));
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"background_tasks":2,"background_task_list":task_rows()})),
    );
    click(&mut visual, "title-tasks");
    assert!(
        visual.debug_bounds("tasks-panel").is_some(),
        "docked when it fits"
    );
    assert!(visual.debug_bounds("tasks-backdrop").is_none());
    for row in ["task-row-sh1", "task-row-ag1", "task-row-old"] {
        assert!(visual.debug_bounds(row).is_some(), "{row}");
    }
    drain(&mut commands);
    click(&mut visual, "task-row-sh1");
    assert_eq!(
        log_request(&drain(&mut commands)),
        Some(None),
        "the first read asks for the tail"
    );
    assert!(visual.debug_bounds("task-log-empty").is_some());
    log_answer(
        &workspace,
        &mut visual,
        5,
        None,
        serde_json::json!({"session_id":"a","task_id":"sh1","offset":0,"next_offset":12,
            "size":12,"text":"ready\nGET /\n","running":true,"status":"running","done":false,
            "process":{"pid":4242,"alive":true,"cpu_percent":1.5,"rss_bytes":1048576}}),
    );
    assert!(visual.debug_bounds("task-log-lines").is_some());
    assert!(visual.debug_bounds("task-log-process").is_some());
    let lines = workspace.read_with(&visual, |this, _| this.tasks.log.clone().unwrap());
    assert_eq!(lines.lines(), ["ready", "GET /"]);
    // The clock's next read continues from the end of what was read.
    visual.update(|_, cx| workspace.update(cx, |this, cx| this.poll_task_log(cx)));
    assert_eq!(log_request(&drain(&mut commands)), Some(Some(12)));
    log_answer(
        &workspace,
        &mut visual,
        6,
        Some(12),
        serde_json::json!({"session_id":"a","task_id":"sh1","offset":12,"next_offset":20,
            "size":20,"text":"GET /a\n","running":true,"status":"running","done":false}),
    );
    let log = workspace.read_with(&visual, |this, _| this.tasks.log.clone().unwrap());
    assert_eq!(log.lines(), ["ready", "GET /", "GET /a"]);
    // Back to the list.
    click(&mut visual, "tasks-back");
    assert!(visual.debug_bounds("task-row-sh1").is_some());
    click(&mut visual, "tasks-close");
    // (debug_bounds can outlive an element by a frame; the state is the truth.)
    assert!(!workspace.read_with(&visual, |this, _| this.tasks.open));
}

#[gpui::test]
fn an_agent_task_opens_its_own_conversation(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(gpui::size(px(1400.), px(800.)));
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"background_tasks":2,"background_task_list":task_rows()})),
    );
    click(&mut visual, "title-tasks");
    drain(&mut commands);
    click(&mut visual, "task-row-ag1");
    let issued = drain(&mut commands);
    assert!(
        issued.iter().any(|c| matches!(c,
            Command::ViewChild(Some(t)) if t.parent == "a" && t.agent == "ag1")),
        "expected command not issued"
    );
    assert_eq!(log_request(&issued), None, "agents have no log here");
}

#[gpui::test]
fn stopping_asks_first_and_sends_the_task_stop(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(gpui::size(px(1400.), px(800.)));
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"background_tasks":2,"background_task_list":task_rows()})),
    );
    click(&mut visual, "title-tasks");
    assert!(
        visual.debug_bounds("task-stop-old").is_none(),
        "a finished task offers no stop"
    );
    drain(&mut commands);
    click(&mut visual, "task-stop-sh1");
    let none = drain(&mut commands);
    assert!(
        !none
            .iter()
            .any(|c| matches!(c, Command::Request(Request::TaskStop { .. }))),
        "the first click only asks"
    );
    click(&mut visual, "task-stop-confirm-sh1");
    let issued = drain(&mut commands);
    assert!(
        issued.iter().any(|c| matches!(c,
            Command::Request(Request::TaskStop { session, task }) if session == "a" && task == "sh1")),
        "expected command not issued"
    );
    // The stop's answer surfaces as a notice in the panel.
    let mut view = (*workspace.read_with(&visual, |this, _| this.view.clone())).clone();
    view.requests.insert(
        "task-stop",
        RequestState {
            number: 9,
            request: Request::TaskStop {
                session: "a".into(),
                task: "sh1".into(),
            },
            loading: false,
            value: Arc::new(serde_json::json!({"ok":true})),
            error: None,
        },
    );
    show(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("tasks-notice").is_some());
}

#[gpui::test]
fn narrow_windows_show_the_panel_as_a_sheet(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(
        &workspace,
        &mut visual,
        with_tasks(serde_json::json!({"background_tasks":1,"background_task_list":task_rows()})),
    );
    click(&mut visual, "title-tasks");
    assert!(visual.debug_bounds("tasks-sheet").is_some());
    assert!(visual.debug_bounds("tasks-panel").is_none());
}
