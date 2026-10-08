//! Prompt caches going cold. A session's cached prefix lives until its last
//! request plus the cache's lifetime (5 minutes or an hour for Claude, an
//! estimated 10 for Codex). After that the next message resends the whole
//! context at the cache-write price, which for a large paused session is many
//! times what the same message costs warm. The snapshot says when the cache
//! expires; this judges against the clock, so a row turns cold while you
//! watch, and a timer redraws at the moment it does.
use super::*;
use wks_native::model::PromptCache;

/// The composer note speaks up from this context size. Below it a cold
/// resend costs cents, and the marker alone says enough.
pub(super) const LARGE_CONTEXT: u64 = 50_000;

#[derive(Default)]
pub(super) struct CacheUi {
    /// The redraw pending at the next expiry, with the expiry it is for.
    clock: Option<(i64, gpui::Task<()>)>,
    /// Notes closed, by session and expiry: a cache that warms up and goes
    /// cold again is news again.
    dismissed: std::collections::HashSet<String>,
}

// Per thread, like the caption preview: a test's clock must not leak into
// another concurrent test.
#[cfg(feature = "ui-tests")]
thread_local! {
    pub(super) static FAKE_NOW: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

/// Unix ms, as every warm/cold judgement here reads it.
pub(super) fn now() -> i64 {
    #[cfg(feature = "ui-tests")]
    if let Some(now) = FAKE_NOW.get() {
        return now;
    }
    timing::now_ms()
}

/// The session's cache once it has expired; `None` while warm, unknown, or
/// while the session is working (its next request is already on its way).
pub(super) fn cold(session: &Session, now: i64) -> Option<&PromptCache> {
    session
        .prompt_cache
        .as_ref()
        .filter(|cache| cache.cold_at(now) && !session.working())
}

/// "just now", "4m", "2h", "3d".
fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 48 * 60 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (24 * 60)),
    }
}

fn dollars(usd: f64) -> String {
    if usd < 0.01 {
        "<$0.01".into()
    } else {
        format!("${usd:.2}")
    }
}

/// What the composer note says, as in "Cache expired 2h ago — your next
/// message re-sends ~624K tokens (≈$6.24, vs $0.31 warm)".
pub(super) fn note_text(cache: &PromptCache, now: i64) -> String {
    let when = ago(now - cache.expires_at);
    let head = if cache.estimated {
        format!("Cache likely expired {when} (estimate)")
    } else {
        format!("Cache expired {when}")
    };
    let tokens = gauge::token_label(cache.context_tokens);
    let cost = match (cache.cold_cost_usd, cache.warm_cost_usd) {
        (Some(cold), Some(warm)) => format!(" (≈{}, vs {} warm)", dollars(cold), dollars(warm)),
        (Some(cold), None) => format!(" (≈{})", dollars(cold)),
        _ => String::new(),
    };
    format!("{head} — your next message re-sends ~{tokens} tokens{cost}")
}

/// The one-line reason behind the marker, for tooltips.
pub(super) fn marker_tooltip(cache: &PromptCache, now: i64) -> String {
    let estimate = if cache.estimated { " (estimate)" } else { "" };
    format!(
        "Prompt cache expired {}{estimate} · the next message re-sends ~{} tokens",
        ago(now - cache.expires_at),
        gauge::token_label(cache.context_tokens)
    )
}

/// The small mark a cold session carries: a snowflake, with its word where
/// there is room for one.
pub(super) fn marker(p: Palette, size: f32, word: bool) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(3.))
        .text_color(rgb(p.accent))
        .child(
            Icon::empty()
                .path("lucide/snowflake.svg")
                .size(px(size))
                .text_color(rgb(p.accent)),
        )
        .when(word, |d| d.child("Cold"))
}

