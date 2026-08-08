//! Integration tests for [`super::App`]. These drive the real struct through
//! whole flows — open an agent, dock a pane, send and retire an echo — so they
//! deliberately cut across the per-concern modules rather than sitting inside
//! one of them.

use super::*;

/// Redirect the config dir away from the real ~/.config/workspacer (pins /
/// names / notes) so tests neither read nor write it. It used to be a pid-named
/// directory under /tmp that nothing ever removed — see crate::testenv.
fn isolate_config() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| crate::testenv::isolate_config_home("app"));
}

/// A docked pane takes the keys when it opens and hands them back on Ctrl-w,
/// so the conversation stays readable beside it either way.
#[tokio::test]
async fn a_docked_pane_takes_focus_and_gives_it_back() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();

    app.toggle_side(SideKind::Changes);
    assert!(app.side_focused(), "opening it focuses it");
    assert!(app.side_of(SideKind::Changes).is_some());

    assert!(app.focus_side(false));
    assert!(!app.side_focused(), "keys go back to the chat");
    assert!(
        app.side_of(SideKind::Changes).is_some(),
        "…but the pane stays docked"
    );

    // From the chat, stepping through the panes lands on it again.
    app.focus_pane(1);
    assert!(app.side_focused());
}

#[tokio::test]
async fn toggling_the_same_pane_closes_it() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();

    app.toggle_side(SideKind::Changes);
    app.toggle_side(SideKind::Changes);
    assert!(app.side.is_none());
}

/// Two kinds, one dock: asking for the other swaps it rather than stacking.
#[tokio::test]
async fn asking_for_the_other_kind_swaps_the_dock() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();

    app.toggle_side(SideKind::Changes);
    app.toggle_side(SideKind::Review);
    assert_eq!(app.side.as_ref().map(|p| p.kind), Some(SideKind::Review));
    assert!(app.side_of(SideKind::Changes).is_none());
}

/// Closing the review has to take its dock with it — a docked pane with no
/// review behind it would render an empty box you can't close.
#[tokio::test]
async fn closing_the_review_undocks_it() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();

    app.open_review();
    assert_eq!(app.side.as_ref().map(|p| p.kind), Some(SideKind::Review));
    app.close_review();
    assert!(app.side.is_none());
    assert!(app.review.is_none());
}

/// Resuming keeps the row's own transport; only a fresh spawn takes the
/// configured one. Flipping a live session's transport would take away the
/// terminal a PTY session has (or promise one a stream session never had).
#[test]
fn respawn_inherits_the_existing_rows_transport() {
    let mut app = test_app();
    app.transport = crate::config::Transport::Stream;
    // Built through serde like the rest of these tests, so the wire shape
    // (and `transport`'s pty default) is what's under test.
    app.all_agents = vec![
        serde_json::from_value(serde_json::json!({
            "session_id": "pty-row", "mode": "input", "transport": "pty"
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "session_id": "stream-row", "mode": "input", "transport": "stream"
        }))
        .unwrap(),
    ];

    assert_eq!(
        app.respawn_transport("pty-row"),
        crate::config::Transport::Pty
    );
    assert_eq!(
        app.respawn_transport("stream-row"),
        crate::config::Transport::Stream
    );
    // A row we've never seen has no transport to inherit — the config's
    // choice is the best guess there is.
    assert_eq!(
        app.respawn_transport("unknown"),
        crate::config::Transport::Stream
    );
    app.transport = crate::config::Transport::Pty;
    assert_eq!(
        app.respawn_transport("unknown"),
        crate::config::Transport::Pty
    );
}

fn test_app() -> App {
    isolate_config();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
    // Points at an unused port; the background stream tasks just fail and
    // retry harmlessly, which is fine for exercising app state.
    let cm = Claudemon::new("http://127.0.0.1:59999".into());
    App::new(cm, Vec::new(), Vec::new(), Config::default(), tx, ptx)
}

/// A conversation snapshot in claudemon's wire shape, for the fold to adopt.
fn snapshot(user_texts: &[&str]) -> Box<serde_json::Value> {
    let items: Vec<serde_json::Value> = user_texts
        .iter()
        .map(|t| serde_json::json!({ "kind": "user_message", "text": t }))
        .collect();
    Box::new(serde_json::json!({ "items": items }))
}

fn agent(id: &str) -> Agent {
    serde_json::from_value(serde_json::json!({ "session_id": id, "mode": "responding" })).unwrap()
}

fn agent_cwd(id: &str, cwd: &str, mode: &str) -> Agent {
    serde_json::from_value(serde_json::json!({ "session_id": id, "cwd": cwd, "mode": mode }))
        .unwrap()
}

#[test]
fn bus_statusline_event_applies_status_line() {
    let mut app = test_app();
    app.apply_bus_event(crate::bus::BusEvent {
        topic: "agent.statusline".into(),
        data: serde_json::json!({
            "sessionId": "s1",
            "statusLine": { "text": "building…" },
        }),
    });
    assert!(app.status_lines.contains_key("s1"));
}

