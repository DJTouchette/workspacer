//! Retained broker assertions against actual actor queues and admission state.
use super::*;

fn event(mail: &mut Mailbox) -> Event {
    mail.events
        .try_recv()
        .expect("expected enqueued event")
        .event
        .unwrap()
}
#[tokio::test]
async fn matching_fanout_topic_changes_and_disconnect_close_actual_queues() {
    let mut core = core();
    let mut a = peer(&mut core, 1, Identity::host("a"), 8);
    let mut b = peer(&mut core, 2, Identity::host("b"), 8);
    let mut silent = peer(&mut core, 3, Identity::host("silent"), 8);
    topics(&mut core, 1, "subscribe", &["agent.*", "agent.*"]);
    assert_eq!(take(&mut a).topics, vec!["agent.*"]);
    topics(&mut core, 2, "subscribe", &["*"]);
    take(&mut b);
    core.publish(Event::new("git.changed", "fixture", Value::Null));
    assert!(a.events.try_recv().is_err());
    assert_eq!(event(&mut b).topic, "git.changed");
    core.publish(Event::new("agent.spawned", "fixture", json!({"id":"one"})));
    let first = event(&mut a);
    assert_eq!(event(&mut b), first);
    assert!(first.id.starts_with("ev-"));
    assert!(chrono::DateTime::parse_from_rfc3339(&first.time).is_ok());
    assert!(
        silent.events.try_recv().is_err(),
        "empty topic list must match nothing"
    );
    topics(&mut core, 1, "unsubscribe", &["agent.*"]);
    assert!(take(&mut a).topics.is_empty());
    core.publish(Event::new("agent.spawned", "fixture", Value::Null));
    assert!(a.events.try_recv().is_err());
    event(&mut b);
    topics(&mut core, 1, "subscribe", &["agent.*"]);
    assert_eq!(take(&mut a).topics, vec!["agent.*"]);
    core.disconnect(2);
    core.disconnect(2); // repeat removal is inert
    assert!(!core.peers.contains_key(&2));
    assert_eq!(
        b.events.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    );
    core.publish(Event::new("agent.done", "fixture", Value::Null));
    assert_eq!(event(&mut a).topic, "agent.done");
    assert!(silent.events.try_recv().is_err());
}

#[tokio::test]
async fn subscription_admission_is_bounded_before_processing_large_batches() {
    let mut core = core();
    let mut subscriber = peer(&mut core, 1, scoped(Scope::View, vec![]), 8);
    let huge: Vec<_> = (0..100_000).map(|i| format!("topic.{i}")).collect();
    let started = Instant::now();
    core.frame(
        1,
        Frame {
            topics: huge.clone(),
            ..Frame::op("subscribe")
        },
    );
    assert!(!take(&mut subscriber).error.is_empty());
    assert!(core.peers[&1].topics.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "oversized frame processing stalled broker"
    );
    for chunk in huge[..768].chunks(256) {
        core.frame(
            1,
            Frame {
                topics: chunk.to_vec(),
                ..Frame::op("subscribe")
            },
        );
        take(&mut subscriber);
    }
    assert_eq!(core.peers[&1].topics.len(), 512);
    assert_eq!(core.peers[&1].topics[511], "topic.511");
    core.frame(
        1,
        Frame {
            topics: huge.clone(),
            ..Frame::op("unsubscribe")
        },
    );
    assert!(!take(&mut subscriber).error.is_empty());
    assert_eq!(
        core.peers[&1].topics.len(),
        512,
        "rejected frame changed membership"
    );
    for chunk in huge[..512].chunks(256) {
        core.frame(
            1,
            Frame {
                topics: chunk.to_vec(),
                ..Frame::op("unsubscribe")
            },
        );
        take(&mut subscriber);
    }
    assert!(core.peers[&1].topics.is_empty());
    topics(&mut core, 1, "subscribe", &["topic.0", "topic.0"]);
    assert_eq!(take(&mut subscriber).topics, vec!["topic.0"]);
}

