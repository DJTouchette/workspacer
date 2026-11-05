//! The context-window gauge. It lives on the title capsule: a hairline along
//! the title row's bottom edge fills with the share of the window in use,
//! and the capsule only speaks up when that matters. From 70% it carries a
//! steady glow in the warning tone, from 90% it grows a notice row, and the
//! exact figures sit beside the actions it reveals. Where the capsule is too
//! narrow for a readable hairline, the composer keeps the older meter. The
//! two never show at once.
use super::chrome::Tone;
use super::motion::{Spring, Tune};
use super::*;
use gpui_component::tooltip::Tooltip;
use std::time::Instant;
use wks_native::model::ContextReading;

/// The gauge turns amber here, the same as the desktop status bar's `ctx`.
const WARN_PCT: f64 = 70.;
/// And red here. The capsule also grows a notice row from this point.
const FULL_PCT: f64 = 90.;
/// From here the notice reads as an error, not a warning.
const CRITICAL_PCT: f64 = 95.;
/// How far the hairline sits in from each end of the title row. At 2px up
/// from the bottom, the capsule's 20px curve is ~11px in.
const INSET: f32 = 14.;
/// The narrowest resting capsule that carries the hairline. Narrower than
/// this (a short title in a narrow window), the track would be too short to
/// read, so the composer keeps its meter.
const MIN_CAPSULE: f32 = 120.;
/// The hairline's fill follows a new reading on a firm spring with no
/// visible overshoot.
const FILL: Tune = Tune {
    response: 0.45,
    damping: 0.9,
};
/// The steady glow fades in and out slowly and never pulses.
const WARMTH: Tune = Tune {
    response: 0.6,
    damping: 1.,
};
/// The steady glow's strength on the notice glow's scale, where a
/// new-notice greeting peaks at 1. Kept low so it reads as a tint.
const WARM_GLOW: f32 = 0.4;

/// What the runtime has said about a session's context window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Gauge {
    /// It reports a window but no usage yet, so the track shows empty.
    Waiting,
    Reading(ContextReading),
}

impl Gauge {
    /// `None` until the runtime reports, and then nothing shows.
    pub(super) fn of(session: &Session) -> Option<Self> {
        match session.context.reading() {
            Some(reading) => Some(Self::Reading(reading)),
            None if session.context.waiting => Some(Self::Waiting),
            None => None,
        }
    }

    /// Share of the window in use, 0 to 100.
    pub(super) fn pct(&self) -> Option<f64> {
        match self {
            Self::Reading(reading) => Some(reading.pct.clamp(0., 100.)),
            Self::Waiting => None,
        }
    }

    /// The fill's share of the track, 0 to 1. Any use shows at least a sliver.
    fn fill(&self) -> f32 {
        match self.pct() {
            Some(pct) if pct > 0. => (pct.max(2.) / 100.) as f32,
            _ => 0.,
        }
    }

    /// The tone of the steady glow, from 70%.
    fn warmth(&self) -> Option<Tone> {
        match self.pct()? {
            pct if pct >= CRITICAL_PCT => Some(Tone::Error),
            pct if pct >= WARN_PCT => Some(Tone::Warning),
            _ => None,
        }
    }

    /// The exact figures, as in "62% · 124K / 200K".
    pub(super) fn detail(&self) -> String {
        let Self::Reading(reading) = self else {
            return "Context —".to_owned();
        };
        let pct = format!("{}%", reading.pct.clamp(0., 100.).round() as u64);
        match (reading.tokens, reading.window) {
            (Some(tokens), Some(window)) => {
                format!("{pct} · {} / {}", token_label(tokens), token_label(window))
            }
            (Some(tokens), None) => format!("{pct} · {}", token_label(tokens)),
            _ => pct,
        }
    }

    /// The longer explanation behind the figures, for tooltips.
    fn explanation(&self) -> String {
        let Self::Reading(reading) = self else {
            return "The provider reported a context window but not current-request usage yet"
                .to_owned();
        };
        let detail = match (reading.tokens, reading.window) {
            (Some(tokens), Some(window)) => format!(
                "{} of {} tokens in context",
                token_label(tokens),
                token_label(window)
            ),
            (Some(tokens), None) => format!("{} tokens in context", token_label(tokens)),
            _ => "Share of the context window in use".to_owned(),
        };
        format!(
            "Context {}% · {detail}",
            reading.pct.clamp(0., 100.).round() as u64
        )
    }
}

