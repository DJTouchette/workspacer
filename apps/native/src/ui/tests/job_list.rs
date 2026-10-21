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
            .any(|c| matches!(c, Command::Request(Request::Jobs))),
        "opening the page reads the jobs"
    );
    request_state(&workspace, &mut visual, Request::Jobs, 1, listed());

    // The proposal sorts first and opens to the job it would change.
    click(&mut visual, "job-row-0");
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
