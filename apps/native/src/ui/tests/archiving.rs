//! Archiving: shared through the hub, never a stop.
use super::*;

#[gpui::test]
fn sidebar_archive_hides_session_without_selecting_or_stopping_it(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let path = std::env::temp_dir().join(format!("native-archive-{}.json", std::process::id()));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings_path = Some(path.clone());
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
        });
    });
    visual.run_until_parked();
    let archive = visual.debug_bounds("sidebar-archive-1").unwrap();
    visual.simulate_click(archive.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.visible_sessions(cx), vec![0]);
        assert_eq!(this.view.selected.as_deref(), Some("a"));
        assert_eq!(this.composer.read(cx).value().as_ref(), "keep draft");
    });
    assert!(commands.try_recv().is_err());
    assert_eq!(Settings::load(&path).unwrap().archived["test"], vec!["b"]);
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.toggle_archive("b", cx);
            assert_eq!(this.visible_sessions(cx), vec![0, 1]);
        })
    });
    let _ = std::fs::remove_file(path);
}

fn archive_doc(version: i64, ids: &[&str]) -> Option<Arc<serde_json::Value>> {
    let archived: serde_json::Map<_, _> = ids
        .iter()
        .map(|id| (id.to_string(), serde_json::json!(1)))
        .collect();
    Some(Arc::new(
        serde_json::json!({"version":version,"archived":archived}),
    ))
}

#[gpui::test]
fn sidebar_archive_is_shared_through_the_hub_and_never_stops(cx: &mut TestAppContext) {
    use wks_native::{controller::ArchiveReceipt, features::Request};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let path =
        std::env::temp_dir().join(format!("native-shared-archive-{}.json", std::process::id()));
    let with = |version, ids: &[&str], receipts: Vec<ArchiveReceipt>| {
        let mut view = state("a");
        view.session_archive = archive_doc(version, ids);
        view.archive_receipts = receipts.into();
        Arc::new(view)
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings_path = Some(path.clone());
            this.update_view(with(1, &[], vec![]), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
        });
    });
    visual.run_until_parked();
    let _ = archive_effects(&mut commands);
    let archive = visual.debug_bounds("sidebar-archive-1").unwrap();
    visual.simulate_click(archive.center(), gpui::Modifiers::default());
    // Hidden at once, selection and draft untouched, and the only effect
    // is the hub archive request: no stop, select, or local-only copy.
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.visible_sessions(cx), vec![0]);
        assert_eq!(this.view.selected.as_deref(), Some("a"));
        assert_eq!(this.composer.read(cx).value().as_ref(), "keep draft");
        assert!(this.settings.archived.get("test").is_none_or(Vec::is_empty));
    });
    let sent = archive_effects(&mut commands);
    assert!(
        matches!(&sent[..], [Command::Request(Request::SetArchive { session, archived: true })] if session == "b"),
        "unexpected effects: {}",
        sent.len()
    );
    let ok = |number, archived| ArchiveReceipt {
        number,
        session: "b".into(),
        archived,
        error: None,
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            // The hub confirms; then the web restores it.
            this.update_view(with(2, &["b"], vec![ok(1, true)]), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![0]);
            assert!(this.extras.archive_pending.is_empty());
            this.update_view(with(3, &[], vec![ok(1, true)]), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![0, 1]);
            // A refused archive comes back into the list and says why.
            this.toggle_archive("b", cx);
            assert_eq!(this.visible_sessions(cx), vec![0]);
            let mut failed = ok(2, true);
            failed.error = Some("denied".into());
            this.update_view(with(3, &[], vec![ok(1, true), failed]), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![0, 1]);
            assert!(this.extras.notice.contains("denied"));
        })
    });
    assert!(
        archive_effects(&mut commands)
            .iter()
            .all(|c| matches!(c, Command::Request(Request::SetArchive { .. })))
    );
    let _ = std::fs::remove_file(path);
}

