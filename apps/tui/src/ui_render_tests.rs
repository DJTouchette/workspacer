//! Render-level characterization tests for [`super::render`].
//!
//! The helper tests in `ui.rs` cover the pure formatters. These cover the part
//! that has no return value to assert on: the draw path itself. They render
//! into a ratatui [`TestBackend`] and read the resulting cell buffer, the same
//! way `claudemon`'s watch TUI tests do (`services/claudemon/src/tui/view`).
//!
//! They deliberately pin *structure* rather than golden screenfuls: the chrome
//! geometry, which screen the view dispatch chose, and which overlay wins when
//! several are open. Those are the invariants a decomposition of this module
//! can silently break — a golden string would also break on every copy edit,
//! which would train everyone to regenerate it without reading it.

use super::*;
use ratatui::{backend::TestBackend, Terminal};

const W: u16 = 100;
const H: u16 = 30;

/// Render `app` and return the buffer as one string per row.
fn rows(app: &mut App, w: u16, h: u16) -> Vec<String> {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| super::render(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn screen(app: &mut App) -> Vec<String> {
    rows(app, W, H)
}

fn joined(app: &mut App) -> String {
    screen(app).join("\n")
}

/// A default App with no live daemon: the channels are dropped receivers, so
/// anything the render path emits goes nowhere.
fn test_app() -> App {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
    App::new(
        crate::claudemon::Claudemon::new("http://127.0.0.1:59999".into()),
        Vec::new(),
        Vec::new(),
        crate::config::Config::default(),
        tx,
        ptx,
    )
}

fn agent(id: &str, mode: &str) -> Agent {
    serde_json::from_value(serde_json::json!({
        "session_id": id,
        "mode": mode,
        "cwd": format!("/tmp/{id}"),
        "provider": "claude",
    }))
    .expect("agent fixture")
}

/// An app with two agents listed, sitting on the dashboard row.
fn app_with_agents() -> App {
    let mut app = test_app();
    app.all_agents = vec![agent("alpha", "responding"), agent("beta", "input")];
    app
}

// ── chrome geometry ──────────────────────────────────────────────────────────

/// The frame is a 1-row header, a 1-row footer, and everything else in between.
/// `render` slices this with fixed `Length(1)` constraints; if a decomposition
/// re-splits the root the body would start or end on the wrong row.
#[test]
fn the_frame_is_a_one_row_header_and_a_one_row_footer() {
    let mut app = app_with_agents();
    let s = screen(&mut app);
    assert_eq!(s.len(), H as usize);

    // Row 0 is the header and row H-1 the footer: both non-blank, and the
    // sidebar's border must not have reached either of them.
    assert!(!s[0].trim().is_empty(), "header row blank: {:?}", s[0]);
    assert!(
        !s[(H - 1) as usize].trim().is_empty(),
        "footer row blank: {:?}",
        s[(H - 1) as usize]
    );
}

/// The sidebar is a fixed 34 columns and the content column takes the rest.
/// This constant is load-bearing for every `render_*` that receives `content`.
#[test]
fn the_sidebar_is_thirty_four_columns_wide() {
    let mut app = app_with_agents();
    let s = screen(&mut app);

    // The sidebar is a bordered block, so its right edge sits at column 33.
    let body: Vec<&String> = s[1..(H - 1) as usize].iter().collect();
    let border_col = body
        .iter()
        .filter_map(|row| row.chars().position(|c| c == '│'))
        .min()
        .expect("a vertical border somewhere in the body");
    assert_eq!(border_col, 0, "sidebar starts at column 0");

    // Its closing border is the 34th column (index 33).
    let closes_at_33 = body
        .iter()
        .any(|row| row.chars().nth(33).is_some_and(|c| c == '│'));
    assert!(closes_at_33, "sidebar should close at column 33");
}

// ── view dispatch ────────────────────────────────────────────────────────────

/// Row 0 of the sidebar is the dashboard; selecting it renders the dashboard
/// into the content column, not an agent's detail.
#[test]
fn the_list_view_shows_the_dashboard_on_row_zero_and_detail_below_it() {
    let mut app = app_with_agents();
    app.view = View::List;
    app.selected = 0;
    assert!(app.dashboard_selected());
    let dash = joined(&mut app);

    app.selected = 1;
    assert!(!app.dashboard_selected());
    let detail = joined(&mut app);

    assert_ne!(
        dash, detail,
        "dashboard and detail must not render identically"
    );
    assert!(
        detail.contains("details"),
        "the detail pane titles itself: {detail}"
    );
    assert!(
        !dash.contains("details"),
        "the dashboard is not the detail pane"
    );
}

/// The runs overlay owns the whole content column — it replaces the view
/// dispatch rather than drawing beside it.
#[test]
fn the_runs_overlay_replaces_the_view_entirely() {
    let mut app = app_with_agents();
    app.selected = 1;
    let detail = joined(&mut app);
    assert!(detail.contains("details"));

    app.runs_open = Some("alpha".into());
    let runs = joined(&mut app);
    assert!(
        !runs.contains("details"),
        "the detail pane must be gone: {runs}"
    );
}

/// A docked side pane splits the content column 55/45, and the runs overlay
/// suppresses it — `render` checks `runs_open` before it decides to split.
#[test]
fn a_docked_side_pane_splits_the_content_column_and_yields_to_runs() {
    let mut app = app_with_agents();
    app.selected = 1;
    let undocked = joined(&mut app);

    app.side = Some(crate::app::SidePane {
        kind: crate::app::SideKind::Changes,
        session_id: "alpha".into(),
        focused: false,
        scroll: 0,
    });
    let docked = joined(&mut app);
    assert_ne!(undocked, docked, "docking a pane must change the layout");

    // With the runs overlay open the split is skipped entirely.
    app.runs_open = Some("alpha".into());
    let with_runs = joined(&mut app);
    assert_ne!(docked, with_runs);
}

// ── overlay precedence ───────────────────────────────────────────────────────

/// Help draws last, so it floats above every other overlay. This ordering is
/// the single most breakable thing in `render`: the modal block is a flat run
/// of `if`s whose *sequence* is the z-order.
#[test]
fn help_floats_above_the_other_overlays() {
    let mut app = app_with_agents();
    app.notes_view = Some(crate::app::NotesState {
        cwd: "/tmp/alpha".into(),
        text: "a note".into(),
        editing: false,
        scroll: 0,
    });
    let notes_only = joined(&mut app);
    assert!(notes_only.contains("a note"), "notes render: {notes_only}");

    app.help = true;
    let with_help = joined(&mut app);
    assert_ne!(notes_only, with_help, "help must paint over the notes");
}

/// Each overlay actually reaches the screen on its own. A decomposition that
/// drops one of these `if` arms would otherwise be invisible until someone
/// pressed the key.
#[test]
fn every_overlay_changes_what_is_drawn() {
    let base = joined(&mut app_with_agents());

    let mut spawn = app_with_agents();
    spawn.spawn_form = Some(crate::app::SpawnForm {
        cwd: "/tmp/alpha".into(),
        profile_idx: 0,
        provider_idx: 0,
        completions: Vec::new(),
        initial_prompt: None,
    });
    assert_ne!(base, joined(&mut spawn), "spawn modal did not render");

    let mut rename = app_with_agents();
    rename.rename = Some(crate::app::RenameForm {
        cwd: "/tmp/alpha".into(),
        input: "renamed".into(),
    });
    let out = joined(&mut rename);
    assert_ne!(base, out, "rename modal did not render");
    assert!(out.contains("renamed"), "rename shows its buffer: {out}");

    let mut help = app_with_agents();
    help.help = true;
    assert_ne!(base, joined(&mut help), "help did not render");

    let mut notes = app_with_agents();
    notes.notes_view = Some(crate::app::NotesState {
        cwd: "/tmp/alpha".into(),
        text: "scratch".into(),
        editing: true,
        scroll: 0,
    });
    assert_ne!(base, joined(&mut notes), "notes did not render");
}

// ── remote nodes ─────────────────────────────────────────────────────────────

/// One node of every state, plus the two notices that only ever show up on a
/// row (a crash record, and failed wakes that left money burning).
fn node_rows() -> Vec<crate::nodes::NodeView> {
    crate::nodes::nodes_from_rows(&serde_json::json!([
        { "id": "den", "label": "Fly node (den)", "state": "stopped", "wakeable": true },
        { "id": "waker", "label": "waker", "state": "waking", "wakeable": true },
        { "id": "draining", "label": "draining", "state": "stopping", "wakeable": true },
        { "id": "broken", "label": "broken", "state": "unreachable", "wakeable": true,
          "wakeFailures": 2 },
        { "id": "fine", "label": "fine", "state": "available", "wakeable": true,
          "lastExit": { "reason": "claudemon-died", "exitCode": 1 } },
    ]))
}

/// The overlay, optionally with a wake armed for `confirm`.
fn nodes_state(confirm: Option<&str>) -> crate::app::NodesState {
    crate::app::NodesState {
        selected: 0,
        confirm: confirm.map(String::from),
        pending: std::collections::HashSet::new(),
        errors: std::collections::HashMap::new(),
    }
}

/// An app holding the registry above, as an operator (so the wake is on offer).
fn app_with_nodes() -> App {
    let mut app = app_with_agents();
    app.nodes = Some(crate::nodes::NodeRegistry::new(node_rows()));
    app.bus_scope = Some("operator".into());
    app
}

/// The overlay reaches the screen and names every machine — including the
/// healthy one, which the dashboard line deliberately stays quiet about. An
/// explicitly opened surface answers in full.
#[test]
fn the_nodes_overlay_lists_every_machine_the_hub_knows_about() {
    let base = joined(&mut app_with_nodes());
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert_ne!(base, out, "nodes overlay did not render");
    assert!(out.contains("remote nodes"), "titled: {out}");
    for id in ["Fly node (den)", "waker", "draining", "broken", "fine"] {
        assert!(out.contains(id), "{id} missing from: {out}");
    }
}

/// The whole point of the five-state model: a booting machine and a broken one
/// must not read the same. Pinned on the rendered text, not just the helpers.
#[test]
fn the_overlay_draws_waking_and_unreachable_as_different_things() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(
        out.contains("starting"),
        "a booting machine reads as progress: {out}"
    );
    assert!(
        out.contains("can't reach"),
        "a broken one reads as a warning: {out}"
    );
    assert!(
        out.contains("asleep"),
        "and a sleeping one as neither: {out}"
    );
}

/// The state this client learned late. A machine draining on purpose used to
/// coerce to `unreachable` here — so the row said "can't reach" and wore the
/// warning mark for a shutdown the person had just asked for.
#[test]
fn the_overlay_draws_a_machine_shutting_down_as_work_in_progress() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(out.contains("shutting down"), "it says so: {out}");
    assert!(
        out.contains("stops billing once it is off"),
        "and says what that means for the meter: {out}"
    );
    // It carries the progress mark `waking` wears, not the warning triangle.
    assert!(out.contains("◑ draining"), "a progress mark: {out}");
    assert!(!out.contains("▲ draining"), "not a warning one: {out}");
    // And it is counted in the title's collapsed reading rather than dropped.
    assert!(out.contains("1 shutting down"), "counted: {out}");
}

