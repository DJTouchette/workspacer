use super::model::{self, Plan};
use super::spawn_watch::{self, Signal};
use super::*;
use serde_json::json;

/// A cwd that is absolute on the host running the tests (`/abs` is not on Windows).
const ABS: &str = if cfg!(windows) { "C:\\abs" } else { "/abs" };

fn item(id: &str, kind: &str, priority: i64, enqueued_at: i64, state: &str) -> Item {
    Item {
        id: id.into(),
        kind: kind.into(),
        priority,
        enqueued_at,
        state: state.into(),
        ..Default::default()
    }
}
fn config(agents: u64, shell: u64) -> QueueConfig {
    QueueConfig {
        max_agents: agents,
        max_shell: shell,
        ..Default::default()
    }
}

#[test]
fn order_is_priority_then_fifo() {
    let items = vec![
        item("late-low", "shell", 0, 30, QUEUED),
        item("early-low", "shell", 0, 10, QUEUED),
        item("high", "shell", 5, 99, QUEUED),
        item("negative", "shell", -1, 1, QUEUED),
    ];
    let plan = model::plan(&items, &config(4, 10), &[]);
    assert_eq!(plan.start, ["high", "early-low", "late-low", "negative"]);
    assert!(plan.waits.is_empty());
}

#[test]
fn per_kind_slots_hold_back_only_their_own_kind() {
    let items = vec![
        item("running-shell", "shell", 0, 1, RUNNING),
        item("shell-head", "shell", 9, 2, QUEUED),
        item("agent", "spawn", 0, 3, QUEUED),
        item("agent-2", "spawn", 0, 4, QUEUED),
    ];
    let plan = model::plan(&items, &config(1, 1), &[]);
    assert_eq!(plan.start, ["agent"]);
    assert_eq!(
        plan.waits,
        [
            (
                "agent-2".to_owned(),
                "no free agent slot (1/1 running)".to_owned()
            ),
            (
                "shell-head".to_owned(),
                "no free shell slot (1/1 running)".to_owned()
            ),
        ]
        .into()
    );
    // Zero slots pauses a kind.
    let plan = model::plan(&items, &config(0, 5), &[]);
    assert_eq!(plan.start, ["shell-head"]);
    assert_eq!(plan.waits.len(), 2);
}

#[test]
fn a_closed_gate_holds_everything_with_its_reasons() {
    let items = vec![
        item("a", "shell", 0, 1, QUEUED),
        item("b", "spawn", 0, 2, QUEUED),
    ];
    let gate = vec![
        "memory pressure 34.0 >= 25.0".to_owned(),
        "free disk 1 MB < 2048 MB".into(),
    ];
    let Plan { start, waits } = model::plan(&items, &config(4, 2), &gate);
    assert!(start.is_empty());
    assert_eq!(
        waits["a"],
        "resource gate closed: memory pressure 34.0 >= 25.0; free disk 1 MB < 2048 MB"
    );
    assert_eq!(waits.len(), 2);
}

#[test]
fn restart_turns_running_into_unknown_and_keeps_queued() {
    let mut items = vec![
        item("run", "spawn", 0, 1, RUNNING),
        item("wait", "shell", 0, 2, QUEUED),
        item("old", "shell", 0, 0, DONE),
    ];
    assert!(model::recover(&mut items, 77));
    assert_eq!(items[0].state, UNKNOWN);
    assert_eq!(items[0].detail, "hub restarted while running");
    assert_eq!(items[0].finished_at, 77);
    assert_eq!(items[1].state, QUEUED);
    assert_eq!(items[2].state, DONE);
    assert!(!model::recover(&mut items, 78));
}

#[test]
fn retention_keeps_newest_terminal_and_every_live_item() {
    let mut items: Vec<Item> = (0..250)
        .map(|n| {
            let mut i = item(&format!("t{n}"), "shell", 0, n, DONE);
            i.finished_at = n;
            i
        })
        .collect();
    items.push(item("q", "shell", 0, 0, QUEUED));
    items.push(item("r", "spawn", 0, 0, RUNNING));
    model::retain(&mut items);
    assert_eq!(items.len(), model::RETAIN_TERMINAL + 2);
    assert!(items.iter().any(|i| i.id == "q") && items.iter().any(|i| i.id == "r"));
    assert!(items.iter().any(|i| i.id == "t249"));
    assert!(!items.iter().any(|i| i.id == "t49"));
    assert!(items.iter().any(|i| i.id == "t50"));
}

