//! Design v2 panels: Changes as a dock with a stacked diff, and the
//! background tasks' badges, retry and confirmed stop.
use super::*;
use wks_native::features::{DiffTarget, GitAction, Request};

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

fn v2(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, design: Design) {
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.force_design(Some(design));
            cx.notify();
        })
    });
    visual.run_until_parked();
}

fn open_changes(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_feature(Screen::Changes, window, cx)
        })
    });
    visual.run_until_parked();
}

const STATUS: &str = r#"{"branch": "main", "root": "/repo", "ahead": 1, "upstream": "origin/main", "files": [
    {"path": "src/lib.rs", "staged": " ", "unstaged": "M"},
    {"path": "new.txt", "staged": "?", "unstaged": "?"}
]}"#;

/// Under v2, Changes docks beside the conversation and reads every file's
/// diff at once; files stack under sticky headers whose Stage and overflow
/// Discard (after a confirmation) write through git; expand opens the page.
#[gpui::test]
fn changes_dock_beside_the_chat_with_every_file_stacked(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    let mut view = state("a");
    Arc::make_mut(&mut view.sessions)[0].cwd = "/repo/sub".into();
    show(&workspace, &mut visual, view);
    v2(&workspace, &mut visual, Design::V2);
    open_changes(&workspace, &mut visual);
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation, "the chat stays");
        assert!(this.review_dock_showing());
    });
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Request(Request::Changes { cwd })] if cwd == "/repo/sub"));
    request_state(
        &workspace,
        &mut visual,
        Request::Changes {
            cwd: "/repo/sub".into(),
        },
        1,
        serde_json::from_str(STATUS).unwrap(),
    );
    let files = match effects(&mut commands).as_slice() {
        [Command::Request(Request::DiffSet { cwd, files })] => {
            assert_eq!(cwd, "/repo/sub");
            files.clone()
        }
        other => panic!("expected one diff-set read, got {} commands", other.len()),
    };
    assert_eq!(
        files,
        [
            DiffTarget {
                path: "src/lib.rs".into(),
                staged: false,
                untracked: false,
            },
            DiffTarget {
                path: "new.txt".into(),
                staged: false,
                untracked: true,
            },
        ]
    );
    request_state(
        &workspace,
        &mut visual,
        Request::DiffSet {
            cwd: "/repo/sub".into(),
            files,
        },
        2,
        serde_json::json!({"files": [
            {"path": "src/lib.rs", "diff": "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n@@ -40,2 +40,2 @@ fn later()\n a\n-b\n+c\n"},
            {"path": "new.txt", "diff": "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n"},
        ]}),
    );
    let dock = bounds_of(&mut visual, "review-dock");
    let chat = bounds_of(&mut visual, "conversation-dock");
    assert!(dock.left() >= chat.right(), "the dock is right of the chat");
    bounds_of(&mut visual, "review-stack");
    bounds_of(&mut visual, "review-stack-header-src/lib.rs");
    bounds_of(&mut visual, "review-stack-header-new.txt");
    // Hunks are indexed for ]c / [c, folds included.
    workspace.read_with(&visual, |this, _| {
        let layout = this.review.v2.laid_out().expect("a laid-out stack");
        assert_eq!(layout.hunks.len(), 3);
        assert_eq!(layout.headers.len(), 2);
    });
    click(&mut visual, "review-stack-stage-src/lib.rs");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Request(Request::Git(GitAction::Stage { path: Some(path), .. }))]
            if path == "src/lib.rs"));
    // Discard lives in the overflow menu and asks first.
    click(&mut visual, "review-stack-more-src/lib.rs");
    click(&mut visual, "review-discard");
    assert!(effects(&mut commands).is_empty(), "the menu only asks");
    bounds_of(&mut visual, "review-discard-dialog");
    click(&mut visual, "review-discard-confirm");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Request(Request::Git(GitAction::Discard { path, .. }))]
            if path == "src/lib.rs"));
    // Split view pairs the sides; the choice is remembered.
    click(&mut visual, "review-split");
    assert!(workspace.read_with(&visual, |this, _| this.settings.review_split));
    // Expand opens the full page; the dock waits behind it.
    click(&mut visual, "review-dock-expand");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Changes));
    bounds_of(&mut visual, "review-view");
    bounds_of(&mut visual, "review-diff-card");
    bounds_of(&mut visual, "review-files");
}

/// Opening Changes again closes the dock; under v1 it stays a page.
#[gpui::test]
fn changes_toggles_the_dock_and_v1_keeps_the_page(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    show(&workspace, &mut visual, state("a"));
    v2(&workspace, &mut visual, Design::V2);
    open_changes(&workspace, &mut visual);
    assert!(workspace.read_with(&visual, |this, _| this.review_dock_showing()));
    open_changes(&workspace, &mut visual);
    assert!(!workspace.read_with(&visual, |this, _| this.review_dock_showing()));
    v2(&workspace, &mut visual, Design::V1);
    open_changes(&workspace, &mut visual);
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Changes);
        assert!(!this.review_dock_showing());
    });
    assert!(visual.debug_bounds("review-dock").is_none());
}

fn with_tasks() -> View {
    let mut view = state("a");
    Arc::make_mut(&mut view.sessions)[0].merge(&serde_json::json!({
        "background_tasks": 1,
        "background_task_list": [
            {"id":"sh1","taskType":"local_bash","status":"running","description":"npm run dev",
             "startedAt":1,"hasOutput":true,"pid":4242},
            {"id":"bad","taskType":"local_bash","status":"failed",
             "description":"cargo test --locked","error":"exit code 101",
             "startedAt":1,"endedAt":5,"hasOutput":true},
        ],
    }));
    view
}

/// v2 tasks: status badges, a failed task's Open log and Retry (drafted in
/// the composer for the user to send), Copy, and a confirmed Stop.
#[gpui::test]
fn v2_tasks_offer_retry_open_log_and_a_confirmed_stop(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    show(&workspace, &mut visual, with_tasks());
    v2(&workspace, &mut visual, Design::V2);
    click(&mut visual, "title-tasks");
    bounds_of(&mut visual, "task-status-sh1");
    bounds_of(&mut visual, "task-status-bad");
    bounds_of(&mut visual, "task-copy-bad");
    assert!(visual.debug_bounds("task-retry-sh1").is_none());
    click(&mut visual, "task-retry-bad");
    workspace.read_with(&visual, |this, cx| {
        let draft = this.composer.read(cx).value().to_string();
        assert!(draft.contains("cargo test --locked"), "{draft}");
        assert!(draft.contains("exit code 101"), "{draft}");
    });
    effects(&mut commands);
    click(&mut visual, "task-stop-sh1");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::TaskStop { .. }))),
        "the first click only asks"
    );
    click(&mut visual, "task-stop-confirm-sh1");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Request(Request::TaskStop { task, .. }) if task == "sh1")));
    click(&mut visual, "task-open-log-bad");
    let log = workspace.read_with(&visual, |this, _| this.tasks.log.clone());
    assert_eq!(log.map(|l| l.task), Some("bad".to_owned()));
}

/// Opening the tasks panel closes the Changes dock: one panel at a time.
#[gpui::test]
fn the_tasks_panel_and_the_changes_dock_share_the_slot(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    show(&workspace, &mut visual, with_tasks());
    v2(&workspace, &mut visual, Design::V2);
    open_changes(&workspace, &mut visual);
    assert!(workspace.read_with(&visual, |this, _| this.review_dock_showing()));
    click(&mut visual, "title-tasks");
    workspace.read_with(&visual, |this, _| {
        assert!(this.tasks_showing());
        assert!(!this.review_dock_showing());
    });
}