#[gpui::test]
fn device_archives_move_to_the_hub_once_and_stay_hidden(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let path = std::env::temp_dir().join(format!(
        "native-archive-migrate-{}.json",
        std::process::id()
    ));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings_path = Some(path.clone());
            this.settings
                .archived
                .insert("test".into(), vec!["a".into()]);
            let mut view = state("b");
            view.session_archive = archive_doc(1, &[]);
            this.update_view(Arc::new(view.clone()), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![1]);
            // A second update before the hub answers does not resend.
            this.update_view(Arc::new(view), window, cx);
        })
    });
    let sent = archive_effects(&mut commands);
    assert!(
        matches!(&sent[..], [Command::Request(Request::SetArchive { session, archived: true })] if session == "a"),
        "unexpected effects: {}",
        sent.len()
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("b");
            view.session_archive = archive_doc(2, &["a"]);
            this.update_view(Arc::new(view), window, cx);
            // Now the hub holds it: the device copy is gone, still hidden.
            assert!(!this.settings.archived.contains_key("test"));
            assert_eq!(this.visible_sessions(cx), vec![1]);
        })
    });
    assert!(Settings::load(&path).unwrap().archived.is_empty());
    let _ = std::fs::remove_file(path);
}

#[gpui::test]
fn archive_restore_during_migration_serializes_writes_and_preserves_the_latest_click(
    cx: &mut TestAppContext,
) {
    use wks_native::controller::ArchiveReceipt;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings_path = None;
            this.settings
                .archived
                .insert("test".into(), vec!["a".into()]);
            let mut next = state("a");
            next.session_archive = archive_doc(1, &[]);
            this.update_view(Arc::new(next), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("retain draft", window, cx));
            this.toggle_archive("a", cx); // restore while migration write is in flight
            assert!(!this.archived("a"));
        })
    });
    let first = archive_effects(&mut commands);
    assert!(
        matches!(&first[..], [Command::Request(Request::SetArchive { session, archived: true })] if session == "a")
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.session_archive = archive_doc(2, &["a"]);
            next.archive_receipts.push_back(ArchiveReceipt {
                number: 1,
                session: "a".into(),
                archived: true,
                error: None,
            });
            this.update_view(Arc::new(next), window, cx);
            assert!(!this.archived("a"));
            this.toggle_archive("a", cx); // archive then restore while restore is in flight
            this.toggle_archive("a", cx);
            assert!(!this.archived("a"));
        })
    });
    let second = archive_effects(&mut commands);
    assert!(
        matches!(&second[..], [Command::Request(Request::SetArchive { session, archived: false })] if session == "a")
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.session_archive = archive_doc(3, &[]);
            next.archive_receipts.push_back(ArchiveReceipt {
                number: 2,
                session: "a".into(),
                archived: false,
                error: None,
            });
            this.update_view(Arc::new(next), window, cx);
            assert!(!this.archived("a"));
            assert!(this.extras.archive_pending.is_empty());
            assert_eq!(this.composer.read(cx).value().as_str(), "retain draft");
            assert_eq!(this.view.selected.as_deref(), Some("a"));
        })
    });
    assert!(archive_effects(&mut commands).is_empty());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.toggle_archive("a", cx);
            let mut offline = state("a");
            offline.connected = false;
            offline.session_archive = archive_doc(3, &[]);
            this.update_view(Arc::new(offline), window, cx);
            assert!(this.extras.archive_pending.is_empty());
            assert!(this.extras.archive_migrating.is_empty());
        })
    });
    let sent = archive_effects(&mut commands);
    assert!(matches!(
        &sent[..],
        [Command::Request(Request::SetArchive { archived: true, .. })]
    ));
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.toggle_archive("a", cx);
            assert!(this.extras.notice.contains("Reconnect"));
            assert_eq!(this.composer.read(cx).value().as_str(), "retain draft");
        })
    });
    assert!(archive_effects(&mut commands).is_empty());
}

#[gpui::test]
fn archive_first_read_hides_rows_until_visibility_is_known(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut next = state("a");
            next.requests.insert(
                "archive",
                wks_native::features::RequestState {
                    number: 1,
                    request: Request::Archive,
                    loading: true,
                    value: Arc::new(serde_json::Value::Null),
                    error: None,
                },
            );
            this.update_view(Arc::new(next.clone()), window, cx);
            assert!(this.visible_sessions(cx).is_empty());
            next.session_archive = archive_doc(1, &["a"]);
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.visible_sessions(cx), vec![1]);
        })
    });
}

#[gpui::test]
fn archived_sessions_are_filtered_per_connection(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.settings
                .archived
                .insert("test".into(), vec!["a".into()]);
            assert_eq!(this.visible_sessions(cx), vec![1]);
            this.project_scope = "another-hub".into();
            assert_eq!(this.visible_sessions(cx), vec![0, 1]);
        })
    });
}