#[test]
fn pty_bytes_event_feeds_the_terminal() {
    let mut app = test_app();
    app.terms.insert("s1".into(), crate::terminal::Term::new());
    app.apply_bus_event(crate::bus::BusEvent {
        topic: "pty.bytes.s1".into(),
        data: serde_json::json!(base64::engine::general_purpose::STANDARD.encode("hello")),
    });
    let screen = app.terms.get("s1").unwrap().screen();
    assert!(
        screen.contents().contains("hello"),
        "got {:?}",
        screen.contents()
    );
}

#[tokio::test]
async fn review_opens_for_selected_agent_and_closes() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 1;
    app.open_review();
    assert_eq!(app.review.as_ref().map(|r| r.cwd.as_str()), Some("/repo"));
    app.close_review();
    assert!(app.review.is_none());
}

#[tokio::test]
async fn open_review_on_dashboard_row_is_noop() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 0; // Dashboard row — no agent
    app.open_review();
    assert!(app.review.is_none());
    assert_eq!(app.toast(), Some("no working directory for this agent"));
}

#[tokio::test]
async fn respawn_refuses_a_running_agent() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 1;
    app.respawn();
    assert_eq!(app.toast(), Some("agent is still running"));
}

#[test]
fn git_error_surfaces_in_open_review() {
    let mut app = test_app();
    app.review = Some(ReviewState::new("/repo".into()));
    app.apply_msg(AppMsg::GitError {
        cwd: "/repo".into(),
        message: "cwd is not inside a git work tree".into(),
    });
    assert_eq!(
        app.review.as_ref().and_then(|r| r.error.as_deref()),
        Some("cwd is not inside a git work tree")
    );
    // A successful status clears the error.
    app.apply_msg(AppMsg::GitStatus {
        cwd: "/repo".into(),
        branch: Some("main".into()),
        files: vec![],
    });
    assert!(app.review.as_ref().unwrap().error.is_none());
}

#[tokio::test]
async fn review_selection_toggle_and_scroll_reset_view_state() {
    let mut app = test_app();
    app.review = Some(ReviewState::new("/repo".into()));
    {
        let r = app.review.as_mut().unwrap();
        r.files = vec![
            FileStatus {
                path: "a.rs".into(),
                orig_path: None,
                staged: String::new(),
                unstaged: "M".into(),
            },
            FileStatus {
                path: "b.rs".into(),
                orig_path: None,
                staged: "M".into(),
                unstaged: String::new(),
            },
        ];
        r.diff = "old diff".into();
        r.diff_scroll = 7;
    }

    app.review_select(1);
    {
        let r = app.review.as_ref().unwrap();
        assert_eq!(r.selected, 1);
        assert!(r.diff.is_empty());
        assert_eq!(r.diff_scroll, 0);
    }

    app.review_scroll(5);
    assert_eq!(app.review.as_ref().unwrap().diff_scroll, 5);
    app.review_scroll(-3);
    assert_eq!(app.review.as_ref().unwrap().diff_scroll, 2);

    app.review_toggle_staged();
    let r = app.review.as_ref().unwrap();
    assert!(r.staged_view);
    assert!(r.diff.is_empty());
    assert_eq!(r.diff_scroll, 0);
}

#[test]
fn status_line_applied_and_pruned_with_session() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1")]);
    app.apply_status_line(
        "s1".into(),
        StatusLine {
            context_used_pct: Some(50.0),
            ..Default::default()
        },
    );
    assert!(app.status_lines.contains_key("s1"));
    app.set_agents(vec![]); // session gone → statusline pruned
    assert!(!app.status_lines.contains_key("s1"));
}

#[tokio::test]
async fn notes_open_loads_existing_text() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 1;
    app.notes.insert("/repo".into(), "remember this".into());
    app.open_notes();
    assert_eq!(
        app.notes_view.as_ref().map(|n| n.text.as_str()),
        Some("remember this")
    );
}

#[tokio::test]
async fn terminal_stays_warm_across_close_and_prunes_when_gone() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1")]);
    app.selected = 1; // row 0 is the Dashboard; the agent is row 1

    app.open_agent();
    assert!(
        app.terms.contains_key("s1"),
        "opening creates a warm terminal"
    );
    assert!(
        app.workspaces.contains_key("s1"),
        "opening creates a workspace"
    );

    app.close_chat();
    assert!(
        app.terms.contains_key("s1"),
        "terminal stays warm after leaving the pane (so re-open is instant)"
    );

    // Agent disappears from the list → its terminal is pruned.
    app.set_agents(vec![]);
    assert!(
        !app.terms.contains_key("s1"),
        "terminal pruned once the session is gone"
    );
}

