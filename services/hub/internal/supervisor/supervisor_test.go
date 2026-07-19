package supervisor

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// capture is a non-blocking Publisher that records events to a buffered channel.
type capture struct{ ch chan event.Envelope }

func newCapture() *capture { return &capture{ch: make(chan event.Envelope, 256)} }

func (c *capture) Publish(ev event.Envelope) {
	select {
	case c.ch <- ev:
	default:
	}
}

func (c *capture) waitFor(t *testing.T, typ string) event.Envelope {
	t.Helper()
	timeout := time.After(3 * time.Second)
	for {
		select {
		case ev := <-c.ch:
			if ev.Type == typ {
				return ev
			}
		case <-timeout:
			t.Fatalf("timed out waiting for %q", typ)
		}
	}
}

func TestStartStop(t *testing.T) {
	cap := newCapture()
	s := New(Spec{Name: "sleeper", Command: "sleep", Args: []string{"30"}}, cap)
	s.Start()
	cap.waitFor(t, "sidecar.running")
	if s.State() != Running {
		t.Fatalf("state=%s", s.State())
	}
	s.Stop()
	cap.waitFor(t, "sidecar.stopped")
	if s.State() != Stopped {
		t.Fatalf("state after stop=%s", s.State())
	}
}

func TestRestartsOnUnexpectedExit(t *testing.T) {
	cap := newCapture()
	s := New(Spec{
		Name:        "flaky",
		Command:     "sh",
		Args:        []string{"-c", "exit 1"},
		RestartWait: 50 * time.Millisecond,
	}, cap)
	s.Start()
	defer s.Stop()

	// First run, crash, then a restart (second run).
	cap.waitFor(t, "sidecar.running")
	cap.waitFor(t, "sidecar.crashed")
	cap.waitFor(t, "sidecar.running")
}

func TestNextBackoff(t *testing.T) {
	limit := 30 * time.Second
	cases := []struct {
		cur, want time.Duration
	}{
		{1 * time.Second, 2 * time.Second},
		{2 * time.Second, 4 * time.Second},
		{16 * time.Second, 30 * time.Second}, // 32s clamps to the 30s cap
		{30 * time.Second, 30 * time.Second}, // already at cap, stays
		{1 << 62, limit},                     // overflow guard clamps to cap
	}
	for _, c := range cases {
		if got := nextBackoff(c.cur, limit); got != c.want {
			t.Errorf("nextBackoff(%v, %v) = %v, want %v", c.cur, limit, got, c.want)
		}
	}
}

// runningGaps records the wall-clock time of each sidecar.running event.
func (c *capture) runningGaps(t *testing.T, n int) []time.Duration {
	t.Helper()
	times := make([]time.Time, 0, n)
	timeout := time.After(5 * time.Second)
	for len(times) < n {
		select {
		case ev := <-c.ch:
			if ev.Type == "sidecar.running" {
				times = append(times, time.Now())
			}
		case <-timeout:
			t.Fatalf("timed out waiting for %d running events, got %d", n, len(times))
		}
	}
	gaps := make([]time.Duration, 0, n-1)
	for i := 1; i < len(times); i++ {
		gaps = append(gaps, times[i].Sub(times[i-1]))
	}
	return gaps
}

// A child that keeps crashing must back off exponentially, not restart on a
// fixed delay — so the gap between successive restarts grows.
func TestRestartBackoffEscalates(t *testing.T) {
	cap := newCapture()
	s := New(Spec{
		Name:              "crasher",
		Command:           "sh",
		Args:              []string{"-c", "exit 1"},
		RestartWait:       120 * time.Millisecond,
		MaxRestartWait:    5 * time.Second,
		RestartResetAfter: 10 * time.Second, // never resets during this fast test
	}, cap)
	s.Start()
	defer s.Stop()

	// Three restarts → two inter-restart gaps: ~120ms then ~240ms.
	gaps := cap.runningGaps(t, 3)
	if gaps[1] <= gaps[0] {
		t.Fatalf("expected escalating backoff, got gaps %v then %v", gaps[0], gaps[1])
	}
	// The second gap should be near double the first (allow generous scheduling
	// slack): at least 1.5x rather than the fixed-delay 1x.
	if gaps[1] < gaps[0]*3/2 {
		t.Fatalf("second gap %v should be ~2x the first %v (fixed-delay regression?)", gaps[1], gaps[0])
	}
}

