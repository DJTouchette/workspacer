//! The title capsule at rest and revealed. Like a Dynamic Island, the
//! conversation's secondary actions stay tucked away until the pointer rests
//! on the capsule, keyboard focus enters it, or a tap on the title pins it
//! open. Hidden actions are clipped to no width, so they have no hit area and
//! cannot be clicked by accident; they stay in Tab order, and focusing one
//! reveals them at once.
use super::chrome::{self, Tone};
use super::motion::{Spring, Tune, smooth};
use super::*;
use gpui::{ClickEvent, DispatchPhase, MouseButton, MouseExitEvent, MouseMoveEvent};
use std::time::{Duration, Instant};

/// Hover intent: a pointer passing across the capsule does not open it.
const OPEN_DELAY: Duration = Duration::from_millis(70);
/// Grace before closing, so skimming an edge or briefly leaving is no flicker.
const CLOSE_GRACE: Duration = Duration::from_millis(220);
/// The reveal: quick, with a few pixels of stretch past the actions before
/// it settles (about 1.5%, peaking at 200ms), at rest ~300ms after it starts.
const REVEAL: Tune = Tune {
    response: 0.24,
    damping: 0.8,
};
/// Reveal progress this close to 0 or 1 is there (well under a pixel).
const REVEAL_REST: f32 = 0.003;

/// Whether a fresh measurement of the actions should replace `current`.
/// The measured row sits inside a box sized from the last measurement, so
/// pixel snapping at fractional scales can shift it by half a pixel each
/// layout; taking every change would re-render forever. Growth always wins
/// (nothing is clipped), but a shrink must be a whole pixel to count.
fn settle_width(current: gpui::Pixels, measured: gpui::Pixels) -> bool {
    measured > current || current - measured >= gpui::px(1.)
}

/// A reversible reveal on a spring: progress 0 is the capsule at rest, 1 is
/// every action shown (it may stretch a touch past 1 before settling).
/// Changing direction mid-way turns from where it is, with its momentum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reveal {
    open: bool,
    /// Whether this change waits for intent (opening) or grace (closing).
    waits: bool,
    spring: Spring,
}

impl Default for Reveal {
    fn default() -> Self {
        Self {
            open: false,
            waits: false,
            spring: Spring::new(REVEAL, 0., REVEAL_REST),
        }
    }
}

impl Reveal {
    pub(super) fn progress(&self, now: Instant) -> f32 {
        self.spring.value(now)
    }

    /// Head toward `open`. `hold` waits for intent or grace first, unless
    /// the capsule is already moving; a change without `hold` (keyboard
    /// focus, a tap) also cuts short a wait already under way.
    pub(super) fn set(&mut self, open: bool, hold: bool, now: Instant) {
        if open == self.open && (hold || !self.waits) {
            return;
        }
        if open == self.open {
            self.waits = false;
            self.spring.hurry(now);
            return;
        }
        let (current, velocity) = self.spring.sample(now);
        let waits = hold && velocity == 0. && (current <= 0. || current >= 1.);
        let wait = match (waits, open) {
            (false, _) => Duration::ZERO,
            (true, true) => OPEN_DELAY,
            (true, false) => CLOSE_GRACE,
        };
        self.open = open;
        self.waits = waits;
        self.spring.set_after(if open { 1. } else { 0. }, now, wait);
    }

    /// Reduced motion: the reveal still waits for intent, then jumps.
    fn retune(&mut self, motion: bool) {
        self.spring
            .retune(if motion { REVEAL } else { Tune::INSTANT });
    }

    /// Already at rest wherever it is headed.
    #[cfg(test)]
    fn snapped(open: bool) -> Self {
        let mut reveal = Self {
            open,
            ..Self::default()
        };
        reveal.spring.snap(if open { 1. } else { 0. });
        reveal
    }

    pub(super) fn settled(&self, now: Instant) -> bool {
        self.spring.settled(now)
    }

    /// Fully shown: only then do pointer clicks reach the actions, so one
    /// cannot land on a control that slid under a resting pointer.
    pub(super) fn ready(&self, now: Instant) -> bool {
        self.open && self.settled(now)
    }
}

