//! A session whose prompt cache expires turns cold while you watch: a
//! snowflake on its sidebar row and context gauge, and above its composer a
//! note saying what the next message will cost.
use super::*;
use std::time::Duration;
use wks_native::model::{ContextUsage, PromptCache};

/// 2026-10-07 10:00 UTC.
const T0: i64 = 1_791_367_200_000;
const MINUTE: i64 = 60_000;

/// Holds the warm/cold clock at a chosen time for the test's duration.
struct Clock;

impl Clock {
    fn at(ms: i64) -> Self {
        cache::FAKE_NOW.set(Some(ms));
        Self
    }

    fn set(&self, ms: i64) {
        cache::FAKE_NOW.set(Some(ms));
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        cache::FAKE_NOW.set(None);
    }
}

/// A paused Claude session holding 624K tokens behind a 5-minute cache last
/// used at `T0`.
fn paused(tokens: u64) -> Session {
    Session {
        id: "a".into(),
        label: "Alpha".into(),
        state: "stopped".into(),
        provider: "claude".into(),
        cwd: "/work/project".into(),
        model: "claude-opus-4-8".into(),
        context: ContextUsage {
            held_tokens: tokens,
            resolved_window: Some(1_000_000),
            ..Default::default()
        },
        prompt_cache: Some(PromptCache {
            ttl_seconds: 300,
            last_request_at: T0,
            expires_at: T0 + 5 * MINUTE,
            context_tokens: tokens,
            estimated: false,
            cold_cost_usd: Some(3.9),
            warm_cost_usd: Some(0.312),
        }),
        ..Default::default()
    }
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, session: Session) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings
                .kept_open
                .insert(this.project_scope.clone(), [("a".to_string(), 1)].into());
            this.update_view(
                Arc::new(View {
                    connected: true,
                    selected: Some("a".into()),
                    sessions: Arc::new(vec![session]),
                    ..Default::default()
                }),
                window,
                cx,
            );
        })
    });
    visual.run_until_parked();
}

fn note(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Option<String> {
    workspace.read_with(visual, |this, _| this.cold_note())
}

#[gpui::test]
fn a_paused_session_goes_cold_while_you_watch(cx: &mut TestAppContext) {
    let clock = Clock::at(T0 + MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, paused(624_000));

    // Warm: no marker, no note, and a redraw waiting on the expiry.
    assert_eq!(note(&workspace, &visual), None);
    assert!(visual.debug_bounds("cold-cache-note").is_none());
    assert!(visual.debug_bounds("sidebar-cold-0").is_none());
    workspace.read_with(&visual, |this, _| {
        assert!(this.paused(&this.view.sessions[0]));
        assert_eq!(
            this.cache_clock_at(),
            Some(T0 + 5 * MINUTE),
            "armed for the expiry"
        );
    });

    // Time passes. No snapshot arrives; the expiry timer redraws.
    clock.set(T0 + 7 * MINUTE);
    visual
        .executor()
        .advance_clock(Duration::from_millis(6 * MINUTE as u64));
    visual.run_until_parked();

    assert_eq!(
        note(&workspace, &visual).as_deref(),
        Some(
            "Cache expired 2m ago — your next message re-sends ~624K tokens (≈$3.90, vs $0.31 warm)"
        )
    );
    assert!(visual.debug_bounds("cold-cache-note").is_some());
    assert!(visual.debug_bounds("sidebar-cold-0").is_some());
    // The spot the summary handoff's button will join, beside Dismiss.
    let actions = bounds_of(&mut visual, "cold-cache-actions");
    let dismiss = bounds_of(&mut visual, "cold-cache-dismiss");
    assert!(actions.contains(&dismiss.center()));
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.cache_clock_at(), None, "nothing left to expire");
    });

    // The note sits above the composer.
    let composer = bounds_of(&mut visual, "chat-composer");
    let cold = bounds_of(&mut visual, "cold-cache-note");
    assert!(cold.bottom() <= composer.top());
}

#[gpui::test]
fn the_context_gauge_carries_the_cold_mark(cx: &mut TestAppContext) {
    let _clock = Clock::at(T0 + 90 * MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, paused(624_000));
    reveal_title(&workspace, &mut visual);
    let detail = bounds_of(&mut visual, "title-context-detail");
    let cold = bounds_of(&mut visual, "title-context-cold");
    assert!(detail.contains(&cold.center()));
}

#[gpui::test]
fn a_small_context_gets_the_mark_but_no_note(cx: &mut TestAppContext) {
    let _clock = Clock::at(T0 + 90 * MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, paused(12_000));
    assert!(visual.debug_bounds("sidebar-cold-0").is_some());
    assert_eq!(note(&workspace, &visual), None);
    assert!(visual.debug_bounds("cold-cache-note").is_none());
}

#[gpui::test]
fn a_dismissed_note_stays_closed_until_the_cache_goes_cold_again(cx: &mut TestAppContext) {
    let clock = Clock::at(T0 + 90 * MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, paused(624_000));
    assert!(note(&workspace, &visual).is_some());
    click(&mut visual, "cold-cache-dismiss");
    assert_eq!(note(&workspace, &visual), None);
    // The marker still says it.
    assert!(visual.debug_bounds("sidebar-cold-0").is_some());

    // Resumed, used, and left again: a new expiry is news again.
    let mut later = paused(624_000);
    let cache = later.prompt_cache.as_mut().unwrap();
    cache.last_request_at = T0 + 100 * MINUTE;
    cache.expires_at = T0 + 105 * MINUTE;
    clock.set(T0 + 101 * MINUTE);
    show(&workspace, &mut visual, later);
    assert_eq!(note(&workspace, &visual), None, "warm again");
    clock.set(T0 + 106 * MINUTE);
    visual
        .executor()
        .advance_clock(Duration::from_millis(5 * MINUTE as u64));
    visual.run_until_parked();
    assert!(note(&workspace, &visual).is_some());
}

#[gpui::test]
fn a_working_session_is_not_marked_cold(cx: &mut TestAppContext) {
    let _clock = Clock::at(T0 + 90 * MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut working = paused(624_000);
    working.state = "responding".into();
    show(&workspace, &mut visual, working);
    assert_eq!(note(&workspace, &visual), None);
    assert!(visual.debug_bounds("sidebar-cold-0").is_none());
}

#[gpui::test]
fn a_codex_estimate_says_so(cx: &mut TestAppContext) {
    let _clock = Clock::at(T0 + 30 * MINUTE);
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut codex = paused(200_000);
    codex.provider = "codex".into();
    codex.prompt_cache = Some(PromptCache {
        ttl_seconds: 600,
        last_request_at: T0,
        expires_at: T0 + 10 * MINUTE,
        context_tokens: 200_000,
        estimated: true,
        cold_cost_usd: Some(0.25),
        warm_cost_usd: Some(0.025),
    });
    show(&workspace, &mut visual, codex);
    assert_eq!(
        note(&workspace, &visual).as_deref(),
        Some(
            "Cache likely expired 20m ago (estimate) — your next message re-sends ~200K tokens (≈$0.25, vs $0.03 warm)"
        )
    );
}
