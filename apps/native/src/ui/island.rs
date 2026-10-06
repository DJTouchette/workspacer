//! The title capsule at rest and revealed. Like a Dynamic Island, the
//! conversation's secondary actions stay tucked away until the pointer rests
//! on the capsule, keyboard focus enters it, or a tap on the title pins it
//! open. Hidden actions are clipped to no width, so they have no hit area and
//! cannot be clicked by accident; they stay in Tab order, and focusing one
//! reveals them at once.
use super::*;
use gpui::{ClickEvent, DispatchPhase, MouseButton, MouseExitEvent, MouseMoveEvent};
use std::time::{Duration, Instant};

/// Hover intent: a pointer passing across the capsule does not open it.
const OPEN_DELAY: Duration = Duration::from_millis(70);
/// Grace before closing, so skimming an edge or briefly leaving is no flicker.
const CLOSE_GRACE: Duration = Duration::from_millis(220);
/// The reveal itself, eased out like the notice tray's fade.
const DURATION: Duration = Duration::from_millis(200);

/// Whether a fresh measurement of the actions should replace `current`.
/// The measured row sits inside a box sized from the last measurement, so
/// pixel snapping at fractional scales can shift it by half a pixel each
/// layout; taking every change would re-render forever. Growth always wins
/// (nothing is clipped), but a shrink must be a whole pixel to count.
fn settle_width(current: gpui::Pixels, measured: gpui::Pixels) -> bool {
    measured > current || current - measured >= gpui::px(1.)
}

/// A reversible, time-based reveal: progress 0 is the capsule at rest, 1 is
/// every action shown. Changing direction mid-way turns from where it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Reveal {
    open: bool,
    from: f32,
    at: Option<Instant>,
    /// Whether this change waits for intent (opening) or grace (closing).
    waits: bool,
}

impl Reveal {
    pub(super) fn progress(&self, now: Instant) -> f32 {
        let target = if self.open { 1. } else { 0. };
        let Some(at) = self.at else {
            return target;
        };
        let wait = match (self.waits, self.open) {
            (false, _) => Duration::ZERO,
            (true, true) => OPEN_DELAY,
            (true, false) => CLOSE_GRACE,
        };
        let moving = now.saturating_duration_since(at).saturating_sub(wait);
        let t = (moving.as_secs_f32() / DURATION.as_secs_f32()).min(1.);
        let eased = 1. - (1. - t).powi(3);
        self.from + (target - self.from) * eased
    }

    /// Head toward `open`. `hold` waits for intent or grace first, unless
    /// the capsule is already moving; a change without `hold` (keyboard
    /// focus, a tap) also cuts short a wait already under way.
    pub(super) fn set(&mut self, open: bool, hold: bool, now: Instant) {
        if open == self.open && (hold || !self.waits) {
            return;
        }
        let current = self.progress(now);
        *self = Self {
            open,
            from: current,
            at: Some(now),
            waits: hold && (current <= 0. || current >= 1.),
        };
    }

    /// Already at rest wherever it is headed.
    #[cfg(test)]
    fn snapped(open: bool) -> Self {
        Self {
            open,
            from: if open { 1. } else { 0. },
            at: None,
            waits: false,
        }
    }

    pub(super) fn settled(&self, now: Instant) -> bool {
        let target = if self.open { 1. } else { 0. };
        self.progress(now) == target
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

impl Workspace {
    /// Appends the capsule's secondary actions behind the reveal, and the
    /// hover and focus tracking that drives it. `noticed` means the island
    /// carries notices: it then keeps the actions' room at rest, with the
    /// capsule's content centered in it, so revealing them never changes
    /// the island's outline, rewraps a notice or moves one of its controls.
    pub(super) fn reveal_title_actions(
        &mut self,
        bar: Stateful<Div>,
        actions: Div,
        noticed: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let now = Instant::now();
        let state = &mut self.title_reveal;
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
            // Fades in a little behind the width, so controls arrive whole.
            .opacity(progress * progress)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        cx.defer(move |cx| {
                            let _ = measure.update(cx, |this, cx| {
                                let width = &mut this.title_reveal.width;
                                if settle_width(*width, bounds.size.width) {
                                    *width = bounds.size.width;
                                    cx.notify();
                                }
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(divider)
            .child(actions);
        // What shows of the actions; until measured, all or nothing.
        let shown = |d: Div| {
            if natural > px(0.) {
                d.w(natural * progress)
            } else if progress < 1. {
                d.w(px(0.))
            } else {
                d
            }
        };
        // Beside notices the island keeps the actions' room and splits what
        // is hidden evenly either side of the content, as a bare capsule
        // shrinks about its center.
        let hidden = natural * (1. - progress);
        let room = div()
            .id("title-actions")
            .debug_selector(|| "title-actions".into())
            .flex_shrink_0()
            .flex()
            .when(noticed && natural > px(0.), |d| d.w(natural - hidden / 2.))
            .child(
                shown(div())
                    .debug_selector(|| "title-actions-shown".into())
                    .flex_shrink_0()
                    .flex()
                    .overflow_hidden()
                    .child(content),
            );
        bar.track_focus(&self.title_reveal.focus)
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
            .when(noticed, |d| d.pl(window.rem_size() + hidden / 2.))
            .child(tracker)
            .child(room)
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
        self.title_reveal.reveal = Reveal {
            open: true,
            from: progress,
            at: Some(Instant::now() + Duration::from_secs(3600)),
            waits: false,
        };
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
    fn hover_waits_for_intent_then_eases_open_and_closes_after_grace() {
        let start = Instant::now();
        let mut reveal = Reveal::default();
        assert_eq!(reveal.progress(start), 0.);
        reveal.set(true, true, start);
        assert_eq!(reveal.progress(at(start, 60)), 0., "still intent");
        let early = reveal.progress(at(start, 120));
        let later = reveal.progress(at(start, 200));
        assert!(0. < early && early < later && later < 1.);
        assert!(!reveal.ready(at(start, 200)), "no clicks mid-reveal");
        assert!(reveal.ready(at(start, 270)));

        let leave = at(start, 400);
        reveal.set(false, true, leave);
        assert_eq!(reveal.progress(at(start, 600)), 1., "grace");
        assert!(reveal.progress(at(start, 700)) < 1.);
        assert!(reveal.settled(at(start, 820)));
        assert_eq!(reveal.progress(at(start, 820)), 0.);
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
        assert!(reveal.ready(at(start, 210)));
    }
}