#[tokio::test]
async fn shell_tabs_add_switch_and_close() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1")]);
    app.selected = 1;
    app.open_agent();
    assert_eq!(
        app.workspace().unwrap().tabs.len(),
        1,
        "starts with the claude tab"
    );

    app.add_shell_tab("s1".into(), "sh-1".into());
    let ws = app.workspace().unwrap();
    assert_eq!(ws.tabs.len(), 2);
    assert_eq!(ws.active, 1, "switches to the new shell tab");
    assert_eq!(app.chat_session_id().as_deref(), Some("sh-1"));

    app.tab_prev();
    assert_eq!(
        app.chat_session_id().as_deref(),
        Some("s1"),
        "back to claude tab"
    );
    app.tab_next();
    assert_eq!(app.chat_session_id().as_deref(), Some("sh-1"));

    app.close_tab(); // closes the active shell tab
    assert_eq!(app.workspace().unwrap().tabs.len(), 1);
    assert_eq!(app.chat_session_id().as_deref(), Some("s1"));
}

#[tokio::test]
async fn splits_tile_focus_and_collapse() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1"), agent("s2"), agent("s3")]);
    app.selected = 1; // first agent (s1)
    app.open_agent();
    assert_eq!(app.tiles, vec!["s1".to_string()]);
    assert_eq!(app.tile_focus, 0);

    // Split brings the next untiled agent into a new pane and focuses it.
    app.split_pane(SplitDir::Columns);
    assert_eq!(app.tiles, vec!["s1".to_string(), "s2".to_string()]);
    assert_eq!(app.tile_focus, 1);
    assert_eq!(app.open_agent_id(), Some("s2"));

    // Focus wraps around the tiles.
    app.focus_pane(1);
    assert_eq!(app.tile_focus, 0);
    assert_eq!(app.open_agent_id(), Some("s1"));

    // A third split, then close the focused pane.
    app.focus_pane(-1); // back to s2
    app.split_pane(SplitDir::Rows);
    assert_eq!(
        app.tiles,
        vec!["s1".to_string(), "s2".to_string(), "s3".to_string()]
    );
    assert_eq!(app.tile_focus, 2);
    app.close_pane();
    assert_eq!(app.tiles, vec!["s1".to_string(), "s2".to_string()]);

    // only_pane keeps just the focused tile; the last close leaves the view.
    app.only_pane();
    assert_eq!(app.tiles.len(), 1);
    app.close_pane();
    assert_eq!(app.view, View::List);
    assert!(app.tiles.is_empty());
}

#[tokio::test]
async fn split_with_no_other_agent_toasts() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1")]);
    app.selected = 1;
    app.open_agent();
    app.split_pane(SplitDir::Columns);
    assert_eq!(app.tiles.len(), 1);
    assert_eq!(app.toast(), Some("no other agent to split"));
}

#[tokio::test]
async fn tiles_prune_when_a_session_vanishes() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1"), agent("s2")]);
    app.selected = 1;
    app.open_agent();
    app.split_pane(SplitDir::Columns); // tiles = [s1, s2], focus s2
    assert_eq!(app.tiles.len(), 2);
    // s2 goes away — its tile drops and focus stays in range.
    app.set_agents(vec![agent("s1")]);
    assert_eq!(app.tiles, vec!["s1".to_string()]);
    assert!(app.tile_focus < app.tiles.len());
}

#[tokio::test]
async fn harpoon_pin_jump_and_alternate() {
    let mut app = test_app();
    // Pins are keyed by cwd, so the agents need distinct working dirs.
    app.set_agents(vec![
        agent_cwd("s1", "/a", "responding"),
        agent_cwd("s2", "/b", "responding"),
        agent_cwd("s3", "/c", "responding"),
    ]);
    app.selected = 1;
    app.open_agent(); // s1
    app.harpoon_toggle();
    app.selected = 2;
    app.open_agent(); // s2
    app.harpoon_toggle();
    assert_eq!(app.harpoon, vec!["s1".to_string(), "s2".to_string()]);
    assert_eq!(app.pinned_cwds, vec!["/a".to_string(), "/b".to_string()]);

    // Teleport to slot 1, then the alternate-agent toggles back and forth.
    app.harpoon_jump(1);
    assert_eq!(app.open_agent_id(), Some("s1"));
    app.alt_agent();
    assert_eq!(app.open_agent_id(), Some("s2"));
    app.alt_agent();
    assert_eq!(app.open_agent_id(), Some("s1"));

    // Unpin the focused agent; an empty slot just toasts.
    app.harpoon_toggle();
    assert_eq!(app.harpoon, vec!["s2".to_string()]);
    app.harpoon_jump(5);
    assert_eq!(app.toast(), Some("no agent pinned at 5"));
}

#[tokio::test]
async fn pins_restore_by_cwd_across_sessions() {
    let mut app = test_app();
    // Simulate pins loaded from disk on startup (ordered cwds).
    app.pinned_cwds = vec!["/work/alpha".into(), "/work/beta".into()];

    // Nothing live yet → no resolvable pins.
    app.set_agents(vec![]);
    assert!(app.harpoon.is_empty());

    // Agents reappear in those cwds with brand-new session ids — the pins
    // resolve to them, in pin order (not agent order).
    app.set_agents(vec![
        agent_cwd("new-beta", "/work/beta", "responding"),
        agent_cwd("new-alpha", "/work/alpha", "responding"),
    ]);
    assert_eq!(
        app.harpoon,
        vec!["new-alpha".to_string(), "new-beta".to_string()]
    );
}

