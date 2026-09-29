use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use workspacer_hub::services::{
    task_store::OwnerLookup,
    wakes::{Capture, Delivery, Inventory, Wakes},
};
struct Fixture {
    wakes: Arc<Wakes>,
    rows: Arc<Mutex<BTreeMap<String, Value>>>,
    replies: Arc<Mutex<BTreeMap<String, Value>>>,
    sent: Arc<Mutex<Vec<(String, String)>>>,
    fail: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    capture_fail: Arc<AtomicBool>,
    fail_recipients: Arc<Mutex<BTreeSet<String>>>,
    fail_escalations: Arc<AtomicBool>,
}
impl Fixture {
    fn new(manager: bool) -> Self {
        Self::with_history(manager, None)
    }
    fn with_history(
        manager: bool,
        history: Option<Arc<workspacer_hub::services::task_store::TaskStore>>,
    ) -> Self {
        let rows = Arc::new(Mutex::new(BTreeMap::from([
            (
                "parent".into(),
                json!({"sessionId":"parent","ambientState":"idle","status":"active","isWakeTarget":manager,"lastActivity":0}),
            ),
            (
                "one".into(),
                json!({"sessionId":"one","parentSessionId":"parent","ambientState":"streaming","status":"active","cwd":"/project","lastActivity":1}),
            ),
            (
                "two".into(),
                json!({"sessionId":"two","parentSessionId":"parent","ambientState":"streaming","status":"active","cwd":"/project","lastActivity":1}),
            ),
        ])));
        let replies = Arc::new(Mutex::new(BTreeMap::from([
            (
                "one".into(),
                json!({"items":[{"kind":"user_message","text":"task"},{"kind":"assistant_text","text":"one done"}]}),
            ),
            (
                "two".into(),
                json!({"items":[{"kind":"user_message","text":"task"},{"kind":"assistant_text","text":"two done"}]}),
            ),
        ])));
        let lookup_rows = rows.clone();
        let lookup: OwnerLookup = Arc::new(move |id| lookup_rows.lock().unwrap().get(id).cloned());
        let list_rows = rows.clone();
        let list: Inventory =
            Arc::new(move || list_rows.lock().unwrap().values().cloned().collect());
        let resume = Arc::new(AtomicBool::new(false));
        let resume_capture = resume.clone();
        let capture_rows = rows.clone();
        let capture_fail = Arc::new(AtomicBool::new(false));
        let failing_capture = capture_fail.clone();
        let capture_replies = replies.clone();
        let capture: Capture = Arc::new(move |id| {
            let (rows, replies, resume) = (
                capture_rows.clone(),
                capture_replies.clone(),
                resume_capture.clone(),
            );
            let failing = failing_capture.clone();
            Box::pin(async move {
                anyhow::ensure!(!failing.load(Ordering::SeqCst), "conversation unavailable");
                if resume.load(Ordering::SeqCst) {
                    rows.lock().unwrap().get_mut(&id).unwrap()["ambientState"] = "streaming".into();
                }
                Ok(replies.lock().unwrap().get(&id).unwrap().clone())
            })
        });
        let sent = Arc::new(Mutex::new(Vec::new()));
        let deliver_sent = sent.clone();
        let fail = Arc::new(AtomicBool::new(false));
        let delivery_fail = fail.clone();
        let fail_recipients = Arc::new(Mutex::new(BTreeSet::new()));
        let recipients = fail_recipients.clone();
        let fail_escalations = Arc::new(AtomicBool::new(false));
        let escalations = fail_escalations.clone();
        let delivery: Delivery = Arc::new(move |id, message, _| {
            let (sent, fail) = (deliver_sent.clone(), delivery_fail.clone());
            let recipients = recipients.clone();
            let escalations = escalations.clone();
            Box::pin(async move {
                anyhow::ensure!(
                    !escalations.load(Ordering::SeqCst)
                        || !message.contains("[fleet] Worker escalated"),
                    "escalation delivery refused"
                );
                anyhow::ensure!(
                    !fail.load(Ordering::SeqCst) && !recipients.lock().unwrap().contains(&id),
                    "explicit delivery refusal"
                );
                sent.lock().unwrap().push((id, message));
                Ok(())
            })
        });
        let wakes = Wakes::new(lookup, list, capture, delivery, history, None);
        wakes.prime(&rows.lock().unwrap().values().cloned().collect::<Vec<_>>());
        Self {
            wakes,
            rows,
            replies,
            sent,
            fail,
            resume,
            capture_fail,
            fail_recipients,
            fail_escalations,
        }
    }
    fn mode(&self, id: &str, mode: &str, now: i64) {
        let row = {
            let mut rows = self.rows.lock().unwrap();
            let row = rows.get_mut(id).unwrap();
            row["ambientState"] = mode.into();
            row["lastActivity"] = now.into();
            row.clone()
        };
        self.wakes.observe(&row, now);
    }
}
#[tokio::test]
async fn finishes_coalesce_and_confirmed_signatures_deduplicate() -> Result<()> {
    let f = Fixture::new(false);
    f.mode("one", "idle", 100);
    f.mode("two", "idle", 200);
    f.wakes.tick(1599).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    f.wakes.tick(1600).await?;
    let sent = f.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("session:one") && sent[0].1.contains("session:two"));
    assert!(sent[0].1.contains("continue your own task"));
    f.mode("one", "streaming", 2000);
    f.mode("one", "idle", 2100);
    f.wakes.tick(3600).await?;
    assert_eq!(f.sent.lock().unwrap().len(), 1);
    Ok(())
}
#[tokio::test]
async fn resumed_or_never_tasked_workers_do_not_emit_obsolete_finishes() -> Result<()> {
    let f = Fixture::new(true);
    f.mode("one", "idle", 100);
    f.resume.store(true, Ordering::SeqCst);
    f.wakes.tick(1600).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    let f = Fixture::new(true);
    f.replies.lock().unwrap().insert(
        "one".into(),
        json!({"items":[{"kind":"assistant_text","text":"ready"}]}),
    );
    f.mode("one", "idle", 100);
    f.wakes.tick(1600).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    Ok(())
}
#[tokio::test]
async fn explicit_delivery_failure_does_not_book_a_signature() -> Result<()> {
    let f = Fixture::new(true);
    f.fail.store(true, Ordering::SeqCst);
    f.mode("one", "idle", 100);
    assert!(f.wakes.tick(1600).await.is_err());
    f.fail.store(false, Ordering::SeqCst);
    f.mode("one", "streaming", 2000);
    f.mode("one", "idle", 2100);
    f.wakes.tick(3600).await?;
    assert_eq!(f.sent.lock().unwrap().len(), 1);
    Ok(())
}
#[tokio::test]
async fn cleared_block_generation_cannot_fire_a_new_blocks_timer_early() -> Result<()> {
    let f = Fixture::new(true);
    f.mode("one", "waiting_approval", 100);
    f.mode("one", "idle", 1000);
    f.wakes.tick(22000).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    f.mode("one", "waiting_input", 30000);
    f.wakes.tick(50000).await?;
    f.mode("one", "idle", 50500);
    f.mode("one", "waiting_input", 51000);
    f.wakes.tick(51500).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    f.wakes.tick(71000).await?;
    f.wakes.tick(72500).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("question") && sent[0].1.starts_with("[supervisor]"));
    Ok(())
}
#[tokio::test]
async fn escalation_and_completion_are_distinct_batches() -> Result<()> {
    let f = Fixture::new(true);
    for id in ["one", "two"] {
        f.rows.lock().unwrap().get_mut(id).unwrap()["resultSchema"] =
            json!({"type":"object","required":["commit"]});
    }
    f.replies.lock().unwrap().get_mut("one").unwrap()["items"][1]["text"] =
        "Done\n```wks-result\n{\"commit\":\"abc\"}\n```".into();
    f.replies.lock().unwrap().get_mut("two").unwrap()["items"][1]["text"]="Blocked\n```wks-escalation\n{\"type\":\"worker-escalation\",\"status\":\"blocked\",\"reason\":\"permission\",\"requiredAuthorityOrDecision\":\"approval\",\"changed\":false,\"nextAction\":\"ask\"}\n```".into();
    f.mode("one", "idle", 100);
    f.mode("two", "idle", 200);
    f.wakes.tick(1600).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[0].1.contains("Worker escalated") && !sent[0].1.contains("Structured result MISSING")
    );
    assert!(sent[1].1.contains("Structured result") && sent[1].1.contains("abc"));
    Ok(())
}
#[test]
fn account_overage_alone_never_mislabels_a_success_as_failure() {
    let row = json!({"statusLine":{"overageOutOfCredits":true}});
    assert!(workspacer_hub::services::wakes::failure_reason(&row, "Tests pass").is_none());
    assert_eq!(
        workspacer_hub::services::wakes::failure_reason(
            &row,
            "⚠️ Error: API Error: 529 Overloaded"
        )
        .unwrap(),
        "API Error: 529 Overloaded"
    );
    assert!(
        workspacer_hub::services::wakes::failure_reason(
            &row,
            "⚠️ Error: You're out of usage credits"
        )
        .unwrap()
        .starts_with("out of credits (overage disabled)")
    );
}