/// The bug in full, on the screen. The hub ACCEPTS a wake on a `stopping` node
/// and cancels the stop, so offering one here would let a person undo their own
/// shutdown with a single key — and be billed for it.
#[test]
fn no_wake_is_offered_on_a_machine_that_is_already_shutting_down() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(
        out.contains("already shutting down"),
        "the row explains itself: {out}"
    );
    // The wake IS on offer elsewhere on this screen (den is asleep), so the
    // absence has to be read per row rather than off the whole frame.
    let draining = out
        .lines()
        .skip_while(|l| !l.contains("draining"))
        .take_while(|l| !l.contains("broken"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !draining.contains("w  wake"),
        "and offers no wake: {draining}"
    );
}

/// A wake spends real money and this client cannot spend it back, so the price
/// is on the screen — a terminal has no hover to hide it behind.
#[test]
fn the_overlay_prints_what_a_wake_costs_beside_the_action() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(out.contains("w  wake"), "the action is offered: {out}");
    assert!(
        out.contains("bills from boot"),
        "the cost is printed: {out}"
    );
    // `nodes.sleep` exists now, so the note names who can stop the machine
    // rather than claiming — as it used to — that nobody can.
    assert!(
        out.contains("not this one"),
        "and says who can stop it: {out}"
    );
}