#[tokio::test]
async fn hides_hydrated_stopped_orphans() {
    let mut app = test_app();
    // A live agent plus two stopped orphans hydrated from history.
    app.set_agents(vec![
        agent_cwd("s1", "/repo", "responding"),
        agent_cwd("old1", "/a", "stopped"),
        agent_cwd("old2", "/b", "stopped"),
    ]);
    assert_eq!(app.agents.len(), 1);
    assert_eq!(app.agents[0].session_id, "s1");
    assert_eq!(app.hidden_count, 2);

    // An agent we watched live stays visible once it stops (respawnable).
    app.set_agents(vec![agent_cwd("s1", "/repo", "stopped")]);
    assert_eq!(
        app.agents.len(),
        1,
        "s1 was seen live, so it survives stopping"
    );
    assert_eq!(app.hidden_count, 0);

    // Toggling show-all reveals stopped history again.
    app.show_all_sessions = true;
    app.set_agents(vec![
        agent_cwd("s1", "/repo", "stopped"),
        agent_cwd("old1", "/a", "stopped"),
    ]);
    assert_eq!(app.agents.len(), 2);
    assert_eq!(app.hidden_count, 0);
}

#[tokio::test]
async fn sidebar_filter_narrows_and_preserves_the_full_set() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/work/alpha", "responding"),
        agent_cwd("s2", "/work/beta", "responding"),
        agent_cwd("s3", "/other/gamma", "responding"),
    ]);
    assert_eq!(app.agents.len(), 3);

    // Filter by a cwd subsequence — the view narrows, the full set is intact.
    app.open_filter();
    assert!(app.filter_editing);
    for c in "beta".chars() {
        app.handle_filter_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(app.agents.len(), 1);
    assert_eq!(app.agents[0].session_id, "s2");
    assert_eq!(
        app.all_agents.len(),
        3,
        "filter is a view; full set is untouched"
    );

    // A poll while filtered keeps the filter applied.
    app.handle_filter_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.filter_editing);
    app.set_agents(vec![
        agent_cwd("s1", "/work/alpha", "responding"),
        agent_cwd("s2", "/work/beta", "responding"),
    ]);
    assert_eq!(app.agents.len(), 1);
    assert_eq!(app.agents[0].session_id, "s2");

    // Esc clears the filter and restores the full view.
    app.open_filter();
    app.handle_filter_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.filter.is_none());
    assert_eq!(app.agents.len(), 2);
}

#[tokio::test]
async fn vim_count_repeats_and_jumps() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let press = |app: &mut App, c: char| {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    };
    let mut app = test_app();
    app.set_agents(vec![
        agent("s1"),
        agent("s2"),
        agent("s3"),
        agent("s4"),
        agent("s5"),
    ]);
    app.selected = 0; // dashboard row

    // `3j` moves the selection down three rows.
    press(&mut app, '3');
    assert_eq!(app.count, Some(3));
    press(&mut app, 'j');
    assert_eq!(app.selected, 3);
    assert_eq!(app.count, None, "count clears after the motion");

    // `2G` jumps to agent 2.
    press(&mut app, '2');
    press(&mut app, 'G');
    assert_eq!(app.selected, 2);
}

#[tokio::test]
async fn content_search_indexes_and_matches() {
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/a", "responding"),
        agent_cwd("s2", "/b", "responding"),
    ]);
    app.open_search();
    assert_eq!(app.search.as_ref().unwrap().pending, 2);

    // Simulate the per-session index results streaming in.
    app.apply_msg(AppMsg::SearchEntries {
        session_id: "s1".into(),
        name: "alpha".into(),
        lines: vec!["hello world".into(), "foo bar".into()],
    });
    app.apply_msg(AppMsg::SearchEntries {
        session_id: "s2".into(),
        name: "beta".into(),
        lines: vec!["another world entirely".into()],
    });
    {
        let s = app.search.as_ref().unwrap();
        assert_eq!(s.pending, 0);
        assert_eq!(s.entries.len(), 3);
        assert!(s.matched.is_empty(), "empty query matches nothing");
    }

    // A query greps across both agents' transcripts.
    let s = app.search.as_mut().unwrap();
    s.query = "world".into();
    s.rematch();
    assert_eq!(s.matched.len(), 2);
    s.query = "foo".into();
    s.rematch();
    assert_eq!(s.matched.len(), 1);
    assert_eq!(s.chosen().unwrap().session_id, "s1");
}

#[tokio::test]
async fn shell_sessions_are_hidden_from_the_sidebar() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1")]);
    app.selected = 1;
    app.open_agent();
    app.add_shell_tab("s1".into(), "sh-1".into());

    // claudemon now also lists the shell session.
    app.set_agents(vec![agent("s1"), agent("sh-1")]);
    assert_eq!(app.all_agents.len(), 2, "the shell is still a live session");
    assert_eq!(app.agents.len(), 1, "but it's hidden from the sidebar");
    assert!(app.agents.iter().all(|a| a.session_id != "sh-1"));
    assert!(app.is_shell_session("sh-1"));
    // The shell tab still resolves (its terminal/title render, not "ended").
    assert_eq!(app.chat_session_id().as_deref(), Some("sh-1"));
    assert!(app.chat_agent().is_some());
}