#[tokio::test]
async fn denied_and_closed_consumers_cost_no_event_slot_or_desync_bookkeeping() {
    let mut core = core();
    let mut denied = peer(&mut core, 1, scoped(Scope::View, vec![]), 1);
    let mut closed = peer(&mut core, 2, Identity::host("closed"), 1);
    for (id, mail) in [(1, &mut denied), (2, &mut closed)] {
        topics(&mut core, id, "subscribe", &["*"]);
        take(mail);
    }
    core.peers[&2].closed.send_replace(true);
    for _ in 0..200 {
        core.publish(Event::new(
            "pty.bytes.SECRET-42",
            "fixture",
            json!({"bytes":"secret"}),
        ));
    }
    for id in [1, 2] {
        assert_eq!(
            core.peers[&id].events.capacity(),
            1,
            "unauthorized event occupied a queue slot"
        );
        assert!(
            core.peers[&id].desynced.lock().unwrap().is_empty(),
            "unauthorized stream left a named repair record"
        );
    }
    core.publish(Event::new(
        "agent.state_changed",
        "fixture",
        json!({"id":"public"}),
    ));
    assert_eq!(
        event(&mut denied).topic,
        "agent.state_changed",
        "admission must not mute allowed events"
    );
    assert!(closed.events.try_recv().is_err());
}

#[tokio::test]
async fn slow_consumer_drops_are_nonblocking_bounded_and_stream_specific() {
    let mut core = core();
    let mut slow = peer(&mut core, 1, Identity::host("slow"), 1);
    let mut fast = peer(&mut core, 2, Identity::host("fast"), 1);
    for (id, mail) in [(1, &mut slow), (2, &mut fast)] {
        topics(&mut core, id, "subscribe", &["*"]);
        take(mail);
    }
    core.publish(Event::new("pty.bytes.filler", "fixture", Value::Null));
    assert!(
        core.peers[&1].desynced.lock().unwrap().is_empty(),
        "delivered stream marked corrupt"
    );
    event(&mut fast);
    let started = Instant::now();
    for i in 0..1000 {
        core.publish(Event::new(
            format!("pty.bytes.session-{i}"),
            "fixture",
            Value::Null,
        ));
        assert_eq!(event(&mut fast).topic, format!("pty.bytes.session-{i}"));
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "full queue stalled publisher or healthy consumer"
    );
    assert_eq!(core.peers[&1].desynced.lock().unwrap().len(), 64);
    assert!(core.peers[&2].desynced.lock().unwrap().is_empty());
    let repairs = std::mem::take(&mut *core.peers[&1].desynced.lock().unwrap());
    assert!(repairs.contains("pty.bytes.session-0"));
    assert!(std::mem::take(&mut *core.peers[&1].desynced.lock().unwrap()).is_empty());
    assert_eq!(event(&mut slow).topic, "pty.bytes.filler");
    core.publish(Event::new("agent.spawned", "fixture", Value::Null));
    core.publish(Event::new("agent.spawned", "fixture", Value::Null));
    assert!(
        core.peers[&1].desynced.lock().unwrap().is_empty(),
        "discrete drop invented a stream repair"
    );
    assert_eq!(event(&mut slow).topic, "agent.spawned");
}

#[tokio::test]
async fn envelope_stamp_preserves_existing_identity_and_typed_json_payload() {
    let mut core = core();
    let mut subscriber = peer(&mut core, 1, Identity::host("fixture"), 4);
    topics(&mut core, 1, "subscribe", &["*"]);
    take(&mut subscriber);
    let supplied = Event {
        id: "upstream-event".into(),
        time: "2026-01-02T03:04:05Z".into(),
        hub: "peer".into(),
        ..Event::new(
            "agent.done",
            "fixture",
            json!({"number":9007199254740993_u64}),
        )
    };
    core.publish(supplied.clone());
    assert_eq!(event(&mut subscriber), supplied);
    core.publish(Event {
        time: "0001-01-01T00:00:00Z".into(),
        ..Event::new("agent.done", "fixture", Value::Null)
    });
    let stamped = event(&mut subscriber);
    assert_eq!(stamped.id, "ev-1");
    assert_ne!(stamped.time, "0001-01-01T00:00:00Z");
    let encoded = serde_json::to_value(&supplied).unwrap();
    assert_eq!(encoded["data"]["number"], 9007199254740993_u64);
    assert_eq!(encoded["type"], "agent.done");
    assert_eq!(encoded["hub"], "peer");
    assert!(
        serde_json::to_value(Event::default())
            .unwrap()
            .get("data")
            .is_none()
    );
    assert!(
        serde_json::to_value(Event::default())
            .unwrap()
            .get("hub")
            .is_none()
    );
}
