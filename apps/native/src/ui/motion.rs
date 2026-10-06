//! Springs for chrome that changes shape. A spring is solved in closed form
//! from where and how fast it was when last retargeted, so it needs no
//! per-frame state: a render samples it at `now`, and it stops asking for
//! frames once it is within `rest` of its target, where it then sits exactly.
//! Retargeting mid-flight keeps position and velocity, so a reversal turns
//! with momentum instead of jumping.
use std::time::{Duration, Instant};

/// How a spring moves: `response` is the undamped period in seconds (lower
/// is quicker) and `damping` the damping ratio (below 1 overshoots a little
/// before settling). A zero response jumps straight to the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tune {
    pub response: f32,
    pub damping: f32,
}

impl Tune {
    /// No motion: values jump to their targets (reduced motion).
    pub const INSTANT: Self = Self {
        response: 0.,
        damping: 1.,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Spring {
    tune: Tune,
    target: f32,
    from: f32,
    velocity: f32,
    /// When the motion from `from` starts; it may lie ahead (a delayed
    /// start, during which the value holds). `None` rests on the target.
    at: Option<Instant>,
    /// Within this of the target counts as arrived, in the value's units.
    rest: f32,
}

impl Spring {
    pub(super) fn new(tune: Tune, value: f32, rest: f32) -> Self {
        Self {
            tune,
            target: value,
            from: value,
            velocity: 0.,
            at: None,
            rest,
        }
    }

    pub(super) fn target(&self) -> f32 {
        self.target
    }

    /// The displacement terms of the solution: x(t) = target + e^(-zwt)(a cos + b sin).
    fn terms(&self) -> (f32, f32, f32, f32) {
        let omega = std::f32::consts::TAU / self.tune.response;
        let zeta = self.tune.damping.clamp(0.05, 0.999);
        let damped = omega * (1. - zeta * zeta).sqrt();
        let a = self.from - self.target;
        let b = (self.velocity + zeta * omega * a) / damped;
        (zeta * omega, damped, a, b)
    }

    /// Seconds from the start until the swing stays within `rest`.
    fn settle_time(&self) -> f32 {
        if self.tune.response <= 0. {
            return 0.;
        }
        let (decay, _, a, b) = self.terms();
        let swing = (a * a + b * b).sqrt();
        if swing <= self.rest {
            0.
        } else {
            (swing / self.rest).ln() / decay
        }
    }

    /// Value and velocity (units per second) at `now`.
    pub(super) fn sample(&self, now: Instant) -> (f32, f32) {
        let Some(at) = self.at else {
            return (self.target, 0.);
        };
        if now < at {
            return (self.from, 0.);
        }
        let t = (now - at).as_secs_f32();
        if t >= self.settle_time() {
            return (self.target, 0.);
        }
        let (decay, damped, a, b) = self.terms();
        let envelope = (-decay * t).exp();
        let (sin, cos) = (damped * t).sin_cos();
        let value = self.target + envelope * (a * cos + b * sin);
        let velocity = envelope * ((b * damped - decay * a) * cos - (a * damped + decay * b) * sin);
        (value, velocity)
    }

    pub(super) fn value(&self, now: Instant) -> f32 {
        self.sample(now).0
    }

    /// Resting on its target, with no delayed start pending.
    pub(super) fn settled(&self, now: Instant) -> bool {
        match self.at {
            None => true,
            // Already there: a pending start has nowhere to go.
            Some(_) if self.from == self.target && self.velocity == 0. => true,
            Some(at) => now >= at + Duration::from_secs_f32(self.settle_time()),
        }
    }

    /// Heads for `target` from wherever it is now, keeping its momentum.
    pub(super) fn set(&mut self, target: f32, now: Instant) {
        self.set_after(target, now, Duration::ZERO);
    }

    /// As `set`, but a spring at rest first waits `delay`; one already
    /// moving turns at once, since holding it would stall it mid-air.
    pub(super) fn set_after(&mut self, target: f32, now: Instant, delay: Duration) {
        if target == self.target {
            return;
        }
        let (from, velocity) = self.sample(now);
        let delay = if velocity == 0. {
            delay
        } else {
            Duration::ZERO
        };
        *self = Self {
            target,
            from,
            velocity,
            at: Some(now + delay),
            ..*self
        };
    }

    /// Restarts toward the same target with no wait, from where it is.
    pub(super) fn hurry(&mut self, now: Instant) {
        let (from, velocity) = self.sample(now);
        self.from = from;
        self.velocity = velocity;
        self.at = Some(now);
    }

    /// Jumps to `value` and rests there.
    pub(super) fn snap(&mut self, value: f32) {
        self.target = value;
        self.from = value;
        self.velocity = 0.;
        self.at = None;
    }

    pub(super) fn retune(&mut self, tune: Tune) {
        self.tune = tune;
    }

    /// Holds at `value` from now until `until`, then heads for the target;
    /// for tests that need a frame mid-motion.
    #[cfg(test)]
    pub(super) fn hold(&mut self, value: f32, until: Instant) {
        self.from = value;
        self.velocity = 0.;
        self.at = Some(until);
    }
}

/// Smoothstep of `t` clamped to 0..1: content fades that ease both ends.
pub(super) fn smooth(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TUNE: Tune = Tune {
        response: 0.3,
        damping: 0.75,
    };

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn overshoots_a_little_then_rests_exactly_on_target() {
        let start = Instant::now();
        let mut spring = Spring::new(TUNE, 0., 0.002);
        spring.set(1., start);
        let peak = (0..600)
            .map(|ms| spring.value(at(start, ms)))
            .fold(0f32, f32::max);
        assert!(peak > 1.01 && peak < 1.06, "slight overshoot: {peak}");
        let settle = (0..2000).find(|&ms| spring.settled(at(start, ms))).unwrap();
        assert!((300..700).contains(&settle), "settles in {settle}ms");
        assert_eq!(spring.value(at(start, settle)), 1.);
        assert_eq!(spring.sample(at(start, settle)).1, 0.);
    }

    #[test]
    fn retargeting_keeps_position_and_momentum() {
        let start = Instant::now();
        let mut spring = Spring::new(TUNE, 0., 0.002);
        spring.set(1., start);
        let (x, v) = spring.sample(at(start, 60));
        assert!(x > 0. && v > 0.);
        spring.set(0., at(start, 60));
        let (y, w) = spring.sample(at(start, 60));
        assert!((x - y).abs() < 1e-5 && (v - w).abs() < 1e-3);
        // Carried on a moment by its momentum, then back home.
        assert!(spring.value(at(start, 64)) > x);
        assert!(spring.settled(at(start, 1000)));
        assert_eq!(spring.value(at(start, 1000)), 0.);
    }

    #[test]
    fn a_delayed_start_holds_and_instant_tune_jumps() {
        let start = Instant::now();
        let mut spring = Spring::new(TUNE, 0., 0.002);
        spring.set_after(1., start, Duration::from_millis(100));
        assert_eq!(spring.value(at(start, 90)), 0.);
        assert!(!spring.settled(at(start, 90)));
        assert!(spring.value(at(start, 150)) > 0.);

        let mut instant = Spring::new(Tune::INSTANT, 0., 0.002);
        instant.set_after(1., start, Duration::from_millis(50));
        assert_eq!(instant.value(at(start, 40)), 0.);
        assert_eq!(instant.value(at(start, 50)), 1.);
        assert!(instant.settled(at(start, 50)));
    }

    #[test]
    fn rests_cost_nothing() {
        let start = Instant::now();
        let mut spring = Spring::new(TUNE, 0.5, 0.002);
        assert!(spring.settled(start));
        spring.set(0.5, start);
        assert!(spring.settled(start), "same target is no motion");
        spring.snap(1.);
        assert_eq!(spring.value(start), 1.);
    }
}