#[tokio::test]
async fn palette_has_commands_and_finds_agents_by_cwd() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/work/alpha", "responding")]);
    app.open_palette();
    let p = app.palette.as_mut().unwrap();

    // The command source is present and fuzzy-findable.
    p.query = "vsplit".into();
    p.refilter();
    assert!(p
        .visible()
        .any(|it| matches!(&it.action, PaletteAction::Command(c) if c == "vsplit")));

    // An agent is findable by a substring of its cwd (in the hint).
    p.query = "alpha".into();
    p.refilter();
    assert!(p
        .visible()
        .any(|it| matches!(&it.action, PaletteAction::OpenAgent(_))));
}

#[tokio::test]
async fn ex_commands_dispatch() {
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/work/alpha", "responding"),
        agent_cwd("s2", "/work/beta", "responding"),
    ]);
    app.selected = 1;
    app.open_agent();

    app.run_command("vsplit");
    assert_eq!(app.tiles.len(), 2, ":vsplit tiles another agent");
    app.run_command("only");
    assert_eq!(app.tiles.len(), 1, ":only collapses to one pane");

    app.run_command("filter beta");
    assert_eq!(app.filter.as_deref(), Some("beta"));
    assert_eq!(app.agents.len(), 1);

    app.run_command("nonsense");
    assert_eq!(app.toast(), Some("unknown command: nonsense"));

    app.run_command("q");
    assert!(app.should_quit);
}

#[test]
fn managed_spawned_records_provider_and_prunes() {
    let mut app = test_app();
    app.apply_msg(AppMsg::ManagedSpawned {
        session_id: "m1".into(),
        provider: "codex".into(),
    });
    assert_eq!(
        app.managed_providers.get("m1").map(String::as_str),
        Some("codex")
    );
    // The record is pruned once the session leaves the live set.
    app.set_agents(vec![]);
    assert!(!app.managed_providers.contains_key("m1"));
}

#[test]
fn permission_mode_message_remembers_mode() {
    let mut app = test_app();
    app.apply_msg(AppMsg::PermissionMode {
        session_id: "s1".into(),
        mode: "plan".into(),
    });
    assert_eq!(app.perm_modes.get("s1").map(String::as_str), Some("plan"));
    assert_eq!(app.toast(), Some("Mode: plan"));
}

#[tokio::test]
async fn model_picker_pending_tracks_managed_provider() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 1;

    // Unknown provider → treated as claude → free-text picker, not pending.
    app.open_model_picker();
    {
        let p = app.picker.as_ref().unwrap();
        assert!(!p.pending);
        assert!(p.allow_free_text);
        assert!(matches!(&p.kind, PickerKind::Model { provider, .. } if provider == "claude"));
    }
    app.picker = None;

    // Known managed → picker starts pending while it fetches the model list.
    app.managed_providers.insert("s1".into(), "codex".into());
    app.open_model_picker();
    assert!(app.picker.as_ref().unwrap().pending);
}

#[test]
fn provider_for_prefers_wire_field_then_falls_back_to_local_map() {
    let mut app = test_app();

    // Wire field carries the provider — resolved with no local map entry.
    let codex: Agent = serde_json::from_value(serde_json::json!({
        "session_id": "s1", "mode": "input", "provider": "codex"
    }))
    .unwrap();
    app.set_agents(vec![codex]);
    assert_eq!(app.provider_for("s1"), "codex");

    // A plain wire "claude" session with no local entry stays claude.
    app.set_agents(vec![agent_cwd("s2", "/repo", "input")]);
    assert_eq!(app.provider_for("s2"), "claude");

    // A just-spawned managed agent not yet in the refreshed list: the local
    // map bridges the gap until the wire field arrives.
    app.apply_msg(AppMsg::ManagedSpawned {
        session_id: "s3".into(),
        provider: "pi".into(),
    });
    assert_eq!(app.provider_for("s3"), "pi");

    // Unknown session (neither list nor map) → claude.
    assert_eq!(app.provider_for("nope"), "claude");
}

#[test]
fn picker_models_fold_and_preselect_default() {
    let mut app = test_app();
    app.picker = Some(Picker {
        title: "model".into(),
        kind: PickerKind::Model {
            provider: "codex".into(),
            effort: None,
        },
        session_id: "s1".into(),
        query: String::new(),
        items: Vec::new(),
        matched: Vec::new(),
        selected: 0,
        pending: true,
        allow_free_text: true,
    });
    app.apply_msg(AppMsg::PickerModels {
        session_id: "s1".into(),
        models: vec![
            crate::claudemon::ProviderModel {
                id: "a".into(),
                label: None,
                default: false,
            },
            crate::claudemon::ProviderModel {
                id: "b".into(),
                label: Some("Model B".into()),
                default: true,
            },
        ],
    });
    let p = app.picker.as_ref().unwrap();
    assert!(!p.pending);
    assert_eq!(p.items.len(), 2);
    assert_eq!(
        p.chosen().map(|i| i.id.as_str()),
        Some("b"),
        "default model preselected"
    );
}