#[tokio::test]
async fn continuous_approval_question_block_keeps_one_survival_window() -> Result<()> {
    let f = Fixture::new(true);
    f.mode("one", "waiting_approval", 100);
    f.mode("one", "waiting_input", 1000);
    f.wakes.tick(20100).await?;
    f.wakes.tick(21600).await?;
    assert_eq!(f.sent.lock().unwrap().len(), 1);
    assert!(f.sent.lock().unwrap()[0].1.contains("question"));
    f.mode("one", "waiting_approval", 30000);
    f.wakes.tick(60000).await?;
    f.wakes.tick(61500).await?;
    assert_eq!(
        f.sent.lock().unwrap().len(),
        1,
        "one continuous block must not re-broadcast"
    );
    f.mode("one", "streaming", 62000);
    f.mode("one", "waiting_approval", 63000);
    f.wakes.tick(83000).await?;
    f.wakes.tick(84500).await?;
    assert_eq!(
        f.sent.lock().unwrap().len(),
        2,
        "a new block after a clear wakes again"
    );
    Ok(())
}
#[tokio::test]
async fn surviving_block_resolves_new_managers_and_excludes_ended_recipients() -> Result<()> {
    let f = Fixture::new(true);
    f.mode("one", "waiting_approval", 100);
    f.rows.lock().unwrap().insert("new-manager".into(),json!({"sessionId":"new-manager","isWakeTarget":true,"status":"active","ambientState":"idle"}));
    f.rows.lock().unwrap().get_mut("parent").unwrap()["status"] = json!("ended");
    f.wakes.tick(20100).await?;
    f.wakes.tick(21600).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "new-manager");
    Ok(())
}
#[tokio::test]
async fn stale_or_forgotten_blocks_are_reverified_at_both_delivery_boundaries() -> Result<()> {
    for during_coalesce in [false, true] {
        for change in ["idle", "ended", "missing", "forgotten"] {
            let f = Fixture::new(true);
            f.mode("one", "waiting_approval", 100);
            if during_coalesce {
                f.wakes.tick(20100).await?;
            }
            match change {
                "idle" => {
                    f.rows.lock().unwrap().get_mut("one").unwrap()["ambientState"] = json!("idle")
                }
                "ended" => {
                    f.rows.lock().unwrap().get_mut("one").unwrap()["status"] = json!("ended")
                }
                "missing" => {
                    f.rows.lock().unwrap().remove("one");
                }
                _ => f.wakes.forget("one"),
            }
            f.wakes.tick(20100).await?;
            f.wakes.tick(21600).await?;
            assert!(
                f.sent.lock().unwrap().is_empty(),
                "{change}, during_coalesce={during_coalesce}"
            );
        }
    }
    Ok(())
}
#[tokio::test]
async fn block_fanout_is_failure_isolated_and_ordinary_recipients_are_direct_parents() -> Result<()>
{
    let f = Fixture::new(false);
    f.rows.lock().unwrap().insert("zz-manager".into(),json!({"sessionId":"zz-manager","isWakeTarget":true,"status":"active","ambientState":"idle"}));
    f.rows.lock().unwrap().insert(
        "unrelated".into(),
        json!({"sessionId":"unrelated","status":"active","ambientState":"idle"}),
    );
    f.fail_recipients.lock().unwrap().insert("parent".into());
    f.mode("one", "waiting_approval", 100);
    f.mode("two", "waiting_input", 200);
    f.wakes.tick(20200).await?;
    assert!(f.wakes.tick(21700).await.is_err());
    let sent = f.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "zz-manager");
    assert!(sent[0].1.contains("session:one") && sent[0].1.contains("session:two"));
    f.fail_recipients.lock().unwrap().clear();
    f.mode("one", "streaming", 30000);
    f.mode("one", "waiting_input", 31000);
    f.wakes.tick(51000).await?;
    f.wakes.tick(52500).await?;
    let sent = f.sent.lock().unwrap();
    assert!(
        sent.iter()
            .any(|(id, msg)| id == "parent" && msg.contains("continue your own work"))
    );
    assert!(!sent.iter().any(|(id, _)| id == "unrelated"));
    Ok(())
}
#[tokio::test]
async fn boot_priming_and_first_sightings_preserve_opposite_block_and_finish_rules() -> Result<()> {
    let f = Fixture::new(true);
    let row = {
        let mut rows = f.rows.lock().unwrap();
        let row = rows.get_mut("one").unwrap();
        row["ambientState"] = json!("waiting_input");
        row.clone()
    };
    f.wakes.prime(&[row.clone()]);
    f.wakes.observe(&row, 100);
    f.wakes.tick(20100).await?;
    f.wakes.tick(21600).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    let blocked = json!({"sessionId":"new-block","ambientState":"waiting_input","status":"active","cwd":"/new-project"});
    f.rows
        .lock()
        .unwrap()
        .insert("new-block".into(), blocked.clone());
    f.wakes.observe(&blocked, 30000);
    let idle = json!({"sessionId":"new-idle","parentSessionId":"parent","ambientState":"idle","status":"active"});
    f.rows
        .lock()
        .unwrap()
        .insert("new-idle".into(), idle.clone());
    f.wakes.observe(&idle, 30000);
    f.wakes.tick(50000).await?;
    f.wakes.tick(51500).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("new-project") && sent[0].1.contains("session:new-block"));
    assert!(!sent[0].1.contains("new-idle"));
    Ok(())
}
#[tokio::test]
async fn only_working_edges_finish_and_an_unparented_finish_arms_nothing() -> Result<()> {
    for previous in [
        "thinking",
        "streaming",
        "background",
        "waiting_approval",
        "waiting_input",
        "idle",
        "",
    ] {
        let f = Fixture::new(true);
        let row = {
            let mut rows = f.rows.lock().unwrap();
            let row = rows.get_mut("one").unwrap();
            row["ambientState"] = json!(previous);
            row.clone()
        };
        f.wakes.prime(&[row]);
        f.mode("one", "idle", 100);
        f.wakes.tick(1600).await?;
        assert_eq!(
            f.sent.lock().unwrap().len(),
            usize::from(matches!(previous, "thinking" | "streaming" | "background")),
            "{previous}"
        );
    }
    let f = Fixture::new(true);
    f.rows.lock().unwrap().get_mut("one").unwrap()["parentSessionId"] = json!("");
    f.mode("one", "idle", 100);
    f.wakes.tick(1600).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    Ok(())
}
#[tokio::test]
async fn finish_windows_are_per_parent_and_capture_the_latest_reply() -> Result<()> {
    let f = Fixture::new(true);
    f.rows.lock().unwrap().insert(
        "second".into(),
        json!({"sessionId":"second","ambientState":"idle","status":"active"}),
    );
    f.rows.lock().unwrap().get_mut("two").unwrap()["parentSessionId"] = json!("second");
    f.mode("one", "idle", 100);
    f.mode("two", "idle", 100);
    f.replies.lock().unwrap().get_mut("one").unwrap()["items"][1]["text"] =
        json!("late final report");
    f.wakes.tick(1600).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().any(|(id, msg)| id == "parent"
        && msg.contains("late final report")
        && !msg.contains("session:two")));
    assert!(sent.iter().any(|(id, msg)| id == "second"
        && msg.contains("session:two")
        && !msg.contains("session:one")));
    Ok(())
}
#[tokio::test]
async fn unreachable_conversation_still_wakes_and_parent_death_suppresses_delivery() -> Result<()> {
    let f = Fixture::new(true);
    f.capture_fail.store(true, Ordering::SeqCst);
    f.mode("one", "idle", 100);
    f.wakes.tick(1600).await?;
    assert_eq!(f.sent.lock().unwrap().len(), 1);
    let f = Fixture::new(true);
    f.mode("one", "idle", 100);
    f.rows.lock().unwrap().get_mut("parent").unwrap()["status"] = json!("ended");
    f.wakes.tick(1600).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    Ok(())
}
#[tokio::test]
async fn backstop_obeys_three_minute_grace_parent_activity_and_busy_state() -> Result<()> {
    for manager in [false, true] {
        for guard in ["recover", "acted", "busy", "working-child", "unknown-time"] {
            let f = Fixture::new(manager);
            {
                let mut rows = f.rows.lock().unwrap();
                rows.get_mut("one").unwrap()["ambientState"] = json!("idle");
                rows.get_mut("one").unwrap()["lastActivity"] = json!(100);
                match guard {
                    "acted" => rows.get_mut("parent").unwrap()["lastActivity"] = json!(100),
                    "busy" => rows.get_mut("parent").unwrap()["ambientState"] = json!("streaming"),
                    "working-child" => {
                        rows.get_mut("one").unwrap()["ambientState"] = json!("streaming")
                    }
                    "unknown-time" => rows.get_mut("one").unwrap()["lastActivity"] = json!(0),
                    _ => (),
                }
            }
            f.wakes.backstop(180100);
            f.wakes.tick(180100).await?;
            assert!(f.sent.lock().unwrap().is_empty(), "grace boundary {guard}");
            f.wakes.backstop(180101);
            f.wakes.tick(180101).await?;
            assert_eq!(
                f.sent.lock().unwrap().len(),
                usize::from(guard == "recover"),
                "manager={manager}/{guard}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn manager_finish_wake_uses_committed_workflow_state_and_current_owner() -> Result<()> {
    use workspacer_hub::services::task_store::TaskStore;
    for (manager, owner, expected) in [
        (true, "parent", true),
        (false, "parent", false),
        (true, "other", false),
    ] {
        let dir = tempfile::tempdir()?;
        let history = Arc::new(TaskStore::open(dir.path().join("history.json"))?);
        history.transaction(|state| {
            state.tasks.push(json!({"taskId":"task","ownerSessionId":owner,"projectCwd":"/project",
                "attempts":[{"dispatchId":"dispatch","sessionId":"one","metrics":{}}],
                "workflow":{"hash":"pin","definition":{"id":"flow","name":"Pinned flow","revision":1,
                    "steps":[{"id":"implement","kind":"implementation"}]},
                    "steps":[{"id":"implement","sessionId":"one","state":"dispatched"}]}}));
            Ok(())
        })?;
        let f = Fixture::with_history(manager, Some(history.clone()));
        f.rows.lock().unwrap().get_mut("one").unwrap()["resultSchema"] = json!({"type":"object"});
        f.replies.lock().unwrap().insert(
            "one".into(),
            json!({"items":[
            {"kind":"user_message","text":"task"},
            {"kind":"assistant_text","text":"Done\n```wks-result\n{\"ok\":true}\n```"}]}),
        );
        f.mode("one", "idle", 100);
        f.wakes.tick(1600).await?;
        let persisted = history.task("task")?.unwrap();
        assert_eq!(persisted["workflow"]["steps"][0]["state"], "completed");
        let sent = f.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0]
                .1
                .contains("All configured steps returned valid result contracts"),
            expected
        );
        assert!(!sent[0].1.contains("Step implement is dispatched"));
    }
    Ok(())
}

#[tokio::test]
async fn a_failed_escalation_group_does_not_silence_completed_workers() -> Result<()> {
    let f = Fixture::new(true);
    f.fail_escalations.store(true, Ordering::SeqCst);
    f.replies.lock().unwrap().insert("one".into(), json!({"items":[
        {"kind":"user_message","text":"task"},
        {"kind":"assistant_text","text":"Need approval\n```wks-escalation\n{\"type\":\"worker-escalation\",\"status\":\"blocked\",\"reason\":\"needs write authority\",\"requiredAuthorityOrDecision\":\"approve publish\",\"changed\":false,\"nextAction\":\"review then decide\"}\n```"}]}));
    f.mode("one", "idle", 100);
    f.mode("two", "idle", 110);
    assert!(f.wakes.tick(1610).await.is_err());
    {
        let sent = f.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.contains("session:two"));
        assert!(!sent[0].1.contains("session:one"));
    }
    f.fail_escalations.store(false, Ordering::SeqCst);
    for id in ["one", "two"] {
        f.mode(id, "streaming", 2000);
        f.mode(id, "idle", 2100);
    }
    f.wakes.tick(3600).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        2,
        "only the rejected escalation may need another wake"
    );
    assert!(sent[1].1.contains("[fleet] Worker escalated"));
    assert!(sent[1].1.contains("session:one"));
    assert!(!sent[1].1.contains("session:two"));
    Ok(())
}

