//! The Jobs page: a view of the hub's jobs whose only writes are the owner's
//! approve, pause/resume, run and remove.
use super::*;
use wks_native::features::{JobAction, Request};

fn listed() -> serde_json::Value {
    serde_json::json!({"jobs":[
        {"id":"job","name":"Prune","enabled":true,"trigger":{"kind":"interval","everyMinutes":240},
         "action":{"kind":"shell","shell":{"command":"git worktree prune"}},
         "lastRun":{"startedAt":1,"finishedAt":2,"status":"ok"}},
        {"id":"change","name":"Prune","enabled":false,"proposedBy":"helper","replaces":"job",
         "trigger":{"kind":"daily","at":"03:00"},
         "action":{"kind":"shell","shell":{"command":"git worktree prune"}}}
    ]})
}

#[gpui::test]
fn jobs_page_reviews_a_proposed_change_and_only_writes_owner_actions(cx: &mut TestAppContext) {
    reviews_a_proposed_change(cx, wks_native::appearance::Design::V1);
}

/// The v2 worked example keeps every selector and write of the shipped page.
#[gpui::test]
fn jobs_page_under_design_v2_keeps_its_actions(cx: &mut TestAppContext) {
    reviews_a_proposed_change(cx, wks_native::appearance::Design::V2);
}

fn reviews_a_proposed_change(cx: &mut TestAppContext, design: wks_native::appearance::Design) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    // Tall enough that the open proposal's details (Host row included)
    // leave the next row's actions on screen.
    visual.simulate_resize(size(px(1000.), px(900.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(design));
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Jobs, window, cx);
        })
    });
    visual.run_until_parked();
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::Jobs))),
        "opening the page reads the jobs"
    );
    request_state(&workspace, &mut visual, Request::Jobs, 1, listed());
    for selector in ["page-header", "page-title", "feature-back"] {
        assert!(
            visual.debug_bounds(selector).is_some(),
            "{design:?} {selector}"
        );
    }
    if design.is_v2() {
        assert!(visual.debug_bounds("jobs-refresh").is_some());
        // Small controls at the v2 height.
        let pause = visual.debug_bounds("job-pause-1").unwrap();
        assert_eq!(
            f32::from(pause.size.height),
            wks_native::tokens::Tokens::V2.layout.control_height_sm
        );
    }

    // The proposal sorts first and opens to the job it would change (v2:
    // R4.3, the decision's Details).
    if design.is_v2() {
        // The whole command and where it runs, beside Approve.
        assert!(visual.debug_bounds("job-action-0").is_some());
        assert!(visual.debug_bounds("decision-where").is_some());
        click(&mut visual, "job-details-0");
    } else {
        click(&mut visual, "job-row-0");
    }
    assert!(visual.debug_bounds("job-detail").is_some());
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::JobHistory { id }) if id == "change"
    )));
    click(&mut visual, "job-approve-0");
    let approval = effects(&mut commands)
        .into_iter()
        .find_map(|c| match c {
            Command::Request(Request::JobAction(JobAction::Save(spec))) => Some(spec),
            _ => None,
        })
        .expect("approve saves the proposal");
    assert_eq!(approval["id"], "change");
    assert_eq!(approval["replaces"], "job");
    assert_eq!(approval["enabled"], true);
    assert!(approval.get("proposedBy").is_none());

    // Removing a job takes a second click; nothing is sent on the first.
    if design.is_v2() {
        // v2 keeps Remove in the row's overflow menu.
        click(&mut visual, "job-1-more");
    }
    click(&mut visual, "job-remove-1");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::JobAction(_))))
    );
    click(&mut visual, "job-remove-1");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::JobAction(JobAction::Remove(id))) if id == "job"
    )));

    // A write answers with the refreshed list, which the page then shows.
    request_state(
        &workspace,
        &mut visual,
        Request::JobAction(JobAction::Remove("job".into())),
        2,
        serde_json::json!({"jobs":[]}),
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.job_rows(), Some(vec![]));
    });
}

