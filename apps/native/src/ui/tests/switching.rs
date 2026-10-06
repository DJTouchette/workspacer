//! Switching the chat between a parent and its provider-native children.
use super::*;

fn replies(texts: &[String]) -> Transcript {
    let mut transcript = Transcript::default();
    transcript.snapshot(ConversationSnapshot {
        seq: texts.len() as u64,
        first_seq: 1,
        items: texts
            .iter()
            .map(|text| Item {
                kind: "assistant_text".into(),
                text: text.clone(),
                ..Default::default()
            })
            .collect(),
    });
    transcript
}

fn family() -> View {
    let mut view = state("a");
    Arc::make_mut(&mut view.sessions)[0].subagents = serde_json::json!([
        {"id": "t1", "description": "Long audit", "status": "complete", "completedAt": 2000},
        {"id": "t2", "description": "Short check", "status": "complete", "completedAt": 2000}
    ]);
    view
}

fn viewing(agent: &str, transcript: Transcript) -> View {
    View {
        child: Some(wks_native::controller::ChildTarget {
            parent: "a".into(),
            agent: agent.into(),
        }),
        transcript,
        ..family()
    }
}

/// Every transcript numbers its rows from zero. A row keyed only by the
/// selected session would adopt the previous transcript's parsed text view and
/// keep painting that content until the deferred reparse (200ms later in a
/// real window; never here, where the clock does not advance).
#[gpui::test]
fn switching_paints_the_new_transcript_in_its_first_frame(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut show = |view: View| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        visual
            .debug_bounds("chat-content-column")
            .expect("the transcript row painted")
            .size
            .height
    };
    let long = (1..=40)
        .map(|n| format!("Paragraph {n} of the child's audit."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let parent = View {
        transcript: replies(&["Parent reply.".into()]),
        ..family()
    };
    let short = show(parent.clone());
    let tall = show(viewing("t1", replies(&[long])));
    assert!(
        tall > short * 10.,
        "the child painted the parent's text: {tall:?} vs {short:?}"
    );
    let sibling = show(viewing("t2", replies(&["Short check passed.".into()])));
    assert!(
        sibling < tall / 10.,
        "a sibling painted the previous child's text: {sibling:?} vs {tall:?}"
    );
    let back = show(viewing(
        "t1",
        replies(&[(1..=40)
            .map(|n| format!("Line {n}."))
            .collect::<Vec<_>>()
            .join("\n\n")]),
    ));
    assert!(back > short * 10., "{back:?} vs {short:?}");
    assert!(
        show(parent) < tall / 10.,
        "the parent painted its child's text"
    );
}

fn dense(count: usize, tag: &str) -> Transcript {
    let mut transcript = Transcript::default();
    let items = wks_native::harness::dense_items(count, tag)
        .into_iter()
        .map(|item| serde_json::from_value::<Item>(item).unwrap())
        .collect::<Vec<_>>();
    transcript.snapshot(ConversationSnapshot {
        seq: items.len() as u64,
        first_seq: 1,
        items,
    });
    transcript
}

/// GPUI-side cost of a switch: `update_view` plus the frame it schedules,
/// on the test platform (no-op text shaping, no GPU). Opt in with
/// `WKS_NATIVE_BENCH_SWITCH=cycles` and `--nocapture`.
#[gpui::test]
fn bench_switch_frames(cx: &mut TestAppContext) {
    let Some(cycles) = std::env::var("WKS_NATIVE_BENCH_SWITCH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    else {
        return;
    };
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let parent = View {
        transcript: dense(wks_native::controller::CONVERSATION_PAGE, "parent"),
        ..family()
    };
    let child = viewing("t1", dense(1000, "t1"));
    let sibling = viewing("t2", dense(1000, "t2"));
    // (update_view, update_view + the frame it schedules) in milliseconds.
    let mut frame = |view: &View| {
        let started = std::time::Instant::now();
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view.clone()), window, cx)
            })
        });
        let update = started.elapsed().as_secs_f64() * 1000.;
        visual.run_until_parked();
        (update, started.elapsed().as_secs_f64() * 1000.)
    };
    frame(&parent);
    let mut samples: std::collections::BTreeMap<&str, Vec<(f64, f64)>> = Default::default();
    for _ in 0..cycles {
        for (name, view) in [
            ("parent_to_child", &child),
            ("child_republish", &child),
            ("child_to_sibling", &sibling),
            ("child_to_parent", &parent),
            ("parent_republish", &parent),
        ] {
            let ms = frame(view);
            samples.entry(name).or_default().push(ms);
        }
    }
    for (name, values) in samples {
        let mut update = values.iter().map(|v| v.0).collect::<Vec<_>>();
        let mut total = values.iter().map(|v| v.1).collect::<Vec<_>>();
        update.sort_by(f64::total_cmp);
        total.sort_by(f64::total_cmp);
        let at = |v: &[f64], p: usize| v[(v.len() - 1) * p / 100];
        println!(
            "bench_switch_frames {name}: total p50 {:.2}ms p95 {:.2}ms max {:.2}ms; update_view p50 {:.2}ms (debug_assertions={})",
            at(&total, 50),
            at(&total, 95),
            at(&total, 100),
            at(&update, 50),
            cfg!(debug_assertions)
        );
    }
}

/// A parent's turn clock measures the parent. Child rows number from zero
/// too, so its duration labels (keyed by row) must not be computed against,
/// or painted on, a viewed child's transcript.
#[gpui::test]
fn a_viewed_child_never_shows_its_parents_turn_durations(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let start = 1_790_852_400_000i64;
    let timed = |text: &str| {
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: text.into(),
                timestamp: chrono::DateTime::from_timestamp_millis(start + 5_000)
                    .map(|t| t.to_rfc3339()),
                ..Default::default()
            }],
        });
        transcript
    };
    let mut labels = |view: View| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.turn_clocks.entry("a".into()).or_default().completed =
                    [timing::CompletedTurn {
                        started_ms: start,
                        ended_ms: start + 10_000,
                        stopped: false,
                    }]
                    .into();
                this.update_view(Arc::new(view), window, cx);
                this.duration_labels.clone()
            })
        })
    };
    let parent = View {
        transcript: timed("Parent finished."),
        ..family()
    };
    assert_eq!(
        labels(parent.clone()).len(),
        1,
        "the parent's turn is labelled"
    );
    assert!(labels(viewing("t1", timed("Child finished."))).is_empty());
    assert_eq!(labels(parent).len(), 1);
}