#[tokio::test]
async fn failure_marker_is_separate_from_valid_terminal_contract_and_outcome() -> Result<()> {
    use workspacer_hub::services::task_store::TaskStore;
    for escalation in [false, true] {
        let dir = tempfile::tempdir()?;
        let history = Arc::new(TaskStore::open(dir.path().join("history.json"))?);
        history.transaction(|state| {
            state.tasks.push(
                json!({"taskId":"task","ownerSessionId":"parent","projectCwd":"/project",
                "attempts":[{"dispatchId":"dispatch","sessionId":"one","metrics":{}}],
                "workflow":{"hash":"pin","definition":{"id":"flow","name":"Flow","revision":1,
                    "steps":[{"id":"work","kind":"implementation"}]},
                    "steps":[{"id":"work","sessionId":"one","state":"dispatched"}]}}),
            );
            Ok(())
        })?;
        let f = Fixture::with_history(true, Some(history.clone()));
        f.rows.lock().unwrap().get_mut("one").unwrap()["resultSchema"] = json!({"type":"object"});
        let (fence, body) = if escalation {
            (
                "wks-escalation",
                json!({"type":"worker-escalation","status":"blocked","reason":"No authority","requiredAuthorityOrDecision":"Approve writing","changed":false,"nextAction":"Review decision"}),
            )
        } else {
            (
                "wks-result",
                json!({"ok":false,"reason":"Provider refused"}),
            )
        };
        f.replies.lock().unwrap().insert("one".into(),json!({"items":[{"kind":"user_message","text":"task"},
            {"kind":"assistant_text","text":format!("⚠️ Error: Provider refused\n```{fence}\n{body}\n```")}]}));
        f.mode("one", "idle", 100);
        f.wakes.tick(1600).await?;
        let task = history.task("task")?.unwrap();
        let step = &task["workflow"]["steps"][0];
        assert_eq!(
            task["attempts"][0]["resultContract"],
            if escalation { "escalated" } else { "valid" }
        );
        assert_eq!(
            step["state"],
            if escalation { "blocked" } else { "completed" }
        );
        if escalation {
            assert_eq!(
                serde_json::from_str::<Value>(step["outcome"].as_str().unwrap())?,
                body
            );
        } else {
            assert_eq!(step["outcome"], body);
        }
        let sent = f.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.contains("FAILED: Provider refused"));
        assert_eq!(sent[0].1.contains("[fleet] Worker escalated"), escalation);
        assert!(!sent[0].1.contains("Structured result MISSING"));
    }
    Ok(())
}