/// The gauge's color: green, amber from 70%, red from 90%, muted while
/// waiting.
pub(super) fn gauge_color(pct: Option<f64>, p: Palette) -> u32 {
    match pct {
        Some(pct) if pct >= FULL_PCT => p.error,
        Some(pct) if pct >= WARN_PCT => p.warning,
        Some(_) => p.success,
        None => p.muted,
    }
}

/// The notice row a nearly full context grows on the island.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContextNotice {
    /// The session and the threshold band, never the exact share: it names
    /// the row and is what a dismissal records. A token tick changes the
    /// words but not the key, so a dismissed row stays dismissed until the
    /// band changes.
    pub(super) key: String,
    pub(super) text: String,
    pub(super) tone: Tone,
}

/// From 90%: a warning, and from 95% an error.
pub(super) fn context_notice(session: &Session) -> Option<ContextNotice> {
    let Some(Gauge::Reading(reading)) = Gauge::of(session) else {
        return None;
    };
    let pct = reading.pct.clamp(0., 100.);
    let (band, tone) = match pct {
        pct if pct >= CRITICAL_PCT => (CRITICAL_PCT, Tone::Error),
        pct if pct >= FULL_PCT => (FULL_PCT, Tone::Warning),
        _ => return None,
    };
    let share = format!("Context {}% full", pct.round() as u64);
    let text = match (reading.tokens, reading.window) {
        (Some(tokens), Some(window)) => format!(
            "{share} — {} of {} tokens",
            token_label(tokens),
            token_label(window)
        ),
        _ => share,
    };
    Some(ContextNotice {
        key: format!("{}@{band}", session.id),
        text,
        tone,
    })
}

/// Whether a resting capsule this wide (inside its border, `0` before its
/// first measurement) carries the gauge, or leaves it to the composer.
pub(super) fn capsule_fits(width: gpui::Pixels) -> bool {
    width == px(0.) || width >= px(MIN_CAPSULE)
}

pub(super) fn token_label(tokens: u64) -> String {
    match tokens {
        t if t >= 1_000_000 => {
            let m = t as f64 / 1_000_000.;
            if m.fract() < 0.05 {
                format!("{m:.0}M")
            } else {
                format!("{m:.1}M")
            }
        }
        t if t >= 1_000 => format!("{}K", (t as f64 / 1_000.).round() as u64),
        t => t.to_string(),
    }
}

/// The hairline along the title row's bottom edge, laid out inside the bar
/// (bottom of its padding box) so it stays on the title row as notices grow
/// the island below. `fill` is the animated share of the track.
pub(super) fn hairline(gauge: Gauge, fill: f32, p: Palette) -> Div {
    let color = gauge_color(gauge.pct(), p);
    let reading = matches!(gauge, Gauge::Reading(_));
    div()
        .debug_selector(|| "title-context".into())
        .absolute()
        .bottom_0()
        .left(px(INSET))
        .right(px(INSET))
        .h(px(2.))
        .rounded_full()
        .overflow_hidden()
        // A faint track helps read the share. Waiting, it is just the track.
        .bg(gpui::Hsla::from(rgb(p.border)).opacity(if reading { 0.55 } else { 0.9 }))
        .when(reading && fill > 0., |d| {
            d.child(
                div()
                    .debug_selector(|| "title-context-fill".into())
                    .h_full()
                    .w(gpui::relative(fill.clamp(0., 1.)))
                    .rounded_full()
                    .bg(rgb(color)),
            )
        })
}