#[test]
fn listing_puts_running_then_queue_order_then_newest_terminal() {
    let mut done_old = item("done-old", "shell", 0, 1, DONE);
    done_old.finished_at = 5;
    let mut done_new = item("done-new", "shell", 0, 2, FAILED);
    done_new.finished_at = 9;
    let items = vec![
        done_old,
        item("q-low", "shell", 0, 1, QUEUED),
        done_new,
        item("run", "spawn", 0, 3, RUNNING),
        item("q-high", "shell", 3, 9, QUEUED),
    ];
    let ids: Vec<_> = model::listing(&items)
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(ids, ["run", "q-high", "q-low", "done-new", "done-old"]);
}

#[test]
fn submissions_are_shape_checked() {
    let shell = model::parse_submit(&json!({"kind":"shell","shell":{"command":"make test\nmore"}}))
        .unwrap();
    assert_eq!(shell.label, "make test");
    assert_eq!(shell.shell.as_ref().unwrap().timeout_sec, 3600);
    assert_eq!(shell.source.kind, "prompt");
    assert_eq!(shell.state, QUEUED);
    let spawn = model::parse_submit(&json!({"kind":"spawn","priority":3,"label":"triage","spawn":{"cwd":ABS,"prompt":"go","provider":"codex"},"source":{"kind":"job","jobId":"j1"}})).unwrap();
    assert_eq!(spawn.priority, 3);
    assert_eq!(spawn.spawn.as_ref().unwrap().provider, "codex");
    assert_eq!(spawn.source.job_id, "j1");
    for bad in [
        json!({"kind":"spawn","spawn":{"cwd":"relative","prompt":"go"}}),
        json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"  "}}),
        json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go"},"shell":{"command":"x"}}),
        json!({"kind":"shell","shell":{"command":"x","timeoutSec":0}}),
        json!({"kind":"shell","shell":{"command":"x","timeoutSec":86401}}),
        json!({"kind":"shell","shell":{"command":"x","skipPermissions":true}}),
        json!({"kind":"shell","shell":{"command":""}}),
        json!({"kind":"shell","shell":{"command":"x","cwd":"rel"}}),
        json!({"kind":"shell","shell":{"command":"x"},"priority":1.5}),
        json!({"kind":"shell","shell":{"command":"x"},"source":{"kind":"admin"}}),
        json!({"kind":"shell","shell":{"command":"x"},"extra":1}),
        json!({"kind":"other"}),
        json!("shell"),
    ] {
        assert!(model::parse_submit(&bad).is_err(), "{bad}");
    }
}

#[test]
fn config_patches_are_strict_and_section_reads_are_tolerant() {
    assert_eq!(
        QueueConfig::validate_patch(&json!({"maxShell":1,"maxCpuPressure":75.5})).unwrap(),
        json!({"maxShell":1,"maxCpuPressure":75.5})
    );
    for bad in [
        json!({"maxShell":1.5}),
        json!({"maxShell":-1}),
        json!({"maxCpuPressure":101}),
        json!({"maxShel":1}),
        json!({"maxShell":"2"}),
        json!([1]),
    ] {
        assert!(QueueConfig::validate_patch(&bad).is_err(), "{bad}");
    }
    let read = QueueConfig::from_section(&json!({"maxAgents":2,"maxShell":"x","minFreeDiskMb":-5}));
    assert_eq!(read.max_agents, 2);
    assert_eq!(read.max_shell, 2, "invalid value falls back to the default");
    assert_eq!(read.min_free_disk_mb, 2048);
    assert_eq!(
        QueueConfig::from_section(&Value::Null),
        QueueConfig::default()
    );
    assert_eq!(
        QueueConfig::default().wire(),
        json!({"maxAgents":4,"maxShell":2,"maxCpuPressure":60.0,"maxMemoryPressure":25.0,"minAvailableMemoryMb":1024,"minFreeDiskMb":2048})
    );
}

