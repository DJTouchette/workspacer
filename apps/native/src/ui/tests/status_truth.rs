//! r4-truth (R4.1, R4.2): one status per session. Every surface (the header
//! strip, the composer, the sidebar row and its group, History, the Fleet
//! tile and its counts, the command palette) reads it from the one
//! derivation, so for each state they all agree.
use super::*;
use crate::ui::attention::Bucket;
use crate::ui::primitives::StatusKind;
use crate::ui::shell::ShellRow;
use crate::ui::sidebar::SidebarRow;
use wks_native::appearance::Design;
use wks_native::background_tasks::{Kind, Task};

fn now() -> i64 {
    timing::now_ms()
}

fn session(id: &str, state: &str) -> Session {
    Session {
        id: id.into(),
        label: id.to_uppercase(),
        state: state.into(),
        provider: "claude".into(),
        cwd: "/home/dev/src/workspacer".into(),
        ..Default::default()
    }
}

fn shell_task(id: &str, status: &str, at: i64) -> Task {
    Task {
        id: id.into(),
        kind: Kind::of("local_bash"),
        status: status.into(),
        description: "cargo test --locked".into(),
        started_at: Some(at),
        ended_at: Some(at + 1_000),
        ..Default::default()
    }
}

/// One session in each state, and the state every surface must show.
fn every_state() -> (View, Vec<(&'static str, StatusKind)>) {
    let now = now();
    let mut fail = session("fail", "input");
    fail.tasks = vec![shell_task("t1", "failed", now - 60_000)];
    fail.telemetry.last_activity_ms = Some(now - 1_000);
    let mut need = session("need", "input");
    need.approval = Some(serde_json::json!({"toolName": "Bash",
        "toolInput": {"command": "cargo test"}}));
    let mut stall = session("stall", "working");
    stall.telemetry.last_activity_ms = Some(now - 10 * 60_000);
    let mut work = session("work", "working");
    work.telemetry.last_activity_ms = Some(now - 2_000);
    // Its own turn is over; a subagent of it is at work.
    let mut kids = session("kids", "input");
    kids.telemetry.last_activity_ms = Some(now - 3_000);
    kids.subagents = Arc::new(serde_json::json!([{"id": "k1", "type": "Explore",
        "description": "Trace restore", "status": "running",
        "startedAt": now - 60_000, "lastActivity": now - 4_000}]));
    let mut idle = session("idle", "input");
    idle.telemetry.last_activity_ms = Some(now - 5_000);
    let end = session("end", "ended");
    let pause = session("pause", "stopped");
    let view = View {
        connected: true,
        selected: Some("idle".into()),
        sessions: Arc::new(vec![fail, need, stall, work, kids, idle, end, pause]),
        session_open: Some(Arc::new(serde_json::json!({"version": 1,
            "open": {"pause": 1}, "ended": {}}))),
        ..Default::default()
    };
    let expected = vec![
        ("fail", StatusKind::Failed),
        ("need", StatusKind::NeedsYou),
        ("stall", StatusKind::NotResponding),
        ("work", StatusKind::Working),
        ("kids", StatusKind::Working),
        ("idle", StatusKind::Idle),
        ("end", StatusKind::Idle),
        ("pause", StatusKind::Paused),
    ];
    (view, expected)
}

fn open(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(Design::V2));
            this.fleet.all = true;
            this.shell.folded = [false; 4];
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

/// What each surface shows for session `id`.
#[derive(Debug, PartialEq)]
struct Surfaces {
    header: StatusKind,
    sidebar_row: StatusKind,
    sidebar_group: Bucket,
    palette: Option<StatusKind>,
    fleet_tile: Option<StatusKind>,
    fleet_group: Option<Bucket>,
    history: StatusKind,
}

fn surfaces(this: &Workspace, id: &str, cx: &App) -> Surfaces {
    let now = now();
    let ix = this.view.sessions.iter().position(|s| s.id == id).unwrap();
    let live = &this.view.sessions[ix];
    // History lists the hub's stored copy: stopped, no live state.
    let stored = Session {
        id: live.id.clone(),
        label: live.label.clone(),
        state: "stopped".into(),
        ..Default::default()
    };
    let group = this.shell_rows(cx).into_iter().find_map(|row| match row {
        ShellRow::Item {
            row: SidebarRow::Session { index, .. },
            bucket,
            ..
        } if index == ix => Some(bucket),
        _ => None,
    });
    Surfaces {
        header: this.title_kind(live),
        sidebar_row: this.session_kind(live, now),
        sidebar_group: group.expect("listed"),
        palette: this.palette_session_status(ix, cx),
        fleet_tile: this
            .fleet_tiles(cx)
            .iter()
            .find(|t| t.index == ix)
            .map(|t| t.kind),
        fleet_group: this
            .fleet_sessions(cx)
            .into_iter()
            .find(|(i, _, _)| *i == ix)
            .map(|(_, _, b)| b),
        history: this.session_status_v2(&stored).kind,
    }
}

fn agreed(kind: StatusKind, group: Bucket) -> Surfaces {
    Surfaces {
        header: kind,
        sidebar_row: kind,
        sidebar_group: group,
        palette: Some(kind),
        fleet_tile: Some(kind),
        fleet_group: Some(group),
        history: kind,
    }
}

fn group_of(kind: StatusKind, id: &str) -> Bucket {
    match kind {
        StatusKind::Failed | StatusKind::NeedsYou | StatusKind::NotResponding => Bucket::NeedsYou,
        StatusKind::Working => Bucket::Working,
        _ if id == "end" => Bucket::Ended,
        _ => Bucket::Idle,
    }
}

#[gpui::test]
fn every_surface_agrees_for_each_state(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let (view, expected) = every_state();
    open(&workspace, &mut visual, view);
    workspace.read_with(&visual, |this, cx| {
        for (id, kind) in &expected {
            assert_eq!(
                surfaces(this, id, cx),
                agreed(*kind, group_of(*kind, id)),
                "{id}"
            );
        }
        // The counts the sidebar's headers show are the Fleet's.
        let headers: Vec<(Bucket, usize)> = this
            .shell_rows(cx)
            .into_iter()
            .filter_map(|row| match row {
                ShellRow::Group { bucket, count, .. } => Some((bucket, count)),
                _ => None,
            })
            .collect();
        let counts = this.attention_counts(cx);
        for (bucket, count) in &headers {
            assert_eq!(counts[bucket.index()], *count, "{bucket:?}");
        }
        assert_eq!(
            counts,
            [3, 2, 2, 1],
            "needs you: fail, need, stall · working: work, kids · idle: idle, pause · ended"
        );
        // The composer says the stall and the subagents as the header does.
        assert_eq!(this.composer_state(false), None, "idle says nothing");
    });

    for (id, kind, words) in [
        ("stall", StatusKind::NotResponding, "Not responding · 10m"),
        ("kids", StatusKind::Working, "1 subagent working"),
    ] {
        workspace.update_in(&mut visual, |this, window, cx| {
            let mut view = (*this.view).clone();
            view.selected = Some(id.into());
            this.update_view(Arc::new(view), window, cx);
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.composer_state(false),
                Some((kind, words.to_owned())),
                "{id}"
            );
        });
    }
}