#[tokio::test]
async fn handoff_picker_lists_target_providers() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
    app.selected = 1;
    app.open_handoff_picker();
    let p = app.picker.as_ref().unwrap();
    assert!(matches!(p.kind, PickerKind::Handoff { .. }));
    assert_eq!(p.items.len(), 4);
    assert!(p.items.iter().any(|i| i.id == "codex"));
}

#[tokio::test]
async fn managed_provider_spawn_flow_selects_provider() {
    // The spawn modal cycles the provider; a non-claude choice takes the
    // managed path (no profile needed).
    let mut app = test_app();
    app.open_spawn();
    let form = app.spawn_form.as_mut().unwrap();
    form.provider_idx = 1; // "codex"
    assert_eq!(SPAWN_PROVIDERS[form.provider_idx], "codex");
}

fn agent_stream(id: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "cwd": "/repo", "mode": "responding", "transport": "stream"
    }))
    .unwrap()
}

#[tokio::test]
async fn stream_sessions_open_transcript_only_without_warming_a_pty() {
    let mut app = test_app();
    app.set_agents(vec![agent_stream("s1")]);
    app.selected = 1;
    app.open_agent();
    assert_eq!(
        app.chat_mode,
        ChatMode::Transcript,
        "headless sessions default straight to the transcript"
    );
    assert!(
        app.terms.is_empty(),
        "no PTY stream is warmed for a stream-transport session"
    );

    // The `t` terminal toggle is a no-op with a toast.
    app.toggle_chat_mode();
    assert_eq!(app.chat_mode, ChatMode::Transcript);
    assert!(app.terms.is_empty());
    assert_eq!(app.toast(), Some("headless session — no terminal"));
}

#[tokio::test]
async fn ensure_terminal_never_streams_a_headless_session() {
    let mut app = test_app();
    app.set_agents(vec![agent_stream("s1")]);
    app.ensure_terminal("s1".into());
    assert!(app.terms.is_empty() && app.term_tasks.is_empty());
    assert!(
        app.no_terminal.contains("s1"),
        "recorded so watch panes say 'transcript only'"
    );
}

#[tokio::test]
async fn send_echo_survives_stale_refolds_and_retires_on_match() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();
    app.input = "hello there".into();
    app.send_input();
    assert_eq!(app.pending_echo.as_deref(), Some("hello there"));
    assert!(app.input.is_empty(), "the composer clears on send");

    // A refold that doesn't yet carry the message keeps the echo…
    app.apply_msg(AppMsg::Transcript {
        session_id: "s1".into(),
        snapshot: snapshot(&["an older message"]),
    });
    assert_eq!(app.pending_echo.as_deref(), Some("hello there"));

    // …and the refold whose trailing user message matches retires it.
    app.apply_msg(AppMsg::Transcript {
        session_id: "s1".into(),
        snapshot: snapshot(&["an older message", "hello there"]),
    });
    assert!(app.pending_echo.is_none());
}

#[tokio::test]
async fn send_failure_restores_the_composer_and_drops_the_echo() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();
    app.input = "ship it".into();
    app.send_input();
    assert!(app.pending_echo.is_some());

    app.apply_msg(AppMsg::SendFailed {
        text: "ship it".into(),
        error: "connection refused".into(),
    });
    assert!(app.pending_echo.is_none());
    assert_eq!(app.input, "ship it", "the message comes back to retry");
    assert_eq!(app.toast(), Some("Failed: connection refused"));
}

#[tokio::test]
async fn a_transcript_refold_invalidates_the_memoized_render() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();
    app.transcript_cache = Some(TranscriptCache {
        width: 80,
        session_id: app.chat_session_id(),
        commits: app.commits(),
        lines: Vec::new(),
    });
    app.apply_msg(AppMsg::Transcript {
        session_id: "s1".into(),
        snapshot: snapshot(&["fresh"]),
    });
    assert!(
        app.transcript_cache.is_none(),
        "new turns drop the cached lines so the next draw re-renders"
    );
}

/// The memo is keyed on the fold's commit counter, so the render cost of a
/// long conversation is paid when a turn commits — not on every streamed
/// token. Before this split a several-hundred-KB transcript was re-parsed
/// and re-wrapped per token, which the delta rate outruns.
#[tokio::test]
async fn a_streamed_token_does_not_invalidate_the_committed_render() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();
    app.seed_fold(
        "s1",
        &serde_json::json!({
            "seq": 1,
            "items": [{ "kind": "user_message", "text": "go" }]
        }),
    );
    let before = app.commits();

    // One assistant_text delta: the live tail moves, nothing commits.
    app.apply_msg(AppMsg::ConvDelta(Box::new(crate::claudemon::ConvDelta {
        session_id: "s1".into(),
        seq: 2,
        reset: false,
        items: vec![serde_json::json!({ "kind": "assistant_text", "text": "wor" })],
    })));
    assert_eq!(
        app.commits(),
        before,
        "streamed text is pending, not a committed turn"
    );
    assert_eq!(app.pending_text(), Some("wor"));

    // A tool call in the same stream DOES commit, so the memo must rebuild.
    app.apply_msg(AppMsg::ConvDelta(Box::new(crate::claudemon::ConvDelta {
        session_id: "s1".into(),
        seq: 3,
        reset: false,
        items: vec![serde_json::json!({
            "kind": "tool_use", "id": "t1", "name": "Bash", "input": {}
        })],
    })));
    assert!(
        app.commits() > before,
        "a committed turn bumps the render key"
    );
}