#[tokio::test]
async fn blocked_manager_excludes_self_and_unparented_blocks_coalesce_by_recipient() -> Result<()> {
    let f = Fixture::new(true);
    f.rows.lock().unwrap().insert(
        "other".into(),
        json!({"sessionId":"other","isWakeTarget":true,"status":"active","ambientState":"idle"}),
    );
    f.mode("parent", "waiting_input", 100);
    f.wakes.tick(20100).await?;
    f.wakes.tick(21600).await?;
    let sent = f.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "other");
    assert!(sent[0].1.contains("(session:parent, question)"));
    let f = Fixture::new(true);
    {
        let mut rows = f.rows.lock().unwrap();
        for id in ["one", "two"] {
            rows.get_mut(id)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("parentSessionId");
        }
        rows.get_mut("one").unwrap()["cwd"] = "/work/unlabelled-worker".into();
    }
    f.mode("one", "waiting_approval", 100);
    f.mode("two", "waiting_input", 100);
    f.wakes.tick(20099).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    f.wakes.tick(20100).await?;
    f.wakes.tick(21599).await?;
    assert!(f.sent.lock().unwrap().is_empty());
    f.wakes.tick(21600).await?;
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .1
            .contains("- unlabelled-worker (session:one, approval)")
    );
    assert!(sent[0].1.contains("(session:two, question)"));
    assert!(sent[0].1.ends_with(
        "Run a /supervise pass: gather the context and notify me with a recommendation."
    ));
    Ok(())
}