/// Everything the capsule needs to decide whether its actions show.
pub(super) struct TitleReveal {
    pub(super) reveal: Reveal,
    pub(super) hovered: bool,
    /// Tapped open (for pointers without hover); a second tap releases it.
    pub(super) pinned: bool,
    /// Contains the capsule's own controls: focus inside reveals them.
    pub(super) focus: FocusHandle,
    /// What focus a pointer press inside the capsule left behind. A clicked
    /// control keeps focus (and its ring), but that is the pointer's doing:
    /// only focus that arrives otherwise (Tab) holds the actions open.
    pointer_focus: Option<gpui::WeakFocusHandle>,
    /// The actions' natural width, measured while clipped.
    pub(super) width: gpui::Pixels,
}

impl TitleReveal {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            reveal: Reveal::default(),
            hovered: false,
            pinned: false,
            focus: cx.focus_handle(),
            pointer_focus: None,
            width: gpui::px(0.),
        }
    }

    /// A tap on the title: pins a capsule that is not already showing its
    /// actions (as hover would), or releases a pin.
    pub(super) fn tap(&mut self, now: Instant) {
        if self.pinned {
            self.pinned = false;
        } else if !self.reveal.ready(now) {
            self.pinned = true;
        }
    }
}

/// The island's own shape: growing around notices, a little looser than the
/// reveal, overshooting about 3% before it settles (at rest in ~430ms).
const SHAPE: Tune = Tune {
    response: 0.36,
    damping: 0.72,
};
/// A row's height following its text when that changes or rewraps.
const HEIGHT: Tune = Tune {
    response: 0.3,
    damping: 0.86,
};
/// Shape progress this close to its target is there.
const SHAPE_REST: f32 = 0.003;
/// Rows arriving together start a beat apart, top first.
const STAGGER: Duration = Duration::from_millis(45);
/// A row's content follows its shape: it starts once the shape is moving.
const CONTENT_DELAY: Duration = Duration::from_millis(70);
const CONTENT_FADE: Duration = Duration::from_millis(200);
/// Leaving, the content goes first and the shape follows it.
const LEAVE_FADE: Duration = Duration::from_millis(110);
const LEAVE_LEAD: Duration = Duration::from_millis(80);
/// The tinted pulse that greets a new notice, once.
const GLOW: Duration = Duration::from_millis(700);
/// How far content drifts as it fades: in from under the seam, out into it.
const DRIFT: f32 = 4.;
/// The island's rows in the order they stack.
const SLOTS: [&str; 6] = [
    "connection",
    "status",
    "feature",
    "context",
    "refresh",
    "omitted",
];

/// One notice the island shows now.
pub(super) struct Notice {
    pub(super) slot: &'static str,
    /// What makes it news. A changed key greets again and re-measures,
    /// while words that change under the same key (a live figure) just
    /// update. For most notices it is the text itself.
    pub(super) key: String,
    pub(super) text: String,
    pub(super) tone: Tone,
    /// Shows a dismiss button, which its ghost keeps while it fades.
    pub(super) dismissible: bool,
}

/// A row the island is showing, arriving or letting go of. A leaving row
/// keeps its words so it can fade out in place before its room closes.
pub(super) struct NoticeRow {
    pub(super) slot: &'static str,
    key: String,
    pub(super) text: String,
    pub(super) tone: Tone,
    pub(super) dismissible: bool,
    pub(super) leaving: bool,
    /// 0 is no room at all, 1 its full height.
    presence: Spring,
    /// Its natural height in pixels, followed smoothly.
    height: Spring,
    measured: bool,
    /// New words await their measurement before the height moves.
    pending: bool,
    /// When the content's fade starts (in or out).
    since: Instant,
}

impl NoticeRow {
    /// Fixed size, a share of its natural height, or its natural height
    /// (`None`) once nothing about it is moving.
    pub(super) fn height(&self, now: Instant) -> Option<gpui::Pixels> {
        let presence = self.presence.value(now);
        let resting = presence == 1. && self.presence.settled(now) && self.height.settled(now);
        if resting && !self.pending {
            return None;
        }
        let natural = if self.measured {
            self.height.value(now)
        } else {
            0.
        };
        Some(px((natural * presence).max(0.)))
    }

