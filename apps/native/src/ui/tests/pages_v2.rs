//! Design v2 pages (area-pages): Hosts behind Details, Projects nesting
//! folders under their project, Jobs' New job and overflow Remove, Settings
//! folding Chat into Appearance and Remote's unavailable state, and About's
//! offline check. Every test forces v2; the v1 pages keep their own tests.
use super::*;
use serde_json::json;
use wks_native::appearance::Design;
use wks_native::features::{HostAction, JobAction, Request, RequestState};

fn open_v2(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    height: f32,
    open: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    visual.simulate_resize(size(px(1500.), px(height)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(Design::V2));
            this.update_view(Arc::new(state("a")), window, cx);
            open(this, window, cx);
        })
    });
    visual.run_until_parked();
}

/// A failed request, as the controller records it.
fn request_error(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    key: &'static str,
    request: Request,
    error: &str,
) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                key,
                RequestState {
                    request,
                    number: 50,
                    loading: false,
                    error: Some(error.into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

fn hosts_snapshot() -> serde_json::Value {
    let resources = |host: &str, open: bool| {
        json!({"host": host, "sampledAt": 1,
            "cpu": {"cores": 8, "usagePct": 23.5, "pressureAvg10": 3.1},
            "memory": {"totalMb": 31800, "availableMb": 12288, "pressureAvg10": 1.0},
            "io": {"pressureAvg10": 0.4},
            "disk": {"freeMb": 80000},
            "queue": {"agentSlots": 4, "agentsRunning": 1, "shellSlots": 2, "shellRunning": 0,
                "queued": 1, "gate": {"open": open,
                    "reasons": if open { json!([]) } else { json!(["memory pressure 34.0 >= 25.0"]) }}}})
    };
    json!({
        "resources": resources("studio", true),
        "list": {"host": "studio", "items": [
            {"id": "q_wait", "kind": "spawn", "state": "queued", "label": "Nightly",
             "spawn": {"provider": "claude", "cwd": "/work/api"}, "enqueuedAt": 3,
             "waitReason": "no free agent slot"}
        ], "peers": []},
        "targets": {"targets": [
            {"host": "local", "local": true, "connected": true, "dispatch": true,
             "resources": resources("studio", true), "cwds": [], "eligible": true, "reasons": []},
            {"host": "gpu-box", "connected": true, "dispatch": true,
             "resources": resources("gpu-box", false), "cwds": [], "eligible": true, "reasons": []}
        ], "best": "local"},
        "config": {"maxAgents": 4, "maxShell": 2, "maxCpuPressure": 60,
            "maxMemoryPressure": 25, "minAvailableMemoryMb": 1024, "minFreeDiskMb": 2048}
    })
}

#[gpui::test]
fn hosts_lead_with_one_line_per_machine_and_open_details_on_demand(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 1400., |this, window, cx| {
        this.open_feature(Screen::Hosts, window, cx)
    });
    request_state(&workspace, &mut visual, Request::Hosts, 1, hosts_snapshot());
    for selector in [
        "host-card-0",
        "host-card-1",
        "host-summary-0",
        "host-gate-0",
        "host-gate-1",
        "host-facts-0",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    // The status column is the same width on every machine.
    let (a, b) = (
        visual.debug_bounds("host-gate-0").unwrap(),
        visual.debug_bounds("host-gate-1").unwrap(),
    );
    assert_eq!(a.size.width, b.size.width);
    assert_eq!(a.origin.x, b.origin.x);
    // Metrics and the queue wait behind Details.
    assert!(visual.debug_bounds("queue-row-0-0").is_none());
    assert!(visual.debug_bounds("host-metric-0-0").is_none());
    click(&mut visual, "host-details-0");
    for selector in ["host-detail-0", "host-metric-0-0", "queue-row-0-0"] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    effects(&mut commands);
    click(&mut visual, "queue-cancel-0-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::HostAction(HostAction::Cancel { id, .. })) if id == "q_wait"
    )));
    // The queue id lives behind Copy ID.
    click(&mut visual, "queue-0-0-more");
    click(&mut visual, "queue-copy-id-0-0");
    let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("q_wait"));
    // (GPUI keeps debug bounds from earlier frames, so state says it.)
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.pages.menu, None, "the menu closes");
    });
    // Details closes again.
    click(&mut visual, "host-details-0");
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.pages.hosts_open.is_empty());
    });
}

fn jobs_listed() -> serde_json::Value {
    json!({"jobs":[
        {"id":"job","name":"Prune","enabled":true,"trigger":{"kind":"interval","everyMinutes":240},
         "action":{"kind":"shell","shell":{"command":"git worktree prune"}},
         "lastRun":{"startedAt":1,"finishedAt":2,"status":"ok"}}
    ]})
}

#[gpui::test]
fn jobs_remove_lives_in_the_overflow_menu_behind_a_confirm(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 900., |this, window, cx| {
        this.open_feature(Screen::Jobs, window, cx)
    });
    request_state(&workspace, &mut visual, Request::Jobs, 1, jobs_listed());
    assert!(visual.debug_bounds("jobs-new").is_some());
    assert!(visual.debug_bounds("job-pause-0").is_some());
    // No Remove on the row until the menu opens.
    assert!(visual.debug_bounds("job-remove-0").is_none());
    click(&mut visual, "job-0-more");
    assert!(visual.debug_bounds("job-0-menu").is_some());
    effects(&mut commands);
    click(&mut visual, "job-remove-0");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::JobAction(_)))),
        "the menu item only asks"
    );
    // Keep backs out; asking again and confirming removes.
    click(&mut visual, "job-keep-0");
    // (GPUI keeps debug bounds from earlier frames, so state says it.)
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.job_confirm_remove, None, "Keep backs out");
        assert_eq!(this.extras.pages.menu, None);
    });
    click(&mut visual, "job-0-more");
    click(&mut visual, "job-remove-0");
    click(&mut visual, "job-remove-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::JobAction(JobAction::Remove(id))) if id == "job"
    )));
}