#[test]
fn the_shared_config_defaults_match_the_queue_defaults() {
    let defaults = crate::services::config::defaults();
    assert_eq!(
        QueueConfig::from_section(&defaults["queue"]).wire(),
        QueueConfig::default().wire()
    );
    assert_eq!(
        defaults["queue"].as_object().unwrap().len(),
        6,
        "every queue.* key has a shared default"
    );
}

#[test]
fn wire_items_never_carry_the_submitter_credential() {
    let mut item = item("q_1", "spawn", 0, 1, QUEUED);
    item.submitter = "fingerprint".into();
    let wire = item.wire();
    assert!(wire.get("submitter").is_none());
    assert_eq!(wire["exitCode"], Value::Null);
    assert_eq!(
        json!(item)["submitter"],
        "fingerprint",
        "persisted form keeps it"
    );
}

#[test]
fn spawn_signals_release_only_after_work_or_on_a_person() {
    let row = |ambient: &str| json!({"sessionId":"s","ambientState":ambient,"status":"active"});
    assert_eq!(spawn_watch::classify(&row("idle"), false), Signal::Wait);
    assert_eq!(
        spawn_watch::classify(&json!({"sessionId":"s"}), false),
        Signal::Wait
    );
    for working in ["streaming", "thinking", "background"] {
        assert_eq!(spawn_watch::classify(&row(working), false), Signal::Working);
    }
    assert_eq!(
        spawn_watch::classify(&row("idle"), true),
        Signal::Release {
            failed: false,
            detail: "turn finished".into()
        }
    );
    for waiting in ["waiting_input", "waiting_approval"] {
        assert!(matches!(
            spawn_watch::classify(&row(waiting), false),
            Signal::Release { failed: false, .. }
        ));
    }
    assert_eq!(
        spawn_watch::classify(&json!({"status":"ended","mode":"stopped"}), false),
        Signal::Release {
            failed: false,
            detail: "session ended".into()
        }
    );
    assert!(matches!(
        spawn_watch::classify(&json!({"mode":"stopped","error":"provider crashed"}), true),
        Signal::Release { failed: true, .. }
    ));
}

#[test]
fn rechecks_are_a_bounded_backstop() {
    let idle = json!({"ambientState":"idle","status":"active"});
    let (signal, strikes) = spawn_watch::recheck(Some(&idle), false, 0);
    assert_eq!((signal, strikes), (Signal::Wait, 1));
    let (signal, _) = spawn_watch::recheck(Some(&idle), false, strikes);
    assert!(matches!(signal, Signal::Release { failed: false, .. }));
    // Working resets the count.
    let busy = json!({"ambientState":"streaming"});
    assert_eq!(
        spawn_watch::recheck(Some(&busy), false, 1),
        (Signal::Working, 0)
    );
    // A session that is no longer reported is released after two checks.
    assert_eq!(spawn_watch::recheck(None, true, 0).0, Signal::Wait);
    assert!(matches!(
        spawn_watch::recheck(Some(&Value::Null), true, 1).0,
        Signal::Release { .. }
    ));
    // A seen-working agent that went idle is released at once.
    assert!(matches!(
        spawn_watch::recheck(Some(&idle), true, 0).0,
        Signal::Release { .. }
    ));
}

fn caller(owner: bool, federated: bool, scope: &str) -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        federated,
        connection_id: 1,
        authenticated_host: owner && !federated,
        trusted: scope == "operator",
        scope: scope.into(),
        plugin_id: String::new(),
        token_id: if federated {
            "peer-fp".into()
        } else {
            "local".into()
        },
    }
}

