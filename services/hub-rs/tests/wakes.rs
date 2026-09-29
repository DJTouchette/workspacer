use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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
}
impl Fixture {
    fn new(manager: bool) -> Self {
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
        let capture_replies = replies.clone();
        let capture: Capture = Arc::new(move |id| {
            let (rows, replies, resume) = (
                capture_rows.clone(),
                capture_replies.clone(),
                resume_capture.clone(),
            );
            Box::pin(async move {
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
        let delivery: Delivery = Arc::new(move |id, message, _| {
            let (sent, fail) = (deliver_sent.clone(), delivery_fail.clone());
            Box::pin(async move {
                anyhow::ensure!(!fail.load(Ordering::SeqCst), "explicit delivery refusal");
                sent.lock().unwrap().push((id, message));
                Ok(())
            })
        });
        let wakes = Wakes::new(lookup, list, capture, delivery, None, None);
        wakes.prime(&rows.lock().unwrap().values().cloned().collect::<Vec<_>>());
        Self {
            wakes,
            rows,
            replies,
            sent,
            fail,
            resume,
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