/// Never render a control that will be refused: a view/triage token gets the
/// STATE and the reason, never a verb that dies on press.
#[test]
fn a_scoped_token_sees_the_state_and_is_told_why_it_gets_no_wake() {
    let mut app = app_with_nodes();
    app.bus_scope = Some("triage".into());
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(out.contains("asleep"), "the state is still shown: {out}");
    assert!(out.contains("operator token"), "with the reason: {out}");
    assert!(!out.contains("w  wake"), "and no live action: {out}");
}

/// The confirmation step names the CONSEQUENCE, and says which single key
/// commits. Nothing else on this screen leads to a bus call.
#[test]
fn the_confirmation_step_names_the_consequence_and_the_one_key_that_commits() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(Some("den")));
    let out = joined(&mut app);
    assert!(out.contains("wake Fly node (den)?"), "it asks: {out}");
    assert!(out.contains("bills from boot"), "and prices it: {out}");
    assert!(out.contains("confirm"), "and names the confirm key: {out}");
}

/// The two notices a person only ever gets here: the node's own crash record
/// (which arrives one wake late by construction) and failed wakes, each of
/// which left a machine that started and never became usable.
#[test]
fn the_overlay_surfaces_a_crash_record_and_failed_wakes() {
    let mut app = app_with_nodes();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(out.contains("did not end cleanly"), "crash notice: {out}");
    assert!(out.contains("2 wakes failed"), "failed wakes: {out}");
    assert!(
        out.contains("never became usable"),
        "described honestly — the hub stopped it again: {out}"
    );
}