/// The exact figures, shown beside the capsule's revealed actions, led by
/// the cold mark when the session's prompt cache has expired.
pub(super) fn detail(gauge: Gauge, cold: Option<String>, p: Palette) -> Stateful<Div> {
    let tooltip = match &cold {
        Some(cold) => format!("{}\n{cold}", gauge.explanation()),
        None => gauge.explanation(),
    };
    let text = gauge.detail();
    let (share, rest) = match text.split_once(" · ") {
        Some((share, rest)) if gauge.pct().is_some() => (share.to_owned(), Some(rest.to_owned())),
        _ => (text, None),
    };
    div()
        .id("title-context-detail")
        .debug_selector(|| "title-context-detail".into())
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .text_size(px(11.))
        .text_color(rgb(p.muted))
        .when(cold.is_some(), |d| {
            d.child(cache::marker(p, 11., true).debug_selector(|| "title-context-cold".into()))
        })
        .child(
            div()
                .text_color(rgb(gauge_color(gauge.pct(), p)))
                .child(share),
        )
        .when_some(rest, |d, rest| d.child("·").child(rest))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
}

/// The composer's context meter, used when the title capsule is too narrow
/// to carry the gauge. It follows the desktop status bar's `ctx` gauge: a
/// thin rounded track, the percentage, and the tokens held of the window in
/// the tooltip. `None` until the runtime reports.
pub(super) fn context_meter(session: &Session, p: Palette) -> Option<Stateful<Div>> {
    let gauge = Gauge::of(session)?;
    let pct = gauge.pct();
    let label = match pct {
        Some(pct) => format!("{}%", pct.round() as u64),
        None => "—".to_owned(),
    };
    let now = cache::now();
    let cold = cache::cold(session, now).map(|cold| cache::marker_tooltip(cold, now));
    let tooltip = match &cold {
        Some(cold) => format!("{}\n{cold}", gauge.explanation()),
        None => gauge.explanation(),
    };
    let color = gauge_color(pct, p);
    const TRACK: f32 = 44.;
    Some(
        div()
            .id("context-meter")
            .debug_selector(|| "context-meter".into())
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px_2()
            .h(px(28.))
            .rounded_full()
            .text_size(px(11.))
            .child(div().text_color(rgb(p.muted)).child("ctx"))
            .child(
                div()
                    .w(px(TRACK))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(p.border))
                    .overflow_hidden()
                    .when(pct.is_some(), |d| {
                        d.child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(rgb(color))
                                .w(px(gauge.fill() * TRACK)),
                        )
                    }),
            )
            .child(div().text_color(rgb(color)).child(label))
            .when(cold.is_some(), |d| {
                d.child(cache::marker(p, 11., false).debug_selector(|| "context-meter-cold".into()))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
    )
}

/// How the gauge moves. The fill follows each new reading on a spring, and
/// the steady glow fades in and out as the share crosses 70%. Like the notice
/// island, it snaps where it belongs on the first render, on a chat switch,
/// when the track first appears, and with reduced motion. It asks for frames
/// only while something is moving.
pub(super) struct GaugeMotion {
    fill: Spring,
    warmth: Spring,
    /// The glow's last tone, kept so it fades out in the color it had.
    tone: Tone,
    /// Whether the last frame drew a track.
    track: bool,
    chat: Option<u64>,
    motion: bool,
}

impl Default for GaugeMotion {
    fn default() -> Self {
        Self {
            fill: Spring::new(FILL, 0., 0.002),
            warmth: Spring::new(WARMTH, 0., 0.01),
            tone: Tone::Warning,
            track: false,
            chat: None,
            motion: true,
        }
    }
}

impl GaugeMotion {
    /// Brings the gauge in line with the reading the capsule shows now.
    pub(super) fn sync(&mut self, gauge: Option<Gauge>, chat: u64, motion: bool, now: Instant) {
        let snap = !motion || self.chat != Some(chat) || !self.track;
        self.chat = Some(chat);
        self.motion = motion;
        self.track = gauge.is_some();
        let tune = |tune| if motion { tune } else { Tune::INSTANT };
        self.fill.retune(tune(FILL));
        self.warmth.retune(tune(WARMTH));
        let fill = gauge.map_or(0., |gauge| gauge.fill());
        let tone = gauge.and_then(|gauge| gauge.warmth());
        if let Some(tone) = tone {
            self.tone = tone;
        }
        let warmth = if tone.is_some() { 1. } else { 0. };
        if snap {
            self.fill.snap(fill);
            self.warmth.snap(warmth);
        } else {
            self.fill.set(fill, now);
            self.warmth.set(warmth, now);
        }
    }