impl Workspace {
    /// Redraw when the next warm cache in `view` expires, so its markers and
    /// note appear on time without a new snapshot.
    pub(super) fn arm_cache_clock(&mut self, view: &View, cx: &mut Context<Self>) {
        let now = now();
        let next = view
            .sessions
            .iter()
            .filter_map(|s| s.prompt_cache.as_ref())
            .map(|cache| cache.expires_at)
            .filter(|at| *at > now)
            .min();
        let Some(next) = next else {
            self.cache.clock = None;
            return;
        };
        if self.cache.clock.as_ref().is_some_and(|(at, _)| *at == next) {
            return;
        }
        let wait = std::time::Duration::from_millis((next - now) as u64);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.cache.clock = None;
                let view = this.view.clone();
                this.arm_cache_clock(&view, cx);
                cx.notify();
            });
        });
        self.cache.clock = Some((next, task));
    }

    /// The selected session's cold cache, when it is large enough to be
    /// worth a note and that note has not been closed.
    fn cold_note_cache(&self, now: i64) -> Option<(&Session, &PromptCache)> {
        if self.view.child.is_some() {
            return None;
        }
        let session = self.selected_session()?;
        let cache = cold(session, now)?;
        (cache.context_tokens >= LARGE_CONTEXT
            && !self.cache.dismissed.contains(&dismissal(session, cache)))
        .then_some((session, cache))
    }

    /// The note above the composer when the selected session's cache has
    /// gone cold and its context is large: what the next message costs.
    pub(super) fn render_cold_note(&self, cx: &mut Context<Self>) -> Option<Div> {
        let p = self.appearance.palette();
        let now = now();
        let (session, cache) = self.cold_note_cache(now)?;
        let key = dismissal(session, cache);
        Some(
            div()
                .debug_selector(|| "cold-cache-note".into())
                .occlude()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(p.control_radius))
                .bg(rgb(p.surface))
                .border_1()
                .border_color(gpui::Hsla::from(rgb(p.warning)).opacity(0.45))
                .child(marker(p, 13., false))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(chrome::scale::META))
                        .line_height(gpui::relative(1.45))
                        .text_color(rgb(p.warning))
                        .child(note_text(cache, now)),
                )
                .child(
                    div()
                        .debug_selector(|| "cold-cache-actions".into())
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .children(self.cold_note_actions(session, cx))
                        .child(
                            self.icon_button(
                                "cold-cache-dismiss",
                                "Dismiss",
                                IconName::Close,
                                true,
                            )
                            .debug_selector(|| "cold-cache-dismiss".into())
                            .size(px(24.))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.cache.dismissed.insert(key.clone());
                                    cx.notify();
                                },
                            )),
                        ),
                ),
        )
    }

    /// Buttons offered beside the cold-cache note's Dismiss: "Start fresh
    /// from a summary" hands off to a new session seeded with a cheap-model
    /// brief instead of resending the whole cold context.
    fn cold_note_actions(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        if !matches!(session.provider.as_str(), "claude" | "codex" | "") {
            return Vec::new();
        }
        let session = session.clone();
        vec![
            self.button("cold-start-fresh", "Start fresh from a summary", true)
                .debug_selector(|| "cold-start-fresh".into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_summary_handoff(&session, window, cx)
                }))
                .into_any_element(),
        ]
    }

    /// The expiry the pending redraw is for.
    #[cfg(test)]
    pub(super) fn cache_clock_at(&self) -> Option<i64> {
        self.cache.clock.as_ref().map(|(at, _)| *at)
    }

    #[cfg(test)]
    pub(super) fn cold_note(&self) -> Option<String> {
        let now = now();
        self.cold_note_cache(now)
            .map(|(_, cache)| note_text(cache, now))
    }
}

fn dismissal(session: &Session, cache: &PromptCache) -> String {
    format!("{}@{}", session.id, cache.expires_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(estimated: bool) -> PromptCache {
        PromptCache {
            ttl_seconds: 3600,
            last_request_at: 0,
            expires_at: 3_600_000,
            context_tokens: 624_000,
            estimated,
            cold_cost_usd: Some(6.24),
            warm_cost_usd: Some(0.312),
        }
    }

    #[test]
    fn the_note_says_how_long_ago_how_much_and_what_it_costs() {
        let hour = 3_600_000;
        assert_eq!(
            note_text(&cache(false), 3 * hour),
            "Cache expired 2h ago — your next message re-sends ~624K tokens (≈$6.24, vs $0.31 warm)"
        );
        assert_eq!(
            note_text(&cache(true), hour + 30_000),
            "Cache likely expired just now (estimate) — your next message re-sends ~624K tokens (≈$6.24, vs $0.31 warm)"
        );
        let unpriced = PromptCache {
            cold_cost_usd: None,
            warm_cost_usd: None,
            ..cache(false)
        };
        assert_eq!(
            note_text(&unpriced, hour + 12 * 60_000),
            "Cache expired 12m ago — your next message re-sends ~624K tokens"
        );
        assert_eq!(ago(3 * 24 * hour), "3d ago");
    }

    #[test]
    fn a_working_session_is_never_cold() {
        let mut session = Session {
            state: "input".into(),
            prompt_cache: Some(cache(false)),
            ..Default::default()
        };
        assert!(cold(&session, 3_599_999).is_none(), "still warm");
        assert!(cold(&session, 3_600_000).is_some());
        session.state = "responding".into();
        assert!(cold(&session, 3_600_000).is_none());
    }
}