/// An explicitly requested overlay always answers — including with the answer
/// that is true for almost every install.
#[test]
fn the_overlay_says_so_when_this_hub_has_no_remote_nodes() {
    let mut app = app_with_agents();
    app.nodes_view = Some(nodes_state(None));
    let out = joined(&mut app);
    assert!(out.contains("no remote nodes"), "it answers: {out}");
}

/// The dashboard, by contrast, stays SILENT about a fleet that is quietly fine
/// — a permanent "all good" row is chrome nobody asked for — and speaks the
/// moment one machine is asleep, unreachable, or back from a crash.
#[test]
fn the_dashboard_line_is_silent_until_a_node_is_not_quietly_fine() {
    let healthy = crate::nodes::nodes_from_rows(&serde_json::json!([
        { "id": "fine", "label": "fine", "state": "available", "wakeable": true }
    ]));
    let mut quiet = app_with_agents();
    quiet.selected = 0;
    quiet.nodes = Some(crate::nodes::NodeRegistry::new(healthy));
    let out = joined(&mut quiet);
    assert!(
        !out.contains("nodes "),
        "nothing to report renders nothing: {out}"
    );

    let mut loud = app_with_nodes();
    loud.selected = 0;
    let out = joined(&mut loud);
    assert!(
        out.contains("asleep"),
        "a sleeping machine is on the overview: {out}"
    );
    assert!(
        out.contains("can't reach"),
        "so is an unreachable one: {out}"
    );
}

// ── resilience ───────────────────────────────────────────────────────────────

// ── small terminals ──────────────────────────────────────────────────────────

