use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use workspacer_hub::services::progress::{Deliver, Progress, flatten_note};

fn fixture(
    fail: bool,
) -> (
    Progress,
    Arc<Mutex<Vec<(String, String)>>>,
    Arc<Mutex<Vec<Value>>>,
) {
    let rows = Arc::new(Mutex::new(vec![
        json!({"sessionId":"worker","parentSessionId":"parent","cwd":"/project/leaf"}),
        json!({"sessionId":"parent","status":"idle"}),
    ]));
    let lookup = rows.clone();
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let sent = deliveries.clone();
    let deliver: Deliver = Arc::new(move |id, text| {
        let sent = sent.clone();
        Box::pin(async move {
            sent.lock().unwrap().push((id, text));
            anyhow::ensure!(!fail, "delivery unavailable");
            Ok(())
        })
    });
    (
        Progress::new(
            Arc::new(move |id| {
                lookup
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|row| row["sessionId"] == id)
                    .cloned()
            }),
            deliver,
        ),
        deliveries,
        rows,
    )
}
#[tokio::test]
async fn progress_only_reaches_current_parent_and_never_claims_completion() {
    let (reports, sent, rows) = fixture(false);
    let response=reports.report(json!({"callerSessionId":"worker","note":"\nphase\tlanded\u{feff}now ","needsDecision":true}),0).await.unwrap();
    assert_eq!(response, json!({"deliveredTo":"parent"}));
    let message = sent.lock().unwrap()[0].1.clone();
    assert!(message.starts_with(
        "[fleet] Progress update from a worker — it is STILL RUNNING; this is NOT a completion:"
    ));
    assert!(message.contains(
        "leaf (session:worker, cwd /project/leaf) — NEEDS A DECISION — reports: phase landed now"
    ));
    rows.lock().unwrap()[1]["status"] = "ended".into();
    assert!(
        reports
            .report(json!({"callerSessionId":"worker","note":"second"}), 60000)
            .await
            .unwrap_err()
            .to_string()
            .contains("has ended")
    );
    assert_eq!(sent.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn failures_consume_budget_and_duplicates_and_rate_limits_refuse_out_loud() {
    let (reports, sent, _) = fixture(true);
    assert!(
        reports
            .report(json!({"callerSessionId":"worker","note":"first"}), 0)
            .await
            .is_err()
    );
    assert!(
        reports
            .report(json!({"callerSessionId":"worker","note":"first"}), 60000)
            .await
            .unwrap_err()
            .to_string()
            .contains("same note")
    );
    assert!(
        reports
            .report(json!({"callerSessionId":"worker","note":"second"}), 1000)
            .await
            .unwrap_err()
            .to_string()
            .contains("one per 60s")
    );
    assert_eq!(sent.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn total_report_and_utf16_note_caps_are_explicit() {
    let (reports, sent, _) = fixture(false);
    assert!(
        reports
            .report(
                json!({"callerSessionId":"worker","note":"😀".repeat(251)}),
                0
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("502 characters")
    );
    for n in 0..20 {
        reports
            .report(
                json!({"callerSessionId":"worker","note":format!("phase {n}")}),
                n * 60000,
            )
            .await
            .unwrap();
    }
    assert!(
        reports
            .report(
                json!({"callerSessionId":"worker","note":"one more"}),
                20 * 60000
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("20 progress updates")
    );
    assert_eq!(sent.lock().unwrap().len(), 20);
    assert_eq!(flatten_note("a\u{0085}b"), "a\u{0085}b");
}