/// A sequencing gap must fire ONE resync, not one per delta that lands
/// during the round trip — each of those re-folded the whole conversation,
/// and an older response arriving after a newer one adopted a stale
/// snapshot.
#[tokio::test]
async fn a_sequencing_gap_resyncs_once_while_the_fetch_is_in_flight() {
    let mut app = test_app();
    app.set_agents(vec![agent_cwd("s1", "/repo", "input")]);
    app.selected = 1;
    app.open_agent();
    app.seed_fold(
        "s1",
        &serde_json::json!({
            "seq": 1,
            "items": [{ "kind": "user_message", "text": "go" }]
        }),
    );

    let gap = |seq: u64| {
        AppMsg::ConvDelta(Box::new(crate::claudemon::ConvDelta {
            session_id: "s1".into(),
            seq,
            reset: false,
            items: vec![serde_json::json!({ "kind": "assistant_text", "text": "x" })],
        }))
    };
    // seq 9 can't follow seq 1 with one item — that's the gap.
    app.apply_msg(gap(9));
    assert!(
        app.resyncing.contains_key("s1"),
        "first gap claims the slot"
    );
    assert!(
        !app.begin_resync("s1"),
        "further gaps during the fetch are dropped"
    );

    // The snapshot landing releases the slot for the next real gap.
    app.apply_msg(AppMsg::Transcript {
        session_id: "s1".into(),
        snapshot: snapshot(&["fresh"]),
    });
    assert!(!app.resyncing.contains_key("s1"));
    assert!(app.begin_resync("s1"), "a later gap can resync again");
}

/// A fetch that fails sends nothing back, so the guard must self-heal
/// rather than wedging resync for the session forever.
#[tokio::test]
async fn a_stale_resync_claim_expires() {
    let mut app = test_app();
    app.resyncing.insert(
        "s1".into(),
        Instant::now() - std::time::Duration::from_secs(30),
    );
    assert!(app.begin_resync("s1"), "a stale claim is reclaimable");
}

fn questions(v: serde_json::Value) -> Vec<crate::types::Question> {
    serde_json::from_value(v).unwrap()
}

#[test]
fn question_flow_clears_when_the_set_is_gone() {
    let mut app = test_app();
    let qs = questions(serde_json::json!([
        {"question": "One?"}, {"question": "Two?"}
    ]));
    app.question_flow = Some(QuestionFlow::new("s1".into(), &qs));
    // s1 is live but no longer has a pending question → the stepper drops.
    app.set_agents(vec![agent("s1")]);
    assert!(app.question_flow.is_none());
}

#[test]
fn question_flow_clears_when_a_same_length_set_supersedes_it() {
    // A different question set of the SAME length (the old one was
    // resolved elsewhere, the agent asked again) must not inherit the
    // stale flow's position and recorded answers.
    let mut app = test_app();
    let old = questions(serde_json::json!([
        {"question": "Old one?"}, {"question": "Old two?"}
    ]));
    let mut flow = QuestionFlow::new("s1".into(), &old);
    flow.idx = 1;
    flow.answers[0] = Some("2".into());
    app.question_flow = Some(flow);

    let asking: Agent = serde_json::from_value(serde_json::json!({
        "session_id": "s1", "cwd": "/w", "mode": "question",
        "pending": {"kind": "question", "questions": [
            {"question": "New one?"}, {"question": "New two?"}
        ]}
    }))
    .unwrap();
    app.set_agents(vec![asking]);
    assert!(
        app.question_flow.is_none(),
        "the superseded flow drops instead of answering unseen questions"
    );
}

#[tokio::test]
async fn jumplist_steps_back_and_forward() {
    let mut app = test_app();
    app.set_agents(vec![agent("s1"), agent("s2"), agent("s3")]);
    app.selected = 1;
    app.open_agent(); // s1
    app.selected = 2;
    app.open_agent(); // s2
    app.selected = 3;
    app.open_agent(); // s3
    app.jump_history(-1);
    assert_eq!(app.open_agent_id(), Some("s2"));
    app.jump_history(-1);
    assert_eq!(app.open_agent_id(), Some("s1"));
    app.jump_history(1);
    assert_eq!(app.open_agent_id(), Some("s2"));
}

// ── sidebar filter ──────────────────────────────────────────────────────