/// A named way of opening one overlay on a fresh app.
type Overlay = (&'static str, fn(&mut App));

/// Every way an overlay can be open, so the size sweep below can try each one
/// on its own rather than only in combination — a panic in one overlay would
/// otherwise be masked by whichever one drew first.
fn overlays() -> Vec<Overlay> {
    vec![
        ("none", |_a: &mut App| {}),
        ("help", |a: &mut App| a.help = true),
        ("runs", |a: &mut App| a.runs_open = Some("alpha".into())),
        ("spawn", |a: &mut App| {
            a.spawn_form = Some(crate::app::SpawnForm {
                cwd: "/tmp/alpha".into(),
                profile_idx: 0,
                provider_idx: 0,
                completions: Vec::new(),
                initial_prompt: None,
            })
        }),
        ("rename", |a: &mut App| {
            a.rename = Some(crate::app::RenameForm {
                cwd: "/tmp/alpha".into(),
                input: "x".into(),
            })
        }),
        ("notes", |a: &mut App| {
            a.notes_view = Some(crate::app::NotesState {
                cwd: "/tmp/alpha".into(),
                text: "n".into(),
                editing: false,
                scroll: 0,
            })
        }),
        ("side", |a: &mut App| {
            a.side = Some(crate::app::SidePane {
                kind: crate::app::SideKind::Changes,
                session_id: "alpha".into(),
                focused: false,
                scroll: 0,
            })
        }),
        ("whichkey", |a: &mut App| {
            a.pending_keys = vec![a.keymap.leader()]
        }),
        ("nodes", |a: &mut App| {
            a.nodes = Some(crate::nodes::NodeRegistry::new(node_rows()));
            a.bus_scope = Some("operator".into());
            a.nodes_view = Some(nodes_state(Some("den")));
        }),
    ]
}

/// The draw path must not panic on a terminal too small to hold its own
/// chrome — you get one by dragging a window shorter or splitting a tmux pane,
/// and ratatui panics on any widget whose rect leaves the buffer.
///
/// This is a regression test with history: before [`super::modal_rect`] each
/// overlay centred itself by hand, and the preferred size was clamped to the
/// frame at only some of those sites. `notes` crashed at 80×3, `spawn` and
/// `rename` at 10×4, and `whichkey` panicked inside `clamp` itself on anything
/// under 18 columns wide.
#[test]
fn no_overlay_panics_on_a_terminal_too_small_for_it() {
    let sizes = [
        (1, 1),
        (2, 3),
        (4, 2),
        (10, 4),
        (17, 5),
        (18, 6),
        (20, 5),
        (34, 2),
        (35, 3),
        (40, 6),
        (80, 1),
        (80, 3),
        (100, 2),
    ];
    for (name, open) in overlays() {
        for (w, h) in sizes {
            let mut app = app_with_agents();
            open(&mut app);
            let drawn = rows(&mut app, w, h);
            assert_eq!(drawn.len(), h as usize, "{name} at {w}x{h}");
        }
    }
}

/// All of them open at once, which is not reachable by keyboard but is the
/// cheapest way to prove the overlays don't corrupt each other's geometry.
#[test]
fn every_overlay_at_once_survives_any_size() {
    for (w, h) in [(1, 1), (10, 4), (35, 3), (80, 24), (200, 60)] {
        let mut app = app_with_agents();
        for (_, open) in overlays() {
            open(&mut app);
        }
        let drawn = rows(&mut app, w, h);
        assert_eq!(drawn.len(), h as usize, "{w}x{h}");
    }
}

// ── modal_rect ───────────────────────────────────────────────────────────────

/// The containment guarantee the overlays now depend on, asserted directly:
/// whatever it is asked for, the result never leaves `area`.
#[test]
fn modal_rect_never_escapes_its_area() {
    for aw in [0u16, 1, 5, 20, 80] {
        for ah in [0u16, 1, 3, 10, 40] {
            let area = Rect {
                x: 3,
                y: 7,
                width: aw,
                height: ah,
            };
            for want in [(0, 0), (1, 1), (24, 6), (76, 24), (500, 500)] {
                for y in [ModalY::Centered, ModalY::Top(2), ModalY::Bottom] {
                    let r = modal_rect(area, want.0, want.1, y);
                    assert!(r.width <= area.width, "{r:?} wider than {area:?}");
                    assert!(r.height <= area.height, "{r:?} taller than {area:?}");
                    assert!(r.x >= area.x && r.y >= area.y, "{r:?} before {area:?}");
                    assert!(
                        r.x + r.width <= area.x + area.width,
                        "{r:?} spills right of {area:?}"
                    );
                    assert!(
                        r.y + r.height <= area.y + area.height,
                        "{r:?} spills below {area:?}"
                    );
                }
            }
        }
    }
}

/// Placement is honoured whenever there is room for it — the clamp is a
/// backstop, not the normal path.
#[test]
fn modal_rect_places_the_box_where_it_was_asked_to() {
    let area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 30,
    };
    let centered = modal_rect(area, 40, 10, ModalY::Centered);
    assert_eq!((centered.x, centered.y), (30, 10), "centred both ways");
    assert_eq!((centered.width, centered.height), (40, 10));

    let top = modal_rect(area, 40, 10, ModalY::Top(2));
    assert_eq!(top.y, 2, "inset from the top");

    let bottom = modal_rect(area, 40, 10, ModalY::Bottom);
    assert_eq!(bottom.y, 20, "flush to the bottom edge");

    // Offsets respect the origin rather than assuming (0, 0).
    let offset = Rect {
        x: 10,
        y: 5,
        width: 100,
        height: 30,
    };
    assert_eq!(modal_rect(offset, 40, 10, ModalY::Top(2)).y, 7);
    assert_eq!(modal_rect(offset, 40, 10, ModalY::Centered).x, 40);
}

