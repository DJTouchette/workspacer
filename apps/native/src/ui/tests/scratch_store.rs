//! The Scratch page: a project's personal files outside git, whose only
//! writes are metadata marks (status, keep, purpose, note). It never runs a
//! file and never writes one's contents.
use super::*;
use wks_native::features::Request;

const CWD: &str = "/work/shop/worktrees/fix-cart";

fn listed() -> serde_json::Value {
    serde_json::json!({
        "project": {"root":"/work/shop","key":"shop-0011223344556677","files":"/h/.workspacer/scratch/shop-0011223344556677/files","exists":true},
        "items": [
            {"path":"old.sql","size":10,"modifiedAt":"2026-10-01T00:00:00Z","indexed":true,"purpose":"old backfill","status":"done"},
            {"path":"repro.sh","size":2048,"modifiedAt":"2026-11-01T10:00:00Z","indexed":false,"purpose":null,"status":null},
            {"path":"prod/backfill.sql","size":300,"modifiedAt":"2026-10-30T00:00:00Z","indexed":true,"purpose":"backfill orders","status":"pending","keep":true,"sensitive":true,"sessionId":"a"}
        ],
        "truncated": false
    })
}

fn with_cwd(id: &str) -> View {
    let mut view = state(id);
    let mut sessions = (*view.sessions).clone();
    sessions[0].cwd = CWD.into();
    sessions[1].cwd = "/work/other".into();
    view.sessions = Arc::new(sessions);
    view
}

fn open_page(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    // Tall enough that an open file's editor and actions sit on screen.
    visual.simulate_resize(size(px(1000.), px(1400.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(with_cwd("a")), window, cx);
            this.open_feature(Screen::Scratch, window, cx);
        })
    });
    visual.run_until_parked();
}

fn marks(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<(String, serde_json::Value)> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Request(Request::ScratchMark { cwd, path, patch }) => {
                assert_eq!(cwd, CWD, "marks name the folder the page was opened on");
                Some((path, patch))
            }
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn scratch_opens_on_the_selected_agents_project_with_pending_files_first(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_page(&workspace, &mut visual);
    assert!(
        effects(&mut commands).iter().any(|c| matches!(
            c,
            Command::Request(Request::ScratchList { cwd }) if cwd == CWD
        )),
        "opening the page lists the selected agent's project"
    );
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchList { cwd: CWD.into() },
        1,
        listed(),
    );
    assert!(visual.debug_bounds("scratch-pending").is_some());
    workspace.read_with(&visual, |this, _| {
        let listing = this.scratch_listing().unwrap();
        assert_eq!(listing.root, "/work/shop");
        assert_eq!(listing.items[0].path, "prod/backfill.sql");
    });
    // Open: the pending file and the unrecorded one; the done file is hidden.
    assert!(visual.debug_bounds("scratch-row-0").is_some());
    assert!(visual.debug_bounds("scratch-row-1").is_some());
    assert!(visual.debug_bounds("scratch-row-2").is_none());

    // Selecting another agent does not swap the list or leave the page.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(with_cwd("b")), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Scratch);
        assert_eq!(this.extras.scratch_cwd.as_deref(), Some(CWD));
    });
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::ScratchList { .. }))),
        "a selection change sends no new list"
    );
}

#[gpui::test]
fn opening_a_file_reads_it_and_marks_send_only_metadata(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_page(&workspace, &mut visual);
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchList { cwd: CWD.into() },
        1,
        listed(),
    );
    effects(&mut commands);

    click(&mut visual, "scratch-row-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::ScratchRead { cwd, path }) if cwd == CWD && path == "prod/backfill.sql"
    )));
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchRead {
            cwd: CWD.into(),
            path: "prod/backfill.sql".into(),
        },
        2,
        serde_json::json!({"path":"prod/backfill.sql","content":"UPDATE orders SET x = 1;"}),
    );
    assert!(visual.debug_bounds("scratch-content").is_some());

    // A pending file's primary action records that it ran.
    click(&mut visual, "scratch-mark-done-0");
    assert_eq!(
        marks(&mut commands),
        vec![(
            "prod/backfill.sql".to_owned(),
            serde_json::json!({"status":"done"})
        )]
    );
    click(&mut visual, "scratch-keep-0");
    assert_eq!(
        marks(&mut commands),
        vec![(
            "prod/backfill.sql".to_owned(),
            serde_json::json!({"keep":false})
        )]
    );

    // The editor opened with the file's purpose: an untouched Save sends
    // nothing; a note sends only the note.
    click(&mut visual, "scratch-save-0");
    assert!(marks(&mut commands).is_empty());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.scratch_note.update(cx, |input, cx| {
                input.set_value("ran on prod 11/1", window, cx)
            });
        })
    });
    visual.run_until_parked();
    click(&mut visual, "scratch-save-0");
    assert_eq!(
        marks(&mut commands),
        vec![(
            "prod/backfill.sql".to_owned(),
            serde_json::json!({"note":"ran on prod 11/1"})
        )]
    );

    // A mark answers with the refreshed list, which the page then shows:
    // the file is done, so Open no longer lists it. (Asserted on the model:
    // the test window keeps an element's debug bounds after it goes away.)
    let mut after = listed();
    after["items"][2]["status"] = "done".into();
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchMark {
            cwd: CWD.into(),
            path: "prod/backfill.sql".into(),
            patch: serde_json::json!({"status":"done"}),
        },
        3,
        after,
    );
    workspace.read_with(&visual, |this, _| {
        let listing = this.scratch_listing().unwrap();
        assert_eq!(listing.pending(), 0);
        let open: Vec<_> = listing
            .items
            .iter()
            .filter(|i| this.extras.scratch_filter.admits(i))
            .map(|i| i.path.as_str())
            .collect();
        assert_eq!(open, ["repro.sh"]);
    });
}

#[gpui::test]
fn sensitive_content_the_hub_withheld_says_so(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_page(&workspace, &mut visual);
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchList { cwd: CWD.into() },
        1,
        listed(),
    );
    click(&mut visual, "scratch-row-0");
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchRead {
            cwd: CWD.into(),
            path: "prod/backfill.sql".into(),
        },
        2,
        serde_json::json!({"path":"prod/backfill.sql","content":null,"withheld":"sensitive scratch content is not served to federated callers"}),
    );
    assert!(visual.debug_bounds("scratch-withheld").is_some());
    assert!(visual.debug_bounds("scratch-content").is_none());
}

#[gpui::test]
fn an_empty_scratch_explains_how_files_get_there(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_page(&workspace, &mut visual);
    request_state(
        &workspace,
        &mut visual,
        Request::ScratchList { cwd: CWD.into() },
        1,
        serde_json::json!({"project":{"root":"/work/shop","key":"k","files":"/h/f","exists":false},"items":[],"truncated":false}),
    );
    assert!(visual.debug_bounds("scratch-empty").is_some());
    assert!(visual.debug_bounds("scratch-pending").is_none());
}

#[gpui::test]
fn the_sidebar_opens_scratch_on_the_selected_agents_project(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(with_cwd("a")), window, cx);
        })
    });
    visual.run_until_parked();
    effects(&mut commands);
    click(&mut visual, "nav-scratch");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Scratch));
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::ScratchList { cwd }) if cwd == CWD
    )));
}
