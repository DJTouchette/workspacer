package supervisor

import (
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