    /// How much room it holds, 0 to 1, for the gaps beside it.
    pub(super) fn presence(&self, now: Instant) -> f32 {
        self.presence.value(now).clamp(0., 1.)
    }

    /// The content's opacity and vertical drift.
    pub(super) fn content(&self, now: Instant) -> (f32, gpui::Pixels) {
        let since = |delay: Duration| {
            now.saturating_duration_since(self.since + delay)
                .as_secs_f32()
        };
        let shown = if self.leaving {
            1. - smooth(since(Duration::ZERO) / LEAVE_FADE.as_secs_f32())
        } else {
            let t = (since(CONTENT_DELAY) / CONTENT_FADE.as_secs_f32()).min(1.);
            1. - (1. - t).powi(3)
        };
        (shown, px(-DRIFT * (1. - shown)))
    }

    fn moving(&self, now: Instant) -> bool {
        let fade = if self.leaving {
            LEAVE_FADE
        } else {
            CONTENT_DELAY + CONTENT_FADE
        };
        !self.presence.settled(now) || !self.height.settled(now) || now < self.since + fade
    }
}

/// How the title island moves as notices come and go: each row grows into
/// place and its words fade in after, rows that go fade first and then
/// close, and the island itself (seam, padding, the actions' room beside
/// notices) grows and shrinks on the same spring. With reduced motion, or
/// across a chat switch, everything is simply where it belongs.
pub(super) struct IslandMotion {
    rows: Vec<NoticeRow>,
    /// The island grown around notices: 0 the bare capsule, 1 grown.
    open: Spring,
    glow: Option<(Tone, Instant)>,
    /// The conversation these rows belong to; another one starts at rest.
    chat: Option<u64>,
    motion: bool,
}

impl Default for IslandMotion {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            open: Spring::new(SHAPE, 0., SHAPE_REST),
            glow: None,
            chat: None,
            motion: true,
        }
    }
}

impl IslandMotion {
    /// Brings the rows in line with what the island shows now.
    pub(super) fn sync(&mut self, notices: &[Notice], chat: u64, motion: bool, now: Instant) {
        let snap = !motion || self.chat != Some(chat);
        self.chat = Some(chat);
        self.motion = motion;
        let tune = if motion { SHAPE } else { Tune::INSTANT };
        self.open.retune(tune);
        self.rows
            .retain(|row| !(row.leaving && row.presence.settled(now)));
        let mut arrived: Option<Tone> = None;
        let mut entering = 0;
        for notice in notices {
            if let Some(row) = self.rows.iter_mut().find(|row| row.slot == notice.slot) {
                if row.leaving {
                    // Back before it was gone: it grows again from where it is.
                    row.leaving = false;
                    row.presence.set(1., now);
                    row.since = now;
                }
                row.dismissible = notice.dismissible;
                row.text.clone_from(&notice.text);
                if row.key != notice.key {
                    row.key.clone_from(&notice.key);
                    row.tone = notice.tone;
                    row.since = now;
                    row.pending = row.measured;
                    arrived = most_urgent(arrived, notice.tone);
                }
                continue;
            }
            let start = now + STAGGER * entering;
            entering += 1;
            let mut presence = Spring::new(tune, 0., SHAPE_REST);
            presence.set_after(1., now, start - now);
            self.rows.push(NoticeRow {
                slot: notice.slot,
                key: notice.key.clone(),
                text: notice.text.clone(),
                tone: notice.tone,
                dismissible: notice.dismissible,
                leaving: false,
                presence,
                height: Spring::new(HEIGHT, 0., 0.5),
                measured: false,
                pending: false,
                since: start,
            });
            arrived = most_urgent(arrived, notice.tone);
        }
        for row in &mut self.rows {
            if !row.leaving && !notices.iter().any(|n| n.slot == row.slot) {
                row.leaving = true;
                row.since = now;
                row.presence.set_after(0., now, LEAVE_LEAD);
            }
        }
        let order = |slot: &str| SLOTS.iter().position(|s| *s == slot).unwrap_or(SLOTS.len());
        self.rows.sort_by_key(|row| order(row.slot));
        if notices.is_empty() {
            self.open.set_after(0., now, LEAVE_LEAD);
        } else {
            self.open.set(1., now);
        }
        if snap {
            self.rest(now);
        } else if let Some(tone) = arrived.filter(|tone| *tone != Tone::Loading) {
            self.glow = Some((tone, now));
        }
    }

