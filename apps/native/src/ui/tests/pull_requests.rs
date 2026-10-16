//! The title's pull request chip and button, and the Pull request page:
//! linking (suggested by branch, or a pasted URL), the review threads, and
//! reply, resolve and "Send to agent".
use super::*;
use serde_json::json;
use wks_native::features::PrAction;

const URL: &str = "https://github.com/o/r/pull/12";

/// Session "a", linked to `URL` when `linked`.
fn session_view(linked: bool) -> View {
    let mut view = state("a");
    let session = &mut Arc::make_mut(&mut view.sessions)[0];
    session.cwd = "/repo".into();
    if linked {
        session.merge(&json!({"compacting": false, "pullRequest": {
            "url": URL, "provider": "github", "repo": "o/r", "number": 12,
            "title": "Link sessions to pull requests", "state": "open",
            "sourceBranch": "feature/pr", "source": "agent", "unresolved": 1}}));
    }
    view
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

fn tap(visual: &mut VisualTestContext, selector: &'static str) {
    click(visual, selector);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

fn requests(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Request> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Request(request) => Some(request),
            _ => None,
        })
        .collect()
}

fn threads() -> serde_json::Value {
    json!({"truncated": false, "unresolved": 1, "threads": [
        {"id": "conversation", "resolvable": false, "resolved": false, "outdated": false,
         "comments": [{"id": "c0", "author": "lead", "body": "Nice start"}]},
        {"id": "T1", "path": "src/lib.rs", "line": 12, "resolvable": true, "resolved": false,
         "outdated": false, "comments": [{"id": "c1", "author": "rev", "body": "Rename this"}]},
        {"id": "T2", "path": "README.md", "line": 3, "resolvable": true, "resolved": true,
         "outdated": false, "comments": [{"id": "c2", "author": "rev", "body": "Typo"}]}
    ]})
}

fn open_page(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_feature(Screen::PullRequest, window, cx)
        })
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

#[gpui::test]
fn a_linked_session_shows_its_pull_request_in_the_title(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    show(&workspace, &mut visual, session_view(true));
    effects(&mut commands);
    assert!(visual.debug_bounds("title-pr").is_some());
    tap(&mut visual, "title-pr");
    assert!(matches!(
        requests(&mut commands).as_slice(),
        [Request::PrThreads { url }] if url == URL
    ));
    assert!(visual.debug_bounds("pr-header").is_some());
    assert!(
        visual.debug_bounds("pr-linker").is_none(),
        "linked: no linker until Change"
    );
}

#[gpui::test]
fn threads_resolve_reply_and_go_to_the_agent(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(1000.)));
    show(&workspace, &mut visual, session_view(true));
    open_page(&workspace, &mut visual);
    effects(&mut commands);
    request_state(
        &workspace,
        &mut visual,
        Request::PrThreads { url: URL.into() },
        50,
        threads(),
    );
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    assert!(visual.debug_bounds("pr-thread-T1").is_some());
    assert!(visual.debug_bounds("pr-thread-conversation").is_some());
    assert!(
        visual.debug_bounds("pr-thread-T2").is_none(),
        "resolved threads start hidden"
    );
    assert!(
        visual
            .debug_bounds("pr-thread-conversation-resolve")
            .is_none(),
        "the conversation cannot be resolved"
    );
    tap(&mut visual, "pr-thread-T1-resolve");
    assert!(matches!(
        requests(&mut commands).as_slice(),
        [Request::PrAction(PrAction::Resolve { url, thread, resolved: true })]
            if url == URL && thread == "T1"
    ));
    tap(&mut visual, "pr-thread-T1-reply");
    visual.simulate_input("Renamed in the next push");
    tap(&mut visual, "pr-thread-T1-reply-send");
    assert!(matches!(
        requests(&mut commands).as_slice(),
        [Request::PrAction(PrAction::Reply { thread, body, .. })]
            if thread == "T1" && body == "Renamed in the next push"
    ));
    tap(&mut visual, "pr-send-unresolved");
    let draft = workspace.read_with(&visual, |this, cx| {
        this.composer.read(cx).value().to_string()
    });
    assert!(draft.contains("GitHub #12"), "{draft}");
    assert!(draft.contains("src/lib.rs:12 (thread T1)"), "{draft}");
    assert!(
        !draft.contains("Typo"),
        "resolved threads stay out: {draft}"
    );
    assert!(
        !draft.contains("Nice start"),
        "the conversation is not unresolved: {draft}"
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation)
    });
}

#[gpui::test]
fn an_unlinked_session_is_offered_its_branch_pull_request(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(900.)));
    show(&workspace, &mut visual, session_view(false));
    assert!(visual.debug_bounds("title-pr").is_none());
    effects(&mut commands);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_feature(Screen::Session, window, cx)
        })
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    tap(&mut visual, "session-pull-request");
    let sent = requests(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Request::PrForBranch { cwd }] if cwd == "/repo"),
        "{sent:?}"
    );
    assert!(visual.debug_bounds("pr-linker").is_some());
    request_state(
        &workspace,
        &mut visual,
        Request::PrForBranch {
            cwd: "/repo".into(),
        },
        60,
        json!({"detected": {"branch": "feature/pr"}, "pullRequest": {
            "url": URL, "provider": "github", "repo": "o/r", "number": 12,
            "title": "Link sessions", "state": "open", "sourceBranch": "feature/pr"}}),
    );
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    tap(&mut visual, "pr-link-suggested");
    let sent = requests(&mut commands);
    assert!(
        matches!(
            sent.as_slice(),
            [Request::PrLink { session, url: Some(url) }, Request::PrThreads { .. }]
                if session == "a" && url == URL
        ),
        "{sent:?}"
    );
}