#[tokio::test]
async fn submit_stamps_identity_and_cancel_honours_the_submitter() {
    let directory = tempfile::tempdir().unwrap();
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let path = directory.path().join("queue.json");
    let open = || {
        WorkQueue::open(
            path.clone(),
            hub.handle(),
            Arc::new(Monitor::new(directory.path().into())),
            Limits::Memory(Mutex::new(json!({}))),
            Wiring {
                host_fingerprint: "host-fp".into(),
                execution: None,
            },
        )
        .unwrap()
    };
    let queue = open();
    let cwd = directory.path().to_str().unwrap();
    let spawn = json!({"kind":"spawn","spawn":{"cwd":cwd,"prompt":"go"},"source":{"kind":"remote","origin":"spoof"}});
    // A local caller cannot claim to be remote.
    let local = queue
        .submit_local(&caller(true, false, "operator"), &spawn)
        .unwrap();
    assert_eq!(local.source.kind, "prompt");
    assert_eq!(local.source.origin, "");
    // A federated operator is always remote, whatever it says.
    let mut claimed = spawn.clone();
    claimed["source"] = json!({"kind":"job","jobId":"nightly","origin":"laptop"});
    let peer = caller(false, true, "operator");
    let remote = queue.submit_local(&peer, &claimed).unwrap();
    assert_eq!(remote.source.kind, "remote");
    assert_eq!(remote.source.job_id, "nightly");
    assert_eq!(remote.source.origin, "laptop");
    assert_eq!(remote.submitter, "peer-fp");
    // Shell is host code: a federated operator token is refused...
    let shell = json!({"kind":"shell","shell":{"command":"true"}});
    let refused = queue.submit_local(&peer, &shell).unwrap_err().to_string();
    assert!(refused.contains("host credential"), "{refused}");
    // ...and so is full access for a spawn.
    let mut full = spawn.clone();
    full["spawn"]["fullAccess"] = json!(true);
    assert!(queue.submit_local(&peer, &full).is_err());
    // A peer paired with this hub's own host credential may submit both.
    let mut paired = peer.clone();
    paired.token_id = "host-fp".into();
    assert!(queue.is_host_credential(&paired));
    let hosted = queue.submit_local(&paired, &shell).unwrap();
    assert_eq!(hosted.source.kind, "remote");
    assert_eq!(hosted.submitter, "host-fp");
    queue.submit_local(&paired, &full).unwrap();
    // The fingerprint alone is not enough: it must be a federated operator.
    let mut local_host = paired.clone();
    local_host.federated = false;
    assert!(!queue.is_host_credential(&local_host));
    let mut viewer = paired.clone();
    viewer.trusted = false;
    viewer.scope = "view".into();
    assert!(!queue.is_host_credential(&viewer));
    assert!(queue.submit_local(&viewer, &shell).is_err());
    assert!(
        queue
            .submit_local(&caller(true, false, "operator"), &full)
            .is_ok()
    );
    assert!(
        queue
            .submit_local(&caller(false, false, "view"), &spawn)
            .is_err()
    );
    // A missing cwd is refused on this host.
    let mut missing = spawn.clone();
    missing["spawn"]["cwd"] = json!("/definitely/not/here/q");
    assert!(
        queue
            .submit_local(&caller(true, false, "operator"), &missing)
            .is_err()
    );
    // Only the submitting peer (or the owner) cancels a remote item.
    let mut other = peer.clone();
    other.token_id = "other-fp".into();
    assert!(queue.cancel(&other, &remote.id).is_err());
    assert!(queue.cancel(&peer, &local.id).is_err());
    assert_eq!(queue.cancel(&peer, &remote.id).unwrap().state, CANCELLED);
    assert!(
        queue
            .cancel(&caller(true, false, "operator"), &remote.id)
            .is_err(),
        "already cancelled"
    );
    // Routing lives in the async dispatcher; a peer never routes onward.
    let mut elsewhere = shell.clone();
    elsewhere["host"] = json!("laptop");
    assert!(
        queue
            .call(
                &caller(true, false, "operator"),
                "queue.submit",
                elsewhere.clone()
            )
            .is_err()
    );
    let onward = queue
        .clone()
        .dispatch(paired.clone(), "queue.submit", elsewhere)
        .await
        .unwrap_err()
        .to_string();
    assert!(onward.contains("second hop"), "{onward}");
    let unknown = queue
        .clone()
        .dispatch(
            caller(true, false, "operator"),
            "queue.submit",
            json!({"host":"nowhere","kind":"shell","shell":{"command":"true"}}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("unknown federation peer"), "{unknown}");
    // Every stored change bumps updatedAt.
    let cancelled = queue
        .snapshot()
        .into_iter()
        .find(|i| i.id == remote.id)
        .unwrap();
    assert!(cancelled.updated_at >= cancelled.enqueued_at && cancelled.updated_at > 0);
    // Persisted 0600 and reloaded as-is.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    drop(queue);
    let reopened = open();
    let ids: Vec<_> = reopened
        .snapshot()
        .into_iter()
        .map(|i| (i.id, i.state))
        .collect();
    assert!(ids.contains(&(local.id.clone(), QUEUED.to_owned())));
    assert!(ids.contains(&(remote.id.clone(), CANCELLED.to_owned())));
    // A damaged file is set aside, not overwritten, and the queue starts empty.
    drop(reopened);
    std::fs::write(&path, b"{broken").unwrap();
    assert!(open().snapshot().is_empty());
    assert!(
        std::fs::read_dir(directory.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("damaged"))
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn resources_report_slots_and_the_gate_from_the_latest_sample() {
    let directory = tempfile::tempdir().unwrap();
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let monitor = Arc::new(Monitor::new(directory.path().into()));
    let queue = WorkQueue::open(
        directory.path().join("queue.json"),
        hub.handle(),
        monitor.clone(),
        Limits::Memory(Mutex::new(json!({}))),
        Wiring::default(),
    )
    .unwrap();
    let open = queue.resources();
    assert_eq!(open["queue"]["gate"], json!({"open":true,"reasons":[]}));
    assert_eq!(open["queue"]["agentSlots"], 4);
    assert!(
        open["cpu"]["pressureAvg10"].is_null(),
        "unsampled is unknown"
    );
    monitor.set_latest(host_resources::Sample {
        memory_pressure: Some(34.0),
        ..Default::default()
    });
    let closed = queue.resources();
    assert_eq!(closed["queue"]["gate"]["open"], false);
    assert_eq!(
        closed["queue"]["gate"]["reasons"],
        json!(["memory pressure 34.0 >= 25.0"])
    );
    let owner = caller(true, false, "operator");
    assert!(
        queue
            .set_config(
                &caller(false, false, "operator"),
                &json!({"maxMemoryPressure":50})
            )
            .is_err()
    );
    queue
        .set_config(&owner, &json!({"maxMemoryPressure":50}))
        .unwrap();
    assert_eq!(queue.resources()["queue"]["gate"]["open"], true);
    assert_eq!(queue.config().max_memory_pressure, 50.0);
    hub.shutdown().unwrap();
}

#[test]
fn spawn_parameters_carry_only_ordinary_launch_fields() {
    let mut item = model::parse_submit(&json!({"kind":"spawn","label":"nightly","spawn":{"cwd":ABS,"prompt":"go","model":"m","permissionMode":"plan"},"source":{"kind":"job","jobId":"j1"}})).unwrap();
    assert_eq!(
        spawn_params(&item),
        json!({"cwd":ABS,"label":"nightly","message":"go","model":"m","permissionMode":"plan","jobId":"j1"})
    );
    item.source.kind = "prompt".into();
    assert!(spawn_params(&item).get("jobId").is_none());
    // permissionMode passes through unchanged (bypass included), and the
    // context window rides along, so a queued launch matches a direct one.
    let item = model::parse_submit(&json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go","permissionMode":"bypassPermissions","contextWindow":1000000}})).unwrap();
    let params = spawn_params(&item);
    assert_eq!(params["permissionMode"], "bypassPermissions");
    assert_eq!(params["contextWindow"], 1_000_000);
    assert!(params.get("skipPermissions").is_none());
    // fullAccess maps like an inline job's agents.jobFullAccess.
    let item = model::parse_submit(&json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go","permissionMode":"plan","fullAccess":true}})).unwrap();
    let params = spawn_params(&item);
    assert_eq!(params["skipPermissions"], true);
    assert!(params.get("permissionMode").is_none());
    for bad in [
        json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go","contextWindow":0}}),
        json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go","contextWindow":"big"}}),
        json!({"kind":"spawn","spawn":{"cwd":ABS,"prompt":"go","fullAccess":"yes"}}),
    ] {
        assert!(model::parse_submit(&bad).is_err(), "{bad}");
    }
}