#[gpui::test]
fn new_job_opens_a_session_prefilled_to_ask_for_one(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 900., |this, window, cx| {
        this.open_feature(Screen::Jobs, window, cx)
    });
    request_state(&workspace, &mut visual, Request::Jobs, 1, jobs_listed());
    click(&mut visual, "jobs-new");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.new_session, "New job opens the New session form");
        assert_eq!(
            this.prompt.read(cx).value().to_string(),
            super::super::jobs::NEW_JOB_PROMPT
        );
    });
}

#[gpui::test]
fn settings_fold_chat_into_appearance(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 2400., |this, window, cx| {
        this.show_screen(Screen::Settings, window, cx)
    });
    assert!(visual.debug_bounds("settings-nav-Appearance").is_some());
    assert!(visual.debug_bounds("settings-nav-Chat").is_none());
    for selector in ["theme-picker", "setting-merge-turn", "setting-clock"] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    // The theme grid fills the card: four equal tiles a row.
    let dark = visual.debug_bounds("theme-Dark").unwrap();
    let tokyo = visual.debug_bounds("theme-Tokyo Night").unwrap();
    assert_eq!(dark.size.width, tokyo.size.width);
    assert_eq!(dark.origin.y, tokyo.origin.y);
    // j / k step through the v2 rail and never land on Chat.
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings_section = settings::SettingsSection::Agents;
            this.step_settings_section(1, cx);
            assert_eq!(this.settings_section, settings::SettingsSection::Keyboard);
        })
    });
}

#[gpui::test]
fn remote_on_a_hub_without_it_is_a_friendly_unavailable_state(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 1200., |this, window, cx| {
        this.show_screen(Screen::Settings, window, cx);
        this.settings_section = settings::SettingsSection::Remote;
    });
    request_error(
        &workspace,
        &mut visual,
        "remote",
        Request::Remote,
        "Unknown fixture method: remote.status",
    );
    assert!(visual.debug_bounds("remote-unavailable").is_some());
    // Pairing has nothing to offer there.
    assert!(visual.debug_bounds("setting-pair-phone").is_none());
    assert!(visual.debug_bounds("setting-paired-devices").is_none());
}

#[gpui::test]
fn about_says_an_offline_check_plainly(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 1200., |this, window, cx| {
        this.show_screen(Screen::Settings, window, cx);
        this.settings_section = settings::SettingsSection::About;
    });
    request_error(
        &workspace,
        &mut visual,
        "updates",
        Request::Updates,
        "error sending request for url (https://api.github.com/repos/x/y/releases)",
    );
    assert!(visual.debug_bounds("update-status").is_some());
    assert!(visual.debug_bounds("update-offline").is_some());
    assert!(visual.debug_bounds("check-updates").is_some());
}

/// R4.5: one page template. Every page, New session included, carries the
/// back link, and its title starts at one left edge.
#[gpui::test]
fn every_page_shares_one_header_and_left_edge(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 900., |_, _, _| {});
    let mut edge = None;
    for (screen, new_session) in [
        (Screen::Projects, false),
        (Screen::Settings, false),
        (Screen::Recent, false),
        (Screen::Jobs, false),
        (Screen::Hosts, false),
        (Screen::Scratch, false),
        (Screen::Setup, false),
        (Screen::Fleet, false),
        (Screen::Changes, false),
        (Screen::Conversation, true),
    ] {
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.screen = screen;
                this.new_session = new_session;
                cx.notify();
            })
        });
        visual.run_until_parked();
        let title = visual.debug_bounds("page-title").unwrap();
        let back = visual
            .debug_bounds("feature-back")
            .unwrap_or_else(|| panic!("{screen:?} has the back link"));
        assert!(back.bottom() <= title.top(), "{screen:?}: back sits above");
        let left = f32::from(title.left());
        let edge = *edge.get_or_insert(left);
        assert!(
            (left - edge).abs() <= 1.,
            "{screen:?} (new session {new_session}) title at {left}, others at {edge}"
        );
    }
}

/// R4.5: table actions sit in fixed slots. Pause and Resume share a width,
/// so Run now lines up on every job row.
#[gpui::test]
fn job_actions_hold_fixed_slots(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    open_v2(&workspace, &mut visual, 900., |this, window, cx| {
        this.open_feature(Screen::Jobs, window, cx)
    });
    request_state(
        &workspace,
        &mut visual,
        Request::Jobs,
        1,
        json!({"jobs":[
            {"id":"a","name":"On","enabled":true,"trigger":{"kind":"interval","everyMinutes":240},
             "action":{"kind":"shell","shell":{"command":"true"}}},
            {"id":"b","name":"Off","enabled":false,"trigger":{"kind":"interval","everyMinutes":240},
             "action":{"kind":"shell","shell":{"command":"true"}}}
        ]}),
    );
    let pause = visual.debug_bounds("job-pause-0").unwrap();
    let resume = visual.debug_bounds("job-pause-1").unwrap();
    assert_eq!(pause.size.width, resume.size.width, "Pause and Resume");
    assert_eq!(pause.left(), resume.left());
    let run0 = visual.debug_bounds("job-run-0").unwrap();
    let run1 = visual.debug_bounds("job-run-1").unwrap();
    assert_eq!(run0.left(), run1.left(), "Run now aligns");
}
