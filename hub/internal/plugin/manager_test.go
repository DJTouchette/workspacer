package plugin

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

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
	deadline := time.After(3 * time.Second)
	for {
		select {
		case ev := <-c.ch:
			if ev.Type == typ {
				return ev
			}
		case <-deadline:
			t.Fatalf("timed out waiting for %q", typ)
		}
	}
}

func TestManagerMetadataOnlyPlugin(t *testing.T) {
	cap := newCapture()
	m := NewManager(cap)
	mf := Manifest{
		ID: "acme.dash", APIVersion: "1",
		Panes: []PaneContribution{{Type: "acme.dash", Title: "Dash"}},
	}
	m.Add(mf)

	ev := cap.waitFor(t, "plugin.loaded")
	var got Manifest
	if err := json.Unmarshal(ev.Data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.ID != "acme.dash" || len(got.Panes) != 1 {
		t.Fatalf("event payload = %+v", got)
	}
	if list := m.List(); len(list) != 1 || list[0].ID != "acme.dash" {
		t.Fatalf("List = %+v", list)
	}

	m.Remove("acme.dash")
	cap.waitFor(t, "plugin.unloaded")
	if len(m.List()) != 0 {
		t.Fatal("List should be empty after Remove")
	}
}

// A plugin WITH a server gets a real supervised sidecar.
func TestManagerSpawnsSidecar(t *testing.T) {
	cap := newCapture()
	m := NewManager(cap)
	m.Add(Manifest{
		ID: "acme.svc", APIVersion: "1",
		Server: &ServerSpec{Command: "sleep", Args: []string{"30"}},
	})
	defer m.Stop()

	cap.waitFor(t, "plugin.loaded")
	cap.waitFor(t, "sidecar.running") // the supervisor actually started it

	// Give State a moment to settle, then assert it's running.
	deadline := time.After(2 * time.Second)
	for m.State("acme.svc") != "running" {
		select {
		case <-deadline:
			t.Fatalf("sidecar state = %q", m.State("acme.svc"))
		case <-time.After(10 * time.Millisecond):
		}
	}
}
