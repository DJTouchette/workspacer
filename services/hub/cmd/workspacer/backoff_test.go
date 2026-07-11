package main

import (
	"testing"
	"time"
)

// fakeClock lets tests advance time explicitly instead of sleeping.
type fakeClock struct{ t time.Time }

func (c *fakeClock) now() time.Time          { return c.t }
func (c *fakeClock) advance(d time.Duration) { c.t = c.t.Add(d) }
func newFakeClock() *fakeClock               { return &fakeClock{t: time.Unix(1000, 0)} }
func testBackoff(clock *fakeClock) *restartBackoff {
	b := newRestartBackoff()
	b.now = clock.now
	return b
}

func TestRestartBackoffEscalatesAndCaps(t *testing.T) {
	clock := newFakeClock()
	b := testBackoff(clock)

	// Immediate consecutive crashes: 1s, 2s, 4s, … capped at 30s.
	want := []time.Duration{
		time.Second, 2 * time.Second, 4 * time.Second, 8 * time.Second,
		16 * time.Second, 30 * time.Second, 30 * time.Second,
	}
	for i, w := range want {
		b.markStarted()
		d, ok := b.nextDelay()
		if !ok {
			t.Fatalf("gave up at attempt %d, before the budget", i)
		}
		if d != w {
			t.Errorf("attempt %d delay = %v, want %v", i, d, w)
		}
	}
}

func TestRestartBackoffGivesUp(t *testing.T) {
	clock := newFakeClock()
	b := testBackoff(clock)
	for i := 0; i < b.maxAttempts; i++ {
		b.markStarted()
		if _, ok := b.nextDelay(); !ok {
			t.Fatalf("gave up early at attempt %d", i)
		}
	}
	b.markStarted()
	if _, ok := b.nextDelay(); ok {
		t.Errorf("still restarting after %d consecutive failures", b.maxAttempts)
	}
}

func TestRestartBackoffResetsAfterHealthyUptime(t *testing.T) {
	clock := newFakeClock()
	b := testBackoff(clock)

	// Burn several attempts, then stay up past resetAfter: the next crash must
	// start a fresh escalation at the base delay, not inherit the long tail.
	for i := 0; i < 5; i++ {
		b.markStarted()
		if _, ok := b.nextDelay(); !ok {
			t.Fatal("unexpected give-up")
		}
	}
	b.markStarted()
	clock.advance(b.resetAfter + time.Second)
	d, ok := b.nextDelay()
	if !ok {
		t.Fatal("gave up after a healthy run")
	}
	if d != b.base {
		t.Errorf("post-healthy delay = %v, want base %v", d, b.base)
	}
}

func TestRestartBackoffShortUptimeDoesNotReset(t *testing.T) {
	clock := newFakeClock()
	b := testBackoff(clock)

	b.markStarted()
	if _, ok := b.nextDelay(); !ok {
		t.Fatal("unexpected give-up")
	}
	b.markStarted()
	clock.advance(b.resetAfter / 2) // crashed again before "healthy"
	d, ok := b.nextDelay()
	if !ok {
		t.Fatal("unexpected give-up")
	}
	if d != 2*b.base {
		t.Errorf("delay = %v, want escalated %v", d, 2*b.base)
	}
}