    pub(super) fn rows(&self) -> &[NoticeRow] {
        &self.rows
    }

    /// How far the island is grown around notices (it may overshoot 1).
    pub(super) fn open(&self, now: Instant) -> f32 {
        self.open.value(now)
    }

    /// Whether the capsule is an island at all: notices showing, or the
    /// shape still on its way back to the bare capsule.
    pub(super) fn shows(&self, now: Instant) -> bool {
        !self.rows.is_empty() || self.open.value(now) > 0.
    }

    /// A row's natural height, as measured; false if nothing changed.
    pub(super) fn measured(&mut self, slot: &str, height: gpui::Pixels, now: Instant) -> bool {
        let motion = self.motion;
        let Some(row) = self.rows.iter_mut().find(|row| row.slot == slot) else {
            return false;
        };
        let height = f32::from(height);
        if row.measured && !row.pending && !settle_width(px(row.height.target()), px(height)) {
            return false;
        }
        // At full size it is already laid out at its natural height, so
        // the spring catches up at once; moving or with new words, it eases.
        let sized = row.height(now).is_some();
        if !row.measured || !motion || !sized {
            row.height.snap(height);
        } else {
            row.height.set(height, now);
        }
        row.measured = true;
        row.pending = false;
        true
    }

    /// Whether another frame would differ from this one.
    pub(super) fn moving(&self, now: Instant) -> bool {
        self.motion
            && (!self.open.settled(now)
                || self.rows.iter().any(|row| row.moving(now))
                || self.glow(now).is_some())
    }

    /// The greeting pulse for a new notice: its tone and strength now.
    pub(super) fn glow(&self, now: Instant) -> Option<(Tone, f32)> {
        let (tone, at) = self.glow?;
        let t = now.saturating_duration_since(at).as_secs_f32() / GLOW.as_secs_f32();
        (t < 1.).then(|| (tone, smooth(t / 0.15) * (1. - t).powi(2)))
    }

    /// Extra roundness for the island's lower corners while its height is
    /// changing, so it reads as one soft shape stretching, not a box.
    pub(super) fn bulge(&self, now: Instant) -> f32 {
        if !self.motion {
            return 0.;
        }
        let (_, chrome) = self.open.sample(now);
        let mut speed = chrome * 13.;
        for row in &self.rows {
            let (presence, grow) = row.presence.sample(now);
            let (height, change) = row.height.sample(now);
            let height = if row.measured { height } else { 0. };
            speed += grow * height + presence * change;
        }
        (speed.abs() * 0.025).min(8.)
    }

    /// Every row and the island itself where they are headed, at once.
    fn rest(&mut self, now: Instant) {
        self.rows.retain(|row| !row.leaving);
        for row in &mut self.rows {
            row.presence.snap(1.);
            row.height.snap(row.height.target());
            row.pending = false;
            row.since = now - CONTENT_DELAY - CONTENT_FADE;
        }
        self.open.snap(self.open.target());
        self.glow = None;
    }

    #[cfg(test)]
    pub(super) fn settle(&mut self) {
        self.rest(Instant::now());
    }

    /// Holds every row and the island part-grown, for a frame mid-motion.
    #[cfg(test)]
    pub(super) fn freeze(&mut self, presence: f32) {
        let until = Instant::now() + Duration::from_secs(3600);
        for row in &mut self.rows {
            row.presence.hold(presence, until);
        }
        self.open.hold(presence, until);
    }
}

/// The more pressing of two tones, for the one pulse a batch of notices gets.
fn most_urgent(current: Option<Tone>, next: Tone) -> Option<Tone> {
    let rank = |tone: Tone| match tone {
        Tone::Loading => 0,
        Tone::Info => 1,
        Tone::Success => 2,
        Tone::Warning => 3,
        Tone::Error => 4,
    };
    Some(match current {
        Some(current) if rank(current) >= rank(next) => current,
        _ => next,
    })
}