    /// The hairline's filled share of its track now.
    pub(super) fn fill(&self, now: Instant) -> f32 {
        self.fill.value(now).clamp(0., 1.)
    }

    /// The steady glow, as a tone and a strength on the notice glow's scale.
    pub(super) fn glow(&self, now: Instant) -> Option<(Tone, f32)> {
        let warmth = self.warmth.value(now).clamp(0., 1.);
        (warmth > 0.).then_some((self.tone, warmth * WARM_GLOW))
    }

    /// Whether another frame would differ from this one.
    pub(super) fn moving(&self, now: Instant) -> bool {
        self.motion && !(self.fill.settled(now) && self.warmth.settled(now))
    }
}

impl Workspace {
    /// Whether the title capsule carries the context gauge now. Otherwise
    /// the composer keeps its meter. It decides for both, so the two never
    /// show at once. A subagent's capsule never carries it, because its
    /// chat has no composer and the gauge belongs to the parent.
    pub(super) fn title_carries_context(&self) -> bool {
        self.view.child.is_none() && capsule_fits(self.title_base_width)
    }

    #[cfg(test)]
    pub(super) fn gauge_moving(&self) -> bool {
        self.gauge_motion.moving(Instant::now())
    }

    /// Whether the last frame drew the capsule's hairline. (GPUI keeps an
    /// element's debug bounds after it stops drawing, so tests ask this.)
    #[cfg(test)]
    pub(super) fn hairline_drawn(&self) -> bool {
        self.gauge_motion.track
    }

    /// The island's rows as the last frame left them, without ghosts.
    #[cfg(test)]
    pub(super) fn island_slots(&self) -> Vec<&'static str> {
        self.island_motion
            .rows()
            .iter()
            .filter(|row| !row.leaving)
            .map(|row| row.slot)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn reading(pct: f64, tokens: Option<u64>, window: Option<u64>) -> Gauge {
        Gauge::Reading(ContextReading {
            pct,
            tokens,
            window,
        })
    }

    fn session(pct: f64) -> Session {
        let mut session = Session {
            id: "a".into(),
            ..Default::default()
        };
        session.merge(&serde_json::json!({
            "statusLine": {"contextUsedPct": pct, "contextWindowSize": 200000}
        }));
        session
    }

    #[test]
    fn colors_follow_the_desktop_bands() {
        let p = Appearance::default().palette();
        assert_eq!(gauge_color(Some(0.), p), p.success);
        assert_eq!(gauge_color(Some(69.9), p), p.success);
        assert_eq!(gauge_color(Some(70.), p), p.warning);
        assert_eq!(gauge_color(Some(89.9), p), p.warning);
        assert_eq!(gauge_color(Some(90.), p), p.error);
        assert_eq!(gauge_color(None, p), p.muted);
    }

    #[test]
    fn figures_read_as_share_then_tokens() {
        let gauge = reading(62., Some(124_000), Some(200_000));
        assert_eq!(gauge.detail(), "62% · 124K / 200K");
        assert_eq!(reading(8.4, Some(84_000), None).detail(), "8% · 84K");
        assert_eq!(reading(41., None, None).detail(), "41%");
        assert_eq!(Gauge::Waiting.detail(), "Context —");
        assert_eq!(reading(0., None, None).fill(), 0.);
        assert_eq!(reading(0.5, None, None).fill(), 0.02, "a sliver shows");
        assert_eq!(reading(130., None, None).fill(), 1.);
    }

    #[test]
    fn no_reading_means_no_gauge_and_waiting_means_an_empty_track() {
        assert_eq!(Gauge::of(&Session::default()), None);
        let mut waiting = Session::default();
        waiting.merge(&serde_json::json!({
            "statusLine": {
                "contextWindowSize": 200000,
                "contextUsageState": "waitingForRuntimeUsage"
            }
        }));
        assert_eq!(Gauge::of(&waiting), Some(Gauge::Waiting));
        assert_eq!(Gauge::Waiting.fill(), 0.);
        assert_eq!(Gauge::Waiting.warmth(), None);
    }

