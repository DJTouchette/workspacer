//! The Hosts page and the launch host picker (`docs/host-work-queue.md`):
//! cards for this hub and its peers, cancel of queued work, the limits
//! editor, and a launch that goes through `queue.submit` instead of
//! starting an agent here.
use super::*;
use serde_json::json;
use wks_native::features::{HostAction, Request};

fn resources(host: &str, open: bool) -> serde_json::Value {
    json!({"host": host, "sampledAt": 1,
        "cpu": {"cores": 8, "usagePct": 23.5, "pressureAvg10": 3.1},
        "memory": {"totalMb": 31800, "availableMb": 12288, "pressureAvg10": null},
        "io": {"pressureAvg10": 0.4},
        "disk": {"freeMb": 80000},
        "queue": {"agentSlots": 4, "agentsRunning": 1, "shellSlots": 2, "shellRunning": 0,
            "queued": 1, "gate": {"open": open,
                "reasons": if open { json!([]) } else { json!(["memory pressure 34.0 >= 25.0"]) }}}})
}

fn targets() -> serde_json::Value {
    json!({"targets": [
        {"host": "local", "local": true, "connected": true, "dispatch": true,
         "resources": resources("studio", true), "cwds": [], "eligible": true, "reasons": []},
        {"host": "gpu-box", "connected": true, "dispatch": true, "resources": resources("gpu-box", false),
         "cwds": ["/srv/api", "/srv/models"], "eligible": true, "reasons": []},
        {"host": "laptop", "connected": false, "dispatch": true, "resources": null,
         "cwds": [], "eligible": false, "reasons": ["not connected"]}
    ], "best": "local"})
}

fn snapshot() -> serde_json::Value {
    json!({
        "resources": resources("studio", true),
        "list": {"host": "studio", "items": [
            {"id": "q_run", "kind": "spawn", "state": "running", "label": "Profile",
             "spawn": {"provider": "claude", "cwd": "/work/hub"}, "sessionId": "s-1",
             "enqueuedAt": 1, "startedAt": 2},
            {"id": "q_wait", "kind": "spawn", "state": "queued", "label": "Nightly", "priority": 2,
             "spawn": {"provider": "claude", "cwd": "/work/api"}, "enqueuedAt": 3,
             "waitReason": "no free agent slot"}
        ], "peers": [
            {"peer": "gpu-box", "ok": true, "items": [
                {"id": "q_peer", "kind": "spawn", "state": "queued", "label": "Eval",
                 "spawn": {"provider": "claude", "cwd": "/srv/models"}, "enqueuedAt": 4}]}
        ]},
        "targets": targets(),
        "config": {"maxAgents": 4, "maxShell": 2, "maxCpuPressure": 60,
            "maxMemoryPressure": 25, "minAvailableMemoryMb": 1024, "minFreeDiskMb": 2048}
    })
}

#[gpui::test]
fn hosts_page_lists_each_host_and_cancels_queued_work(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    visual.run_until_parked();
    click(&mut visual, "nav-hosts");
    let sent = effects(&mut commands);
    assert!(sent.iter().any(|c| matches!(c, Command::WatchHosts(true))));
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::Request(Request::Hosts))),
        "opening the page reads the hosts"
    );
    // Tall enough for every card without scrolling.
    visual.simulate_resize(size(px(1000.), px(1800.)));
    request_state(&workspace, &mut visual, Request::Hosts, 1, snapshot());
    for selector in [
        "host-card-0",
        "host-card-1",
        "host-card-2",
        "queue-row-0-0",
        "queue-row-0-1",
        "queue-row-1-0",
        "host-facts-0",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    // Running work has no Cancel; queued work does, on its own host.
    assert!(visual.debug_bounds("queue-cancel-0-0").is_none());
    click(&mut visual, "queue-cancel-1-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::HostAction(HostAction::Cancel { id, host }))
            if id == "q_peer" && host == "gpu-box"
    )));
    click(&mut visual, "queue-cancel-0-1");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Request(Request::HostAction(HostAction::Cancel { id, host }))
            if id == "q_wait" && host == "local"
    )));

    // The limits editor sends only what changed and shows a refusal inline.
    click(&mut visual, "host-limits");
    assert!(visual.debug_bounds("host-limits-editor").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.limit_inputs[0].update(cx, |input, cx| input.set_value("6", window, cx));
        })
    });
    click(&mut visual, "host-limits-save");
    let patch = effects(&mut commands)
        .into_iter()
        .find_map(|c| match c {
            Command::Request(Request::HostAction(HostAction::SetConfig(patch))) => Some(patch),
            _ => None,
        })
        .expect("save sends queue.setConfig");
    assert_eq!(patch, json!({"maxAgents": 6}));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                "host-action",
                wks_native::features::RequestState {
                    request: Request::HostAction(HostAction::SetConfig(patch.clone())),
                    number: 9,
                    loading: false,
                    error: Some("queue.setConfig requires the server owner".into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("host-limits-error").is_some());
    assert!(visual.debug_bounds("host-limits-editor").is_some());
}