#[tokio::test]
async fn terminal_wire_distinguishes_stopped_failed_long_and_invalid_escalation() -> Result<()> {
    for (reply, stopped, full, failed, invalid) in [
        ("short reply".to_owned(), false, false, false, false),
        ("long report ".repeat(80), false, true, false, false),
        ("short reply".to_owned(), true, false, false, false),
        (
            "⚠️ Error: Credit balance is too low".to_owned(),
            false,
            false,
            true,
            false,
        ),
        (
            "Cannot publish\n```wks-escalation\n{}\n```".to_owned(),
            false,
            true,
            false,
            true,
        ),
        (
            "I cannot publish with this authority".to_owned(),
            false,
            false,
            false,
            false,
        ),
    ] {
        let f = Fixture::new(true);
        f.replies.lock().unwrap().insert("one".into(),json!({"items":[{"kind":"user_message","text":"task"},{"kind":"assistant_text","text":reply}]}));
        if stopped {
            f.rows.lock().unwrap().get_mut("one").unwrap()["status"] = "ended".into();
        }
        f.mode("one", "idle", 100);
        f.wakes.tick(1600).await?;
        let sent = f.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let message = &sent[0].1;
        assert_eq!(message.contains("Full final message —"), full, "{message}");
        assert_eq!(message.contains("stopped/killed"), stopped, "{message}");
        assert_eq!(
            message.starts_with("[fleet] Worker FAILED"),
            failed,
            "{message}"
        );
        assert_eq!(
            message.contains("Worker escalation INVALID"),
            invalid,
            "{message}"
        );
        if failed {
            assert!(message.contains("then type /logout and then /login"));
        }
        assert!(!message.starts_with("[fleet] Worker escalated"));
    }
    Ok(())
}