    #[test]
    fn the_notice_is_keyed_on_its_band_not_the_exact_share() {
        assert_eq!(context_notice(&session(89.)), None);
        let first = context_notice(&session(92.)).unwrap();
        assert_eq!(first.text, "Context 92% full — 184K of 200K tokens");
        assert_eq!(first.tone, Tone::Warning);
        let later = context_notice(&session(94.4)).unwrap();
        assert_eq!(later.key, first.key, "a token tick is the same band");
        assert_ne!(later.text, first.text, "the words stay current");
        let critical = context_notice(&session(96.)).unwrap();
        assert_ne!(critical.key, first.key);
        assert_eq!(critical.tone, Tone::Error);
        let mut other = session(92.);
        other.id = "b".into();
        assert_ne!(context_notice(&other).unwrap().key, first.key);
    }

    #[test]
    fn warmth_starts_at_seventy_and_turns_to_error_at_ninety_five() {
        assert_eq!(reading(69., None, None).warmth(), None);
        assert_eq!(reading(70., None, None).warmth(), Some(Tone::Warning));
        assert_eq!(reading(92., None, None).warmth(), Some(Tone::Warning));
        assert_eq!(reading(95., None, None).warmth(), Some(Tone::Error));
    }

    #[test]
    fn a_narrow_capsule_leaves_the_gauge_to_the_composer() {
        assert!(capsule_fits(px(0.)), "unmeasured: the capsule");
        assert!(capsule_fits(px(260.)));
        assert!(!capsule_fits(px(80.)));
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn the_fill_springs_to_each_new_reading_and_then_rests() {
        let start = Instant::now();
        let mut gauge = GaugeMotion::default();
        // The first render snaps.
        gauge.sync(Some(reading(30., None, None)), 1, true, start);
        assert_eq!(gauge.fill(start), 0.3);
        assert!(!gauge.moving(start) && gauge.glow(start).is_none());
        // A new reading moves smoothly, warming as it crosses 70%.
        gauge.sync(Some(reading(75., None, None)), 1, true, start);
        assert!(gauge.moving(start));
        let mid = gauge.fill(at(start, 120));
        assert!(0.3 < mid && mid < 0.75, "{mid}");
        assert!(
            matches!(gauge.glow(at(start, 120)), Some((Tone::Warning, s)) if s > 0. && s < WARM_GLOW)
        );
        let rest = at(start, 2000);
        assert!(!gauge.moving(rest), "idle once settled");
        assert_eq!(gauge.fill(rest), 0.75);
        assert_eq!(gauge.glow(rest), Some((Tone::Warning, WARM_GLOW)));
        // The same reading again is no motion at all.
        gauge.sync(Some(reading(75., None, None)), 1, true, rest);
        assert!(!gauge.moving(rest));
        // Back under 70%, the glow fades out in its own tone.
        gauge.sync(Some(reading(40., None, None)), 1, true, rest);
        assert!(matches!(
            gauge.glow(at(start, 2100)),
            Some((Tone::Warning, _))
        ));
        assert!(gauge.glow(at(start, 4000)).is_none());
    }

    #[test]
    fn chat_switches_reduced_motion_and_a_new_track_snap() {
        let start = Instant::now();
        let mut gauge = GaugeMotion::default();
        gauge.sync(Some(reading(30., None, None)), 1, true, start);
        gauge.sync(Some(reading(80., None, None)), 2, true, start);
        assert_eq!(gauge.fill(start), 0.8, "another chat");
        assert!(!gauge.moving(start));
        gauge.sync(Some(reading(20., None, None)), 2, false, start);
        assert_eq!(gauge.fill(start), 0.2, "reduced motion");
        assert!(!gauge.moving(start) && gauge.glow(start).is_none());
        gauge.sync(None, 2, true, start);
        gauge.sync(Some(reading(60., None, None)), 2, true, start);
        assert_eq!(gauge.fill(start), 0.6, "the track appears whole");
        // An empty track that gets its first reading fills in.
        gauge.sync(Some(Gauge::Waiting), 3, true, start);
        assert_eq!(gauge.fill(start), 0.);
        gauge.sync(Some(reading(50., None, None)), 3, true, start);
        assert!(gauge.moving(start));
    }
}