#[gpui::test]
fn streaming_is_never_not_responding_anywhere(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let (mut view, _) = every_state();
    view.selected = Some("stall".into());
    open(&workspace, &mut visual, view);
    // Output arrives in its open conversation: it is heard from.
    workspace.update(&mut visual, |this, cx| {
        this.shell.output_at = Some(("stall".into(), now()));
        cx.notify();
    });
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            surfaces(this, "stall", cx),
            agreed(StatusKind::Working, Bucket::Working)
        );
        assert_eq!(this.composer_state(true), None);
    });
}

#[gpui::test]
fn a_failure_clears_everywhere_once_the_same_command_passes(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let (mut view, _) = every_state();
    view.selected = Some("fail".into());
    let mut sessions = (*view.sessions).clone();
    sessions[0]
        .tasks
        .push(shell_task("t2", "completed", now() - 30_000));
    view.sessions = Arc::new(sessions);
    open(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("title-failed").is_none(), "no red chip");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            surfaces(this, "fail", cx),
            agreed(StatusKind::Idle, Bucket::Idle)
        );
    });
}

#[gpui::test]
fn the_failure_chip_names_its_task(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let (mut view, _) = every_state();
    view.selected = Some("fail".into());
    open(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("title-failed").is_some());
    workspace.read_with(&visual, |this, _| {
        let fail = &this.view.sessions[0];
        let task = this.open_failures(fail)[0];
        assert_eq!(
            crate::ui::status::failure_label(task, &this.session_title(fail), 28),
            "cargo test failed · FAIL"
        );
    });
}

