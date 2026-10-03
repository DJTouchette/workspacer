//! Smooth mouse-wheel scrolling for the conversation. GPUI applies a wheel
//! notch as one instant jump; touchpads already send fine pixel deltas. Wheel
//! notches arrive as `ScrollDelta::Lines` on Windows, Wayland and X11, so the
//! chat takes those in the capture phase and eases the list there over a few
//! frames, with the same clamping and follow-the-tail rules as keyboard paging.
use super::*;
use gpui::{DispatchPhase, ScrollDelta, ScrollWheelEvent};
use std::time::{Duration, Instant};

/// Distance per wheel line; platforms send three lines per notch.
const LINE: f32 = 32.;
/// Easing time constant: most of a notch lands within ~150ms.
const TAU: f32 = 0.06;
const FRAME: Duration = Duration::from_millis(8);

thread_local! {
    /// Bounds of scrollable panels inside messages, recorded as they lay out
    /// this frame; the wheel over them stays native so they keep scrolling.
    static ZONES: std::cell::RefCell<Vec<gpui::Bounds<gpui::Pixels>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Wrap a scrollable panel that can appear inside a message.
pub(super) fn scroll_zone(content: impl IntoElement) -> Div {
    div().relative().child(content).child(
        canvas(
            |bounds, _, _| ZONES.with(|zones| zones.borrow_mut().push(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

#[derive(Default)]
pub(super) struct SmoothScroll {
    /// Wheel sign: positive moves toward older messages.
    pending: f32,
    task: Option<Task<()>>,
}

impl SmoothScroll {
    #[cfg(test)]
    pub(super) fn gliding(&self) -> bool {
        self.task.is_some()
    }

    /// Add a notch; reversing direction drops what was still in flight so
    /// the change of mind takes effect at once.
    fn push(&mut self, delta: f32) {
        if self.pending != 0. && self.pending.signum() != delta.signum() {
            self.pending = 0.;
        }
        self.pending += delta;
    }

    /// The share of the remaining distance to apply after `dt` seconds;
    /// snaps the last fraction of a pixel so the stream ends.
    fn step(&mut self, dt: f32) -> f32 {
        let mut step = self.pending * (1. - (-dt / TAU).exp());
        if (self.pending - step).abs() < 0.5 {
            step = self.pending;
        }
        self.pending -= step;
        step
    }
}

impl Workspace {
    /// Registers the capture-phase wheel handler for this frame.
    pub(super) fn wheel_smoother(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                // Panels inside the list laid out before this sibling.
                let zones = ZONES.with(|zones| std::mem::take(&mut *zones.borrow_mut()));
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    let ScrollDelta::Lines(lines) = event.delta else {
                        return;
                    };
                    if phase != DispatchPhase::Capture
                        || lines.y == 0.
                        || lines.x != 0.
                        || zones.iter().any(|zone| zone.contains(&event.position))
                    {
                        return;
                    }
                    let taken = entity
                        .update(cx, |this, cx| {
                            let over_chat = this.list.viewport_bounds().contains(&event.position)
                                && !this.composer_dock_bounds.contains(&event.position)
                                && !this.usage_open
                                && !this.viewer_modal(window)
                                && this.screen == Screen::Conversation
                                && !this.new_session;
                            if over_chat {
                                this.smooth_scroll.push(lines.y * f32::from(px(LINE)));
                                this.run_smooth_scroll(window, cx);
                            }
                            over_chat
                        })
                        .unwrap_or(false);
                    if taken {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn run_smooth_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.smooth_scroll.task.is_some() {
            return;
        }
        self.smooth_scroll.task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut last = Instant::now();
            loop {
                cx.background_executor().timer(FRAME).await;
                let now = Instant::now();
                let dt = (now - last).as_secs_f32().min(0.05);
                last = now;
                let done = this
                    .update(cx, |this, cx| {
                        let step = this.smooth_scroll.step(dt);
                        this.apply_wheel_step(step);
                        cx.notify();
                        let done = this.smooth_scroll.pending == 0.;
                        if done {
                            this.smooth_scroll.task = None;
                        }
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }

    /// Move the transcript by one eased step (wheel sign: positive = older).
    fn apply_wheel_step(&mut self, step: f32) {
        if step == 0. {
            return;
        }
        if step > 0. {
            // Same normalization keyboard paging uses before leaving the tail.
            self.pause_follow();
            self.list.scroll_by(gpui::px(-step));
            return;
        }
        if self.follow {
            return; // Already pinned to the newest message.
        }
        let top = self.header_bounds.size.height + px(16.);
        let bottom = self.composer_dock_bounds.size.height + px(12.);
        let max = self.list.max_offset_for_scrollbar().height + top + bottom;
        let offset = -self.list.scroll_px_offset_for_scrollbar().y;
        if offset - gpui::px(step) >= max - gpui::px(0.5) {
            self.follow = true;
            self.smooth_scroll.pending = 0.;
            self.list.scroll_to(ListOffset {
                item_ix: self.list.item_count(),
                offset_in_item: gpui::px(0.),
            });
        } else {
            self.list.scroll_by(gpui::px(-step));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_out_lands_exactly_and_reverses_at_once() {
        let mut s = SmoothScroll::default();
        s.push(96.);
        let first = s.step(0.008);
        assert!(first > 0. && first < 96. * 0.2, "starts gently: {first}");
        let mut total = first;
        let mut frames = 1;
        while s.pending != 0. {
            total += s.step(0.008);
            frames += 1;
            assert!(frames < 200, "the stream ends");
        }
        assert!((total - 96.).abs() < 1e-3, "lands on the distance: {total}");
        assert!(frames * 8 <= 500, "settles quickly: {frames} frames");
        s.push(96.);
        s.step(0.008);
        s.push(-32.);
        assert_eq!(s.pending, -32., "a reversal drops the stale remainder");
    }
}