// A restart backoff must reset after the child stays up long enough, so a
// transient crash after a healthy run doesn't inherit a long delay.
func TestRestartBackoffResetsAfterHealthyUptime(t *testing.T) {
	// resetAfter is tiny, so every run (each ~immediately crashing after a short
	// sleep) counts as "healthy" and the backoff never escalates — successive
	// gaps stay near the base delay.
	cap := newCapture()
	s := New(Spec{
		Name:              "blip",
		Command:           "sh",
		Args:              []string{"-c", "sleep 0.15; exit 1"},
		RestartWait:       80 * time.Millisecond,
		MaxRestartWait:    5 * time.Second,
		RestartResetAfter: 50 * time.Millisecond, // 150ms uptime > this → reset
	}, cap)
	s.Start()
	defer s.Stop()

	gaps := cap.runningGaps(t, 3)
	// Both gaps ≈ uptime(150ms)+base(80ms); the second must not have escalated to
	// ~2x base beyond the first. Assert they stay within a tight band.
	if gaps[1] > gaps[0]*2 {
		t.Fatalf("backoff should have reset after healthy uptime; gaps %v then %v", gaps[0], gaps[1])
	}
}

// waitForLog waits for a plugin.log event on the given stream carrying the given
// line, draining other events. It fails the test on timeout.
func (c *capture) waitForLog(t *testing.T, stream, line string) {
	t.Helper()
	timeout := time.After(3 * time.Second)
	for {
		select {
		case ev := <-c.ch:
			if ev.Type != "plugin.log" {
				continue
			}
			var d logData
			if err := json.Unmarshal(ev.Data, &d); err != nil {
				t.Fatalf("unmarshal plugin.log: %v", err)
			}
			if d.Stream == stream && d.Line == line {
				return
			}
		case <-timeout:
			t.Fatalf("timed out waiting for plugin.log stream=%q line=%q", stream, line)
		}
	}
}

// With LogLines set, a sidecar's stdout/stderr is published line-by-line as
// plugin.log events (this is what `workspacer plugin dev` prints).
func TestLogLinesStreamsOutput(t *testing.T) {
	cap := newCapture()
	s := New(Spec{
		Name:     "logger",
		Command:  "sh",
		Args:     []string{"-c", `printf "hello\n"; printf "oops\n" 1>&2`},
		LogLines: true,
	}, cap)
	s.Start()
	defer s.Stop()

	cap.waitForLog(t, "stdout", "hello")
	cap.waitForLog(t, "stderr", "oops")
}

// Without LogLines, no plugin.log events are published — production `serve` must
// not stream sidecar output onto the bus.
func TestLogLinesOffPublishesNothing(t *testing.T) {
	cap := newCapture()
	s := New(Spec{
		Name:    "quiet",
		Command: "sh",
		Args:    []string{"-c", `printf "hello\n"; printf "oops\n" 1>&2`},
		// LogLines defaults to false.
	}, cap)
	s.Start()

	// The child exits immediately; wait for it to run then stop so all its output
	// has been processed before we assert nothing was logged.
	cap.waitFor(t, "sidecar.running")
	s.Stop()

	for {
		select {
		case ev := <-cap.ch:
			if ev.Type == "plugin.log" {
				t.Fatalf("plugin.log published with LogLines off: %s", string(ev.Data))
			}
		default:
			return // drained the buffer, no plugin.log seen
		}
	}
}

func TestHealthTransitions(t *testing.T) {
	var healthy atomic.Bool
	healthy.Store(true)
	hs := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		if healthy.Load() {
			w.WriteHeader(200)
		} else {
			w.WriteHeader(500)
		}
	}))
	defer hs.Close()

	cap := newCapture()
	s := New(Spec{
		Name:         "svc",
		Command:      "sleep",
		Args:         []string{"30"},
		HealthURL:    hs.URL,
		HealthPeriod: 30 * time.Millisecond,
	}, cap)
	s.Start()
	defer s.Stop()

	cap.waitFor(t, "sidecar.healthy")
	healthy.Store(false)
	cap.waitFor(t, "sidecar.unhealthy")
}