/// `apply_filter` derives the visible `agents` list from `all_agents`. Two
/// rules ride on it that nothing else enforces: TUI-spawned shells never get
/// a sidebar row, and the selection follows the agent it was on rather than
/// the row index it happened to occupy.
#[test]
fn the_filter_narrows_by_name_cwd_or_state() {
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/repo/alpha", "responding"),
        agent_cwd("s2", "/repo/beta", "input"),
    ]);

    let visible =
        |app: &App| -> Vec<String> { app.agents.iter().map(|a| a.session_id.clone()).collect() };
    assert_eq!(visible(&app), vec!["s1", "s2"], "no filter shows all");

    app.filter = Some("alpha".into());
    app.apply_filter();
    assert_eq!(visible(&app), vec!["s1"], "matches the cwd");

    app.filter = Some("input".into());
    app.apply_filter();
    assert_eq!(visible(&app), vec!["s2"], "matches the state");

    // Subsequence, not substring — that is what fuzzy_match does. "aph" is
    // not contiguous anywhere in /repo/alpha, but its letters appear in
    // order; /repo/beta has no 'p' after an 'a' at all.
    app.filter = Some("aph".into());
    app.apply_filter();
    assert_eq!(visible(&app), vec!["s1"]);

    app.filter = Some("zzz".into());
    app.apply_filter();
    assert!(visible(&app).is_empty());

    // An empty string is not a filter.
    app.filter = Some(String::new());
    app.apply_filter();
    assert_eq!(visible(&app), vec!["s1", "s2"]);
}

#[test]
fn the_filter_keeps_the_selection_on_the_same_agent() {
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/repo/alpha", "input"),
        agent_cwd("s2", "/repo/beta", "input"),
    ]);
    app.selected = 2; // row 0 is the dashboard, so this is s2

    app.filter = Some("beta".into());
    app.apply_filter();
    assert_eq!(
        app.selected, 1,
        "s2 is now the only row, and still selected"
    );

    // Filtering the selected agent away drops the selection to the dashboard
    // rather than leaving it pointing at a row that no longer exists.
    app.filter = Some("alpha".into());
    app.apply_filter();
    assert_eq!(app.selected, 0);
}

// ── harpoon ─────────────────────────────────────────────────────────────

/// Pins persist as cwds, not session ids — a cwd outlives the session in it.
/// `rebuild_harpoon` re-resolves them, and a pin whose agent isn't running is
/// simply absent, so the reachable slots stay gap-free.
#[test]
fn harpoon_resolves_pinned_cwds_to_whatever_session_is_in_them() {
    let mut app = test_app();
    app.set_agents(vec![
        agent_cwd("s1", "/repo/alpha", "input"),
        agent_cwd("s2", "/repo/beta", "input"),
    ]);
    app.pinned_cwds = vec!["/repo/beta".into(), "/repo/alpha".into()];
    app.rebuild_harpoon();
    assert_eq!(
        app.harpoon,
        vec!["s2", "s1"],
        "in pin order, not agent order"
    );

    // The session in /repo/beta is replaced by a new one in the same cwd.
    app.set_agents(vec![
        agent_cwd("s1", "/repo/alpha", "input"),
        agent_cwd("s9", "/repo/beta", "input"),
    ]);
    app.rebuild_harpoon();
    assert_eq!(app.harpoon, vec!["s9", "s1"], "the cwd is the identity");

    // A pin with nothing running in it closes the gap rather than leaving a
    // hole, so slot 1 is always the first reachable pin.
    app.set_agents(vec![agent_cwd("s1", "/repo/alpha", "input")]);
    app.rebuild_harpoon();
    assert_eq!(app.harpoon, vec!["s1"]);
}

// ── transport resolution ────────────────────────────────────────────────

/// Which transport a session runs on, and which a respawn should use. These
/// disagree on purpose: an unknown session reads as `pty` for display, but a
/// respawn of one falls back to the *configured* transport, because that is
/// the only signal available for a row we have never seen live.
#[test]
fn transport_reads_the_session_and_respawn_falls_back_to_config() {
    use crate::config::Transport;
    let mut app = test_app();
    let stream: Agent = serde_json::from_value(serde_json::json!({
        "session_id": "s-stream", "cwd": "/repo", "mode": "input", "transport": "stream"
    }))
    .unwrap();
    app.set_agents(vec![agent_cwd("s-pty", "/repo", "input"), stream]);

    assert_eq!(app.transport_for("s-pty"), "pty");
    assert_eq!(app.transport_for("s-stream"), "stream");
    assert_eq!(
        app.transport_for("s-never-seen"),
        "pty",
        "an unknown session displays as pty"
    );

    assert!(!app.is_stream_session("s-pty"));
    assert!(app.is_stream_session("s-stream"));
    assert!(!app.is_stream_session("s-never-seen"));

    // A respawn keeps the session's own transport — flipping one under the
    // user loses a terminal view, or hands out one that cannot exist.
    assert_eq!(app.respawn_transport("s-pty"), Transport::Pty);
    assert_eq!(app.respawn_transport("s-stream"), Transport::Stream);

    // ...but a session we have never seen live respawns on the configured
    // transport, NOT on the pty default that transport_for reports.
    app.transport = Transport::Stream;
    assert_eq!(app.respawn_transport("s-never-seen"), Transport::Stream);
    app.transport = Transport::Pty;
    assert_eq!(app.respawn_transport("s-never-seen"), Transport::Pty);
}