impl Workspace {
    /// Appends the capsule's secondary actions behind the reveal, and the
    /// hover and focus tracking that drives it. `noticed` is how far the
    /// island is grown around notices (0 to 1, from `IslandMotion::open`):
    /// grown, it keeps the actions' room at rest, with the capsule's content
    /// centered in it, so revealing them never changes the island's
    /// outline, rewraps a notice or moves one of its controls. Also returns
    /// how much wider than at rest the actions make the capsule now.
    pub(super) fn reveal_title_actions(
        &mut self,
        bar: Stateful<Div>,
        actions: Div,
        noticed: f32,
        now: Instant,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Stateful<Div>, gpui::Pixels) {
        let p = self.appearance.palette();
        let motion = !self.settings.reduce_motion;
        let state = &mut self.title_reveal;
        state.reveal.retune(motion);
        // A pointer press's focus counts once focus moves on (Tab, or away
        // and back), so it is only remembered while it stays put.
        if window.focused(cx) != state.pointer_focus.as_ref().and_then(|f| f.upgrade()) {
            state.pointer_focus = None;
        }
        let focused = state.focus.contains_focused(window, cx) && state.pointer_focus.is_none();
        let open = state.hovered || state.pinned || focused;
        // Pointer hover waits for intent; focus and taps answer at once.
        let hold = !open || !(focused || state.pinned);
        state.reveal.set(open, hold, now);
        let progress = state.reveal.progress(now);
        if !state.reveal.settled(now) {
            window.request_animation_frame();
        }
        let natural = state.width;
        let measure = cx.entity().downgrade();
        let hover = cx.entity().downgrade();
        let exit = hover.clone();
        let tracker = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                // Recomputed on every move against the workspace's own flag,
                // so a capsule that remounts never keeps a stale hover.
                window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let over = hitbox.is_hovered(window) && !cx.has_active_drag();
                    let _ = hover.update(cx, |this, cx| {
                        if this.title_reveal.hovered != over {
                            this.title_reveal.hovered = over;
                            cx.notify();
                        }
                    });
                });
                // Leaving the window sends no move outside the capsule.
                window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = exit.update(cx, |this, cx| {
                        if this.title_reveal.hovered {
                            this.title_reveal.hovered = false;
                            cx.notify();
                        }
                    });
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let divider = div().w(px(1.)).h(px(18.)).flex_shrink_0().bg(rgb(p.border));
        let content = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            // Arrives behind the width and leaves ahead of it, so controls
            // show whole and the capsule never closes over visible ones.
            .opacity(smooth((progress - 0.4) / 0.5))
            .child(chrome::measure(measure, |this: &mut Self, bounds, _| {
                let width = &mut this.title_reveal.width;
                let changed = settle_width(*width, bounds.size.width);
                if changed {
                    *width = bounds.size.width;
                }
                changed
            }))
            .child(divider)
            .child(actions);
        // A bare capsule stretches a few pixels past its actions before
        // settling; grown around notices, its outline holds, so nothing does.
        let held = noticed.clamp(0., 1.);
        let part = progress.clamp(0., 1.);
        let width = natural * (part + (1. - held) * (progress.max(0.) - part));
        // What shows of the actions; until measured, all or nothing.
        let shown = |d: Div| {
            if natural > px(0.) {
                d.w(width)
            } else if progress < 1. {
                d.w(px(0.))
            } else {
                d
            }
        };
        // Beside notices the island keeps the actions' room and splits what
        // is hidden evenly either side of the content, as a bare capsule
        // shrinks about its center. While the island grows or shrinks
        // around notices, the room grows or shrinks with it.
        let hidden = natural * (1. - part);
        let side = hidden * noticed.max(0.) / 2.;
        let kept = noticed > 0. && natural > px(0.);
        let room = div()
            .id("title-actions")
            .debug_selector(|| "title-actions".into())
            .flex_shrink_0()
            .flex()
            .when(kept, |d| d.w(width + side))
            .child(
                shown(div())
                    .debug_selector(|| "title-actions-shown".into())
                    .flex_shrink_0()
                    .flex()
                    .overflow_hidden()
                    .child(content),
            );
        let bar = bar
            .track_focus(&self.title_reveal.focus)
            // The capsule only reports focus inside it: a click on its
            // surface leaves focus (and the composer's caret) where it was.
            // Runs after a pressed control has taken focus, so it can note it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.prevent_default();
                    this.title_reveal.pointer_focus = window.focused(cx).map(|f| f.downgrade());
                }),
            )
            // `pl_4` is a rem; the hidden half adds to it.
            .when(kept, |d| d.pl(window.rem_size() + side))
            .child(tracker)
            .child(room);
        let extra = if natural > px(0.) {
            width + if kept { side * 2. } else { px(0.) }
        } else {
            px(0.)
        };
        (bar, extra)
    }

    /// Whether a pointer click may act on a title action now; keyboard
    /// activation only happens once focus has revealed them.
    pub(super) fn title_action_allowed(&self, event: &ClickEvent) -> bool {
        matches!(event, ClickEvent::Keyboard(_)) || self.title_reveal.reveal.ready(Instant::now())
    }

    /// Where the reveal is headed, as the last render decided it.
    #[cfg(test)]
    pub(super) fn title_reveal_target(&self) -> bool {
        self.title_reveal.reveal.open
    }

    /// Holds an opening reveal part-way, for checks mid-animation.
    #[cfg(test)]
    pub(super) fn freeze_title_reveal(&mut self, progress: f32) {
        let reveal = &mut self.title_reveal.reveal;
        reveal.open = true;
        reveal.waits = false;
        reveal.spring.set(1., Instant::now());
        reveal
            .spring
            .hold(progress, Instant::now() + Duration::from_secs(3600));
    }

    /// Finishes the notice island's motion wherever it is headed.
    #[cfg(test)]
    pub(super) fn settle_island(&mut self) {
        self.island_motion.settle();
    }

    /// Holds the notice island part-grown, for checks mid-motion.
    #[cfg(test)]
    pub(super) fn freeze_island(&mut self, presence: f32) {
        self.island_motion.freeze(presence);
    }

    #[cfg(test)]
    pub(super) fn island_moving(&self) -> bool {
        self.island_motion.moving(Instant::now())
    }

    #[cfg(test)]
    pub(super) fn settle_title_reveal(&mut self) {
        let open = self.title_reveal.reveal.open;
        self.title_reveal.reveal = Reveal::snapped(open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn hover_waits_for_intent_then_springs_open_and_closes_after_grace() {
        let start = Instant::now();
        let mut reveal = Reveal::default();
        assert_eq!(reveal.progress(start), 0.);
        reveal.set(true, true, start);
        assert_eq!(reveal.progress(at(start, 60)), 0., "still intent");
        let early = reveal.progress(at(start, 120));
        let later = reveal.progress(at(start, 200));
        assert!(0. < early && early < later && later < 1.);
        assert!(!reveal.ready(at(start, 200)), "no clicks mid-reveal");
        // A little stretch past the actions, then home.
        let peak = (200..400)
            .map(|ms| reveal.progress(at(start, ms)))
            .fold(0f32, f32::max);
        assert!(1. < peak && peak < 1.03, "{peak}");
        assert!(!reveal.ready(at(start, 300)), "no clicks while it settles");
        assert!(reveal.ready(at(start, 380)));
        assert_eq!(reveal.progress(at(start, 380)), 1.);

        let leave = at(start, 500);
        reveal.set(false, true, leave);
        assert_eq!(reveal.progress(at(start, 700)), 1., "grace");
        assert!(reveal.progress(at(start, 800)) < 1.);
        assert!(reveal.settled(at(start, 1100)));
        assert_eq!(reveal.progress(at(start, 1100)), 0.);
    }

    #[test]
    fn reduced_motion_still_waits_for_intent_then_jumps() {
        let start = Instant::now();
        let mut reveal = Reveal::default();
        reveal.retune(false);
        reveal.set(true, true, start);
        assert_eq!(reveal.progress(at(start, 60)), 0.);
        assert!(reveal.ready(at(start, 70)));
        reveal.set(false, true, at(start, 100));
        assert_eq!(reveal.progress(at(start, 300)), 1., "grace");
        assert_eq!(reveal.progress(at(start, 320)), 0.);
        assert!(reveal.settled(at(start, 320)));
    }

    fn notice(slot: &'static str, text: &str, tone: Tone) -> Notice {
        Notice {
            slot,
            key: text.into(),
            text: text.into(),
            tone,
            dismissible: true,
        }
    }

    #[test]
    fn notices_grow_in_a_stagger_and_their_words_follow_the_shape() {
        let start = Instant::now();
        let mut island = IslandMotion::default();
        island.sync(&[], 1, true, start);
        assert!(
            !island.shows(start) && !island.moving(start),
            "bare at rest"
        );
        let both = [
            notice("status", "Model change refused", Tone::Error),
            notice("feature", "Saved", Tone::Success),
        ];
        island.sync(&both, 1, true, start);
        assert!(island.shows(start) && island.moving(start));
        for (slot, height) in [("status", 30.), ("feature", 24.)] {
            assert!(island.measured(slot, px(height), start));
        }
        let rows = island.rows();
        assert_eq!(rows[0].height(start), Some(px(0.)), "grows from nothing");
        let first = rows[0].height(at(start, 60)).unwrap();
        let second = rows[1].height(at(start, 60)).unwrap();
        assert!(first > second && second > px(0.), "a beat apart");
        assert!(
            rows[0].content(at(start, 60)).0 == 0.,
            "words wait on the shape"
        );
        assert!(rows[0].content(at(start, 200)).0 > 0.);
        // The most pressing tone greets them, briefly.
        assert!(matches!(island.glow(at(start, 100)), Some((Tone::Error, s)) if s > 0.));
        assert!(island.glow(at(start, 700)).is_none());
        let peak = (0..500)
            .map(|ms| f32::from(rows[0].height(at(start, ms)).unwrap_or(px(30.))))
            .fold(0f32, f32::max);
        assert!(peak > 30. && peak < 32., "a soft overshoot: {peak}");
        assert!(
            island.bulge(at(start, 80)) > 0.,
            "corners soften mid-stretch"
        );
        let rest = at(start, 1000);
        assert!(!island.moving(rest), "idle once settled");
        assert_eq!(
            island.rows()[0].height(rest),
            None,
            "natural height at rest"
        );
        assert_eq!(island.bulge(rest), 0.);
        assert_eq!(island.open(rest), 1.);
    }

    #[test]
    fn a_dismissed_row_fades_then_closes_and_the_island_lets_go() {
        let start = Instant::now();
        let mut island = IslandMotion::default();
        let status = [notice("status", "Change queued", Tone::Info)];
        island.sync(&status, 1, true, start);
        island.measured("status", px(30.), start);
        let gone = at(start, 1000);
        island.sync(&[], 1, true, gone);
        let row = &island.rows()[0];
        assert!(row.leaving && row.text == "Change queued");
        // Words leave first while the room holds, then the room closes.
        assert_eq!(row.height(at(start, 1060)), Some(px(30.)));
        assert!(row.content(at(start, 1060)).0 < 1.);
        assert_eq!(row.content(at(start, 1110)).0, 0.);
        assert!(row.height(at(start, 1200)).unwrap() < px(30.));
        assert!(island.shows(at(start, 1200)));
        let rest = at(start, 2000);
        island.sync(&[], 1, true, rest);
        assert!(island.rows().is_empty() && !island.shows(rest) && !island.moving(rest));
    }

    #[test]
    fn new_words_hold_their_room_until_measured_then_ease() {
        let start = Instant::now();
        let mut island = IslandMotion::default();
        island.sync(&[notice("status", "Saved", Tone::Success)], 1, true, start);
        island.measured("status", px(24.), start);
        let later = at(start, 1000);
        island.sync(
            &[notice("status", "A much longer refusal", Tone::Error)],
            1,
            true,
            later,
        );
        assert_eq!(
            island.rows()[0].height(later),
            Some(px(24.)),
            "no one-frame jump"
        );
        assert!(island.measured("status", px(48.), later));
        let mid = island.rows()[0].height(at(start, 1100)).unwrap();
        assert!(px(24.) < mid && mid < px(49.), "{mid:?}");
        assert!(!island.moving(at(start, 2000)));
        // A rewrap at rest (a resize) is already laid out: no catching up.
        assert!(island.measured("status", px(72.), at(start, 2000)));
        assert_eq!(island.rows()[0].height(at(start, 2000)), None);
        assert!(
            !island.measured("status", px(71.5), at(start, 2000)),
            "sub-pixel jitter"
        );
    }

    #[test]
    fn live_words_under_the_same_key_update_without_greeting_again() {
        let start = Instant::now();
        let mut island = IslandMotion::default();
        island.sync(&[], 1, true, start);
        let context = |key: &str, text: &str, tone| Notice {
            slot: "context",
            key: key.into(),
            text: text.into(),
            tone,
            dismissible: true,
        };
        island.sync(
            &[context("a@90", "Context 92% full", Tone::Warning)],
            1,
            true,
            start,
        );
        island.measured("context", px(30.), start);
        let rest = at(start, 1000);
        assert!(!island.moving(rest));
        island.sync(
            &[context("a@90", "Context 93% full", Tone::Warning)],
            1,
            true,
            rest,
        );
        assert_eq!(island.rows()[0].text, "Context 93% full");
        assert!(!island.moving(rest), "a tick is no news");
        assert_eq!(island.rows()[0].height(rest), None);
        island.sync(
            &[context("a@95", "Context 96% full", Tone::Error)],
            1,
            true,
            rest,
        );
        assert!(matches!(
            island.glow(at(start, 1100)),
            Some((Tone::Error, _))
        ));
    }

    #[test]
    fn chat_switches_and_reduced_motion_show_rows_where_they_belong() {
        let start = Instant::now();
        let mut island = IslandMotion::default();
        let status = [notice("status", "Refreshing conversation…", Tone::Loading)];
        // The first render, another chat, or reduced motion: no motion.
        island.sync(&status, 1, true, start);
        assert_eq!(island.rows()[0].height(start), None);
        assert!(!island.moving(start) && island.glow(start).is_none());
        island.sync(&[], 2, true, start);
        assert!(!island.shows(start), "nothing lingers from the last chat");
        island.sync(&status, 2, false, start);
        assert_eq!(island.rows()[0].height(start), None);
        assert_eq!(island.rows()[0].content(start), (1., px(0.)));
        island.sync(&[], 2, false, start);
        assert!(!island.shows(start) && !island.moving(start));
        // Loading never pulses.
        island.sync(&status, 2, true, start);
        assert!(island.glow(start).is_none());
    }

    #[test]
    fn reentering_within_grace_never_moves() {
        let start = Instant::now();
        let mut reveal = Reveal::snapped(true);
        reveal.set(false, true, start);
        reveal.set(true, true, at(start, 100));
        for ms in [100, 150, 300, 600] {
            assert_eq!(reveal.progress(at(start, ms)), 1., "{ms}ms");
        }
    }

    #[test]
    fn reversing_mid_way_turns_from_where_it_is() {
        let start = Instant::now();
        let mut reveal = Reveal::default();
        reveal.set(true, false, start);
        let mid = reveal.progress(at(start, 80));
        assert!(0. < mid && mid < 1.);
        reveal.set(false, true, at(start, 80));
        // Already moving: no grace pause and no jump.
        assert!((reveal.progress(at(start, 80)) - mid).abs() < 1e-6);
        assert!(reveal.progress(at(start, 120)) < mid);
    }

    #[test]
    fn sub_pixel_measurement_jitter_settles() {
        use gpui::px;
        assert!(settle_width(px(0.), px(277.5)));
        assert!(settle_width(px(277.5), px(278.)), "growth always counts");
        assert!(
            !settle_width(px(278.), px(277.5)),
            "half-pixel snap is jitter"
        );
        assert!(settle_width(px(278.), px(187.)), "a real shrink counts");
    }

    #[test]
    fn focus_cuts_a_hover_wait_short() {
        let start = Instant::now();
        let mut reveal = Reveal::default();
        reveal.set(true, true, start);
        reveal.set(true, false, at(start, 10));
        assert!(reveal.progress(at(start, 60)) > 0.);
        assert!(reveal.ready(at(start, 330)));
    }
}