#[gpui::test]
fn an_older_hub_says_the_queue_is_not_supported(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Hosts, window, cx);
        })
    });
    let missing = |m: &str| json!({"error": format!("no provider for {m}")});
    request_state(
        &workspace,
        &mut visual,
        Request::Hosts,
        1,
        json!({"resources": missing("host.resources"), "list": missing("queue.list"),
            "targets": missing("queue.targets"), "config": missing("queue.config")}),
    );
    assert!(visual.debug_bounds("hosts-unsupported").is_some());
    assert!(visual.debug_bounds("host-card-0").is_none());
}

#[gpui::test]
fn a_launch_on_a_peer_submits_to_its_queue_and_opens_hosts(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1000.), px(1100.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
            this.update_view(Arc::new(view), window, cx);
            this.show_new_session(window, cx);
        })
    });
    visual.run_until_parked();
    // No queue.targets answer (an older hub): no picker.
    assert!(visual.debug_bounds("launch-hosts").is_none());
    request_state(
        &workspace,
        &mut visual,
        Request::QueueTargets { provider: None },
        1,
        targets(),
    );
    assert!(visual.debug_bounds("launch-hosts").is_some());
    // The laptop is offline: listed, disabled, with its reason.
    assert!(visual.debug_bounds("launch-host-reason-0").is_some());
    click(&mut visual, "launch-host-2");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.extras.launch_host,
            wks_native::hosts::HostChoice::Local
        )
    });

    click(&mut visual, "launch-host-1");
    // A peer's folders replace the local project picker.
    assert!(visual.debug_bounds("launch-peer-folders").is_some());
    click(&mut visual, "launch-host-cwd-1");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.prompt
                .update(cx, |input, cx| input.set_value("Run the evals", window, cx));
        })
    });
    effects(&mut commands);
    click(&mut visual, "launch-start");
    let sent = effects(&mut commands);
    assert!(
        !sent.iter().any(|c| matches!(c, Command::Create(_))),
        "a peer launch never spawns here"
    );
    let params = sent
        .into_iter()
        .find_map(|c| match c {
            Command::Request(Request::QueueSubmit(params)) => Some(params),
            _ => None,
        })
        .expect("a peer launch submits to the queue");
    assert_eq!(params["host"], "gpu-box");
    assert_eq!(params["kind"], "spawn");
    assert_eq!(params["spawn"]["cwd"], "/srv/models");
    assert_eq!(params["spawn"]["prompt"], "Run the evals");
    assert_eq!(params["spawn"]["provider"], "claude");

    // Where it landed is confirmed on the Hosts page.
    request_state(
        &workspace,
        &mut visual,
        Request::QueueSubmit(params),
        2,
        json!({"host": "gpu-box", "item": {"id": "q_new", "state": "queued", "kind": "spawn"}}),
    );
    workspace.read_with(&visual, |this, _| {
        assert!(!this.new_session);
        assert_eq!(this.screen, Screen::Hosts);
        assert_eq!(this.extras.notice, "Queued on gpu-box as q_new.");
    });
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::Hosts)))
    );
}

#[gpui::test]
fn this_machine_still_spawns_directly_unless_queued(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1000.), px(1100.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
            this.update_view(Arc::new(view), window, cx);
            this.show_new_session(window, cx);
        })
    });
    request_state(
        &workspace,
        &mut visual,
        Request::QueueTargets { provider: None },
        1,
        targets(),
    );
    effects(&mut commands);
    click(&mut visual, "launch-start");
    let sent = effects(&mut commands);
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::Create(r) if r.cwd == "/work/alpha"))
    );
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, Command::Request(Request::QueueSubmit(_))))
    );

    // "Queue if busy" sends the same launch through this hub's queue.
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1000.), px(1100.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
            this.update_view(Arc::new(view), window, cx);
            this.show_new_session(window, cx);
            this.prompt
                .update(cx, |input, cx| input.set_value("Tidy up", window, cx));
        })
    });
    request_state(
        &workspace,
        &mut visual,
        Request::QueueTargets { provider: None },
        1,
        targets(),
    );
    click(&mut visual, "launch-queue");
    effects(&mut commands);
    click(&mut visual, "launch-start");
    let sent = effects(&mut commands);
    assert!(!sent.iter().any(|c| matches!(c, Command::Create(_))));
    assert!(sent.iter().any(|c| matches!(
        c,
        Command::Request(Request::QueueSubmit(p))
            if p["host"] == "local" && p["spawn"]["cwd"] == "/work/alpha"
    )));
}