#[gpui::test]
fn runs_tab_lists_every_jobs_runs_and_opens_one_to_its_output(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Jobs, window, cx);
        })
    });
    visual.run_until_parked();
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::JobRuns))),
        "opening the page reads every job's runs"
    );
    request_state(&workspace, &mut visual, Request::Jobs, 1, listed());
    let now = wks_native::timing::now_ms();
    request_state(
        &workspace,
        &mut visual,
        Request::JobRuns,
        2,
        serde_json::json!({"runs":[
            {"jobId":"job","startedAt":now - 60_000,"finishedAt":now - 55_000,
             "status":"error","detail":"shell exited nonzero — output: fatal: not a git repository"},
            {"jobId":"gone","startedAt":now - 120_000,"finishedAt":now - 119_000,"status":"ok"},
            {"jobId":"job","startedAt":now - 86_400_000,"finishedAt":now - 86_399_000,"status":"ok"}
        ]}),
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.job_runs().map(|r| r.len()), Some(3));
    });

    click(&mut visual, "jobs-tab-runs");
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::JobRuns))),
        "the Runs tab reads the runs again"
    );
    workspace.read_with(&visual, |this, _| assert!(this.extras.job_runs_tab));

    // A run opens to its whole output.
    click(&mut visual, "job-run-0");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.extras.job_run_open,
            Some(("job".to_owned(), now - 60_000))
        );
    });
    assert!(visual.debug_bounds("job-run-detail-0").is_some());

    click(&mut visual, "job-runs-failed");
    workspace.read_with(&visual, |this, _| assert!(this.extras.job_failed_only));

    // Show job returns to the job, opened on its own history.
    click(&mut visual, "job-run-job-0");
    workspace.read_with(&visual, |this, _| {
        assert!(!this.extras.job_runs_tab);
        assert_eq!(this.extras.job_expanded.as_deref(), Some("job"));
    });
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::JobHistory { id }) if id == "job"
    )));

    // A hub that cannot list every job's runs leaves the page usable.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                Request::JobRuns.key(),
                wks_native::features::RequestState {
                    request: Request::JobRuns,
                    number: 3,
                    loading: false,
                    error: Some("jobs.history requires {id}".into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    click(&mut visual, "jobs-tab-runs");
    workspace.read_with(&visual, |this, _| assert_eq!(this.job_runs(), None));
}

/// A session a job started says so in the sidebar, and its run in the Runs
/// tab names it and opens it directly.
#[gpui::test]
fn job_sessions_are_marked_and_open_from_their_run(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut view = state("b");
    let mut sessions = (*view.sessions).clone();
    sessions[0].job_id = "job".into();
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(view), window, cx);
            this.open_feature(Screen::Jobs, window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-job-0").is_some(), "job chip");
    assert!(visual.debug_bounds("sidebar-job-1").is_none());
    request_state(&workspace, &mut visual, Request::Jobs, 1, listed());
    let now = wks_native::timing::now_ms();
    request_state(
        &workspace,
        &mut visual,
        Request::JobRuns,
        2,
        serde_json::json!({"runs":[
            {"jobId":"job","startedAt":now - 60_000,"finishedAt":now - 55_000,
             "status":"ok","detail":"spawned a"},
            {"jobId":"job","startedAt":now - 120_000,"finishedAt":now - 119_000,
             "status":"ok","detail":"spawned gone"}
        ]}),
    );
    click(&mut visual, "jobs-tab-runs");
    // A session no longer listed has nothing to open.
    assert!(visual.debug_bounds("job-run-row-session-1").is_none());
    effects(&mut commands);
    click(&mut visual, "job-run-row-session-0");
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Select(id) if id == "a")),
        "Open selects the job's session"
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation);
        // The row's own toggle did not also fire.
        assert_eq!(this.extras.job_run_open, None);
    });
}

/// A hub that cannot list jobs gets a plain explanation and a retry, not a
/// bare error as the page's first line.
#[gpui::test]
fn jobs_that_cannot_load_explain_and_retry(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Jobs, window, cx);
            let mut view = (*this.view).clone();
            view.requests.insert(
                Request::Jobs.key(),
                wks_native::features::RequestState {
                    request: Request::Jobs,
                    number: 1,
                    loading: false,
                    error: Some("Unknown method: jobs.list".into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("jobs-unavailable").is_some());
    effects(&mut commands);
    click(&mut visual, "jobs-retry");
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::Jobs))),
        "Try again reads the jobs again"
    );
    // Once the hub lists them, the rows show. (Test frames keep every
    // selector ever drawn, so the explanation's absence is not checkable.)
    request_state(&workspace, &mut visual, Request::Jobs, 2, listed());
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.job_rows().map(|rows| rows.len()), Some(2))
    });
    assert!(visual.debug_bounds("job-row-0").is_some());
}
