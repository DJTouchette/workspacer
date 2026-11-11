//! Projects as named groups of folders (docs/features/projects.md): the
//! Projects page's cards and editor, and the sidebar filter for a whole
//! project.
use super::*;
use wks_native::features::Request;

fn groups(revision: u64) -> serde_json::Value {
    serde_json::json!({"revision": revision, "groups": [
        {"id":"p_acme","name":"Acme","folders":["/w/acme-api","/w/acme-web"],"icon":"🚀"}
    ]})
}

fn sessions() -> View {
    let session = |id: &str, cwd: &str, parent: &str| Session {
        id: id.into(),
        label: id.into(),
        cwd: cwd.into(),
        parent_session_id: parent.into(),
        state: "input".into(),
        ..Default::default()
    };
    View {
        connected: true,
        selected: Some("api".into()),
        sessions: Arc::new(vec![
            session("api", "/w/acme-api", ""),
            session("web", "/w/acme-web/src", ""),
            session("worker", "/h/.workspacer/worktrees/acme-api/fix", "api"),
            session("other", "/w/other", ""),
        ]),
        ..Default::default()
    }
}

fn open_projects(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    commands: &mut tokio::sync::mpsc::Receiver<Command>,
) {
    // Landscape: a tall window collapses the sidebar to its rail.
    visual.simulate_resize(size(px(1500.), px(1200.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(sessions()), window, cx);
            this.show_screen(Screen::Projects, window, cx);
        })
    });
    visual.run_until_parked();
    let sent = effects(commands);
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::Request(Request::ProjectGroups))),
        "Projects reads the groups with the registry"
    );
    request_state(workspace, visual, Request::ProjectGroups, 1, groups(1));
}

#[gpui::test]
fn projects_page_shows_projects_with_sessions_and_tags_their_folders(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_projects(&workspace, &mut visual, &mut commands);
    assert!(visual.debug_bounds("project-group-0").is_some());
    workspace.read_with(&visual, |this, _| {
        let groups = this.project_groups();
        assert_eq!(groups.revision, 1);
        // The worker in the hub's worktree belongs to Acme through its repo.
        let root = this.worktree_root().to_owned();
        let of = |ix| {
            groups
                .of_session(&this.view.sessions, ix, &root)
                .map(|g| g.name.clone())
        };
        assert_eq!(of(1).as_deref(), Some("Acme"));
        assert_eq!(of(2).as_deref(), Some("Acme"));
        assert_eq!(of(3), None);
    });
}

#[gpui::test]
fn a_new_project_is_named_given_folders_and_saved_with_the_revision(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_projects(&workspace, &mut visual, &mut commands);
    click(&mut visual, "new-project-group");
    assert!(visual.debug_bounds("project-group-editor").is_some());

    // Nothing is sent while the draft is not a project yet.
    click(&mut visual, "project-group-save");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::SaveProjectGroup { .. })))
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.group_error.contains("name"));
    });

    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .group_name
                .update(cx, |input, cx| input.set_value("Side", window, cx));
            this.extras
                .group_folder
                .update(cx, |input, cx| input.set_value("/w/side-a/", window, cx));
        })
    });
    visual.run_until_parked();
    click(&mut visual, "group-folder-add");
    // A folder only a session knows (and not in Acme) is offered in one click.
    click(&mut visual, "group-folder-suggest-0");
    workspace.read_with(&visual, |this, _| {
        let draft = this.extras.group_draft.clone().unwrap();
        assert_eq!(draft.folders, ["/w/side-a", "/w/other"]);
    });
    click(&mut visual, "project-group-save");
    let params = effects(&mut commands)
        .into_iter()
        .find_map(|c| match c {
            Command::Request(Request::SaveProjectGroup { params }) => Some(params),
            _ => None,
        })
        .expect("Save sends the project");
    assert_eq!(
        params,
        serde_json::json!({"name":"Side","folders":["/w/side-a","/w/other"],"revision":1})
    );

    // The hub's answer closes the editor and says so.
    let mut saved = groups(2);
    saved["saved"] = "p_side".into();
    request_state(
        &workspace,
        &mut visual,
        Request::SaveProjectGroup { params },
        5,
        saved,
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.group_draft.is_none());
        assert_eq!(this.projects.notice, "Project saved.");
        assert_eq!(this.project_groups().revision, 2);
    });
}

#[gpui::test]
fn a_refused_save_keeps_the_editor_open_and_deleting_asks_twice(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_projects(&workspace, &mut visual, &mut commands);
    click(&mut visual, "project-group-edit-0");
    click(&mut visual, "project-group-save");
    let params = effects(&mut commands)
        .into_iter()
        .find_map(|c| match c {
            Command::Request(Request::SaveProjectGroup { params }) => Some(params),
            _ => None,
        })
        .unwrap();
    assert_eq!(params["id"], "p_acme");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                "project-group-save",
                wks_native::features::RequestState {
                    request: Request::SaveProjectGroup { params },
                    number: 6,
                    loading: false,
                    error: Some("conflict: revision 1 is not 3".into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.extras.group_draft.is_some(),
            "a refused save keeps the draft"
        );
        assert!(this.extras.group_error.contains("changed elsewhere"));
    });

    click(&mut visual, "project-group-delete");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::RemoveProjectGroup { .. })))
    );
    click(&mut visual, "project-group-delete");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::RemoveProjectGroup { id, revision: 1 }) if id == "p_acme"
    )));
}

#[gpui::test]
fn filtering_the_sidebar_to_a_project_shows_every_folder_and_its_workers(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_projects(&workspace, &mut visual, &mut commands);
    click(&mut visual, "project-group-0");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.project_filter.as_deref(), Some("group:p_acme"));
        let visible: Vec<_> = this
            .visible_sessions(cx)
            .into_iter()
            .map(|ix| this.view.sessions[ix].id.clone())
            .collect();
        assert!(visible.contains(&"api".to_owned()));
        assert!(visible.contains(&"web".to_owned()));
        assert!(visible.contains(&"worker".to_owned()));
        assert!(!visible.contains(&"other".to_owned()));
        assert_eq!(this.filter_folder().as_deref(), Some("/w/acme-api"));
    });
    assert!(visual.debug_bounds("sidebar-project-filter").is_some());
}

#[gpui::test]
fn the_collapsed_rail_shows_the_filtered_project_and_clears_it(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_projects(&workspace, &mut visual, &mut commands);
    click(&mut visual, "project-group-0");
    visual.simulate_resize(size(px(900.), px(1300.)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
    click(&mut visual, "rail-project-filter");
    workspace.read_with(&visual, |this, _| assert_eq!(this.project_filter, None));
}
