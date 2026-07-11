package main

import "time"

// restartBackoff is the pure bookkeeping for restarting a crashed child — a
// direct port of the desktop's RestartBackoff (apps/desktop daemonUtils.ts) so
// headless supervision behaves like the app users already know: exponential
// delays, a clean slate after a healthy stretch, and a hard give-up after
// repeated consecutive failures (unlike internal/supervisor, which retries
// forever — a headless server should stop hammering a broken binary and say
// so). The caller owns the actual spawn + timer; `now` is injectable so tests
// never sleep.
type restartBackoff struct {
	base        time.Duration // delay before the first restart
	max         time.Duration // backoff cap
	maxAttempts int           // give up after this many consecutive failures
	resetAfter  time.Duration // uptime that counts as healthy → counter resets
	now         func() time.Time

	attempts  int
	startedAt time.Time
}

func newRestartBackoff() *restartBackoff {
	return &restartBackoff{
		base:        time.Second,
		max:         30 * time.Second,
		maxAttempts: 10,
		resetAfter:  time.Minute,
		now:         time.Now,
	}
}

// markStarted records a (re)spawn, for the uptime-reset heuristic.
func (b *restartBackoff) markStarted() {
	b.startedAt = b.now()
}

// nextDelay returns the wait before the next restart, or ok=false once the
// failure budget is exhausted.
func (b *restartBackoff) nextDelay() (time.Duration, bool) {
	// A run that stayed up past resetAfter was genuinely healthy — this crash
	// starts a fresh escalation instead of inheriting the old crash-loop's tail.
	if !b.startedAt.IsZero() && b.now().Sub(b.startedAt) >= b.resetAfter {
		b.attempts = 0
	}
	if b.attempts >= b.maxAttempts {
		return 0, false
	}
	d := b.base << b.attempts // base * 2^attempts
	if d <= 0 || d > b.max {  // <=0 guards shift overflow
		d = b.max
	}
	b.attempts++
	return d, true
}
