package plugin

import (
	"sync"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/sandbox"
)

// recorder collects published events for assertions.
type recorder struct {
	mu     sync.Mutex
	events []event.Envelope
}

func (r *recorder) Publish(ev event.Envelope) {
	r.mu.Lock()
	r.events = append(r.events, ev)
	r.mu.Unlock()
}

func (r *recorder) has(t string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, e := range r.events {
		if e.Type == t {
			return true
		}
	}
	return false
}

func (r *recorder) count() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.events)
}

func sidecarManifest() Manifest {
	return Manifest{ID: "p", Dir: "/plugins/p", Server: &ServerSpec{Command: "/bin/srv", Args: []string{"--port", "9"}}}
}

func TestSandboxSidecar_Off(t *testing.T) {
	rec := &recorder{}
	m := NewManager(rec, nil)
	m.SetSandboxMode(sandbox.ModeOff)
	mf := sidecarManifest()

	cmd, args, run := m.sandboxSidecar(mf)
	if !run || cmd != "/bin/srv" || len(args) != 2 {
		t.Fatalf("off: got cmd=%q args=%v run=%v, want original + run", cmd, args, run)
	}
	if rec.count() != 0 {
		t.Errorf("off mode should emit no sandbox event, got %d", rec.count())
	}
}

func TestSandboxSidecar_BestEffort(t *testing.T) {
	rec := &recorder{}
	m := NewManager(rec, nil)
	m.SetSandboxMode(sandbox.ModeBestEffort)
	mf := sidecarManifest()

	// What the platform can actually do right now.
	res := sandbox.Wrap(mf.Server.Command, mf.Server.Args, sandbox.Policy{WriteRoots: []string{mf.Dir}})

	cmd, _, run := m.sandboxSidecar(mf)
	if !run {
		t.Fatal("best-effort must always start the sidecar")
	}
	if res.Available {
		if cmd != res.Path {
			t.Errorf("available: got cmd %q, want wrapped %q", cmd, res.Path)
		}
		if !rec.has("plugin.sandboxed") {
			t.Error("expected plugin.sandboxed event")
		}
	} else {
		if cmd != mf.Server.Command {
			t.Errorf("unavailable: got cmd %q, want original", cmd)
		}
		if !rec.has("plugin.unsandboxed") {
			t.Error("expected plugin.unsandboxed event")
		}
	}
}

func TestSandboxSidecar_Enforce(t *testing.T) {
	rec := &recorder{}
	m := NewManager(rec, nil)
	m.SetSandboxMode(sandbox.ModeEnforce)
	mf := sidecarManifest()

	res := sandbox.Wrap(mf.Server.Command, mf.Server.Args, sandbox.Policy{WriteRoots: []string{mf.Dir}})

	cmd, _, run := m.sandboxSidecar(mf)
	// Enforce starts the sidecar iff a confinement mechanism is available.
	if run != res.Available {
		t.Fatalf("enforce: run=%v, want %v (mechanism available=%v)", run, res.Available, res.Available)
	}
	if res.Available {
		if cmd != res.Path || !rec.has("plugin.sandboxed") {
			t.Errorf("enforce+available: cmd=%q, want wrapped + sandboxed event", cmd)
		}
	} else {
		if run {
			t.Error("enforce without a mechanism must refuse to start")
		}
		if !rec.has("plugin.sandbox.refused") {
			t.Error("expected plugin.sandbox.refused event")
		}
	}
}