/// A box that cannot fit degrades to the area rather than being pushed off it:
/// the top inset collapses before the height does.
#[test]
fn modal_rect_gives_up_its_inset_before_its_size() {
    let short = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 4,
    };
    let r = modal_rect(short, 40, 4, ModalY::Top(2));
    assert_eq!(
        (r.y, r.height),
        (0, 4),
        "kept full height, dropped the inset"
    );

    let squeezed = modal_rect(short, 40, 10, ModalY::Top(2));
    assert_eq!((r.y, squeezed.height), (0, 4), "height capped to the area");
}

/// The project mark, end to end through the real draw path.
///
/// The unit tests in `projects.rs` pin the resolver against its desktop twin;
/// this pins that the sidebar actually *reaches* it — a mark that resolves
/// correctly and never gets drawn (or gets drawn uncoloured) is the failure this
/// feature exists to prevent. Two sibling repos sharing a prefix are the case
/// that matters: they must differ in both channels, glyph and colour.
#[test]
fn the_sidebar_draws_each_agent_a_coloured_project_mark() {
    let mut app = test_app();
    let listed = |cwd: &str, id: &str| -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "mode": "input", "cwd": cwd, "provider": "claude",
        }))
        .expect("agent fixture")
    };
    app.agents = vec![listed("/w/api-gateway", "a"), listed("/w/api-worker", "b")];
    app.all_agents = app.agents.clone();

    let backend = TestBackend::new(W, H);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| super::render(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();

    // Sidebar row 0 is the Dashboard (two lines), so the agents' name lines are
    // the next two odd rows inside the border.
    let mark_at = |y: u16| -> (String, ratatui::style::Color) {
        let cells: Vec<_> = (3..5).map(|x| &buffer[(x, y)]).collect();
        (
            cells.iter().map(|c| c.symbol()).collect::<String>(),
            cells[0].style().fg.expect("the mark is coloured"),
        )
    };
    let (gateway, gateway_fg) = mark_at(4);
    let (worker, worker_fg) = mark_at(6);

    assert_eq!(gateway, "AG", "row 1 is api-gateway: {gateway:?}");
    assert_eq!(worker, "AW", "row 2 is api-worker: {worker:?}");
    assert_eq!(gateway_fg, ratatui::style::Color::Rgb(0xfb, 0x92, 0x3c));
    assert_eq!(worker_fg, ratatui::style::Color::Rgb(0xc0, 0x84, 0xfc));
    assert_ne!(
        gateway_fg, worker_fg,
        "sibling repos must differ in colour as well as in initials"
    );

    // A configured label renames the mark without recolouring it — the wiring
    // reads `config.projects`, not just the path.
    app.projects.insert(
        "/w/api-gateway".to_string(),
        crate::projects::ProjectIdentity {
            label: Some("Public Edge".into()),
            color: None,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();
    terminal.draw(|f| super::render(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let renamed: String = (3..5).map(|x| buffer[(x, 4)].symbol()).collect();
    assert_eq!(renamed, "PE", "the configured label drives the initials");
    assert_eq!(
        buffer[(3, 4)].style().fg,
        Some(gateway_fg),
        "renaming a project must not recolour it"
    );
}

/// An empty fleet still draws its chrome — the first-run screen is a real
/// state, not an error path.
#[test]
fn rendering_survives_an_empty_fleet() {
    let mut app = test_app();
    let s = screen(&mut app);
    assert!(!s[0].trim().is_empty(), "header still draws with no agents");
    assert!(!s[(H - 1) as usize].trim().is_empty(), "footer still draws");
}