#[gpui::test]
fn a_not_responding_session_offers_nudge_and_stop_in_the_header(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    // (gpui's test `debug_bounds` keeps an element's last bounds after it
    // stops drawing, so the working session is shown first.)
    let (mut view, _) = every_state();
    view.selected = Some("work".into());
    open(&workspace, &mut visual, view);
    assert!(
        visual.debug_bounds("title-nudge").is_none(),
        "Working offers neither"
    );
    assert!(visual.debug_bounds("title-stop").is_none());
    workspace.update_in(&mut visual, |this, window, cx| {
        let mut view = (*this.view).clone();
        view.selected = Some("stall".into());
        this.update_view(Arc::new(view), window, cx);
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("title-nudge").is_some());
    assert!(visual.debug_bounds("title-stop").is_some());
    effects(&mut commands);
    click(&mut visual, "title-nudge");
    let sent = effects(&mut commands).into_iter().any(|c| {
        matches!(&c, Command::Act { session, action: Action::SendNow(text) }
            if session == "stall" && text == crate::ui::transcript_v2::NUDGE_TEXT)
    });
    assert!(
        sent,
        "Nudge sends the stalled session a line to read at once"
    );
}

#[gpui::test]
fn job_proposals_count_in_needs_you(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    let mut idle = session("idle", "input");
    idle.telemetry.last_activity_ms = Some(now());
    open(
        &workspace,
        &mut visual,
        View {
            connected: true,
            selected: Some("idle".into()),
            sessions: Arc::new(vec![idle]),
            ..Default::default()
        },
    );
    // Nothing waits: All clear.
    assert!(visual.debug_bounds("sidebar-all-clear").is_some());
    assert!(visual.debug_bounds("jobs-needs-you-badge").is_none());
    request_state(
        &workspace,
        &mut visual,
        wks_native::features::Request::Jobs,
        1,
        serde_json::json!({"jobs": [
            {"id": "prune", "name": "Prune merged worktrees", "enabled": false,
             "proposedBy": "Release checklist", "trigger": {"kind": "daily", "at": "18:00"},
             "action": {"kind": "shell", "shell": {"command": "git worktree prune"}}}
        ]}),
    );
    assert!(visual.debug_bounds("sidebar-proposal-1").is_some());
    assert!(visual.debug_bounds("jobs-needs-you-badge").is_some());
    workspace.read_with(&visual, |this, cx| {
        assert!(
            !this
                .shell_rows(cx)
                .iter()
                .any(|row| matches!(row, ShellRow::AllClear)),
            "not all clear while a proposal waits"
        );
        let counts = this.attention_counts(cx);
        assert_eq!(counts[Bucket::NeedsYou.index()], 1);
        let header = this.shell_rows(cx).into_iter().find_map(|row| match row {
            ShellRow::Group {
                bucket: Bucket::NeedsYou,
                count,
                ..
            } => Some(count),
            _ => None,
        });
        assert_eq!(header, Some(1));
    });
}
