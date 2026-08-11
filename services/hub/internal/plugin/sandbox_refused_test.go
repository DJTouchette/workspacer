package plugin

// WORKSPACER_PLUGIN_SANDBOX=enforce on a host with no confinement mechanism
// (Linux without bubblewrap) is a PERMANENT decision: the sidecar is never
// started, no supervisor is constructed, and therefore no sidecar.running or
// sidecar.crashed event is ever emitted for that plugin. The Plugins pane
// derives its state from those events alone and falls back to "starting", so
// the user is shown an in-progress label for a process that will never start.
//
// Enforce logged nothing at all; best-effort on the SAME host logs a 300-byte
// warning for a situation that still runs.

import (
	"bytes"
	"log"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/sandbox"
)

type refusePub struct{ events []event.Envelope }

func (p *refusePub) Publish(e event.Envelope) { p.events = append(p.events, e) }

func captureLogTo(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	prevOut, prevFlags := log.Writer(), log.Flags()
	log.SetOutput(&buf)
	log.SetFlags(0)
	t.Cleanup(func() {
		log.SetOutput(prevOut)
		log.SetFlags(prevFlags)
	})
	return &buf
}

func TestARefusedSidecarIsSaidOutLoud(t *testing.T) {
	// No PATH ⇒ no bubblewrap/sandbox-exec ⇒ no mechanism available.
	t.Setenv("PATH", "")
	pub := &refusePub{}
	m := &Manager{pub: pub, sandboxMode: sandbox.ModeEnforce}
	mf := Manifest{ID: "p", Dir: t.TempDir(), Server: &ServerSpec{Command: "/bin/true"}}

	buf := captureLogTo(t)
	_, _, run := m.sandboxSidecar(mf)
	if run {
		t.Skip("this host has a confinement mechanism; nothing is refused here")
	}
	out := buf.String()
	if len(pub.events) == 0 || pub.events[0].Type != "plugin.sandbox.refused" {
		t.Fatalf("no plugin.sandbox.refused event: %+v", pub.events)
	}
	if !strings.Contains(out, "REFUSED") || !strings.Contains(out, `"p"`) {
		t.Fatalf("enforce+no-mechanism refused the sidecar and logged NOTHING: the plugin's server plane is permanently dead and the UI shows \"starting\" forever.\nlog was: %q", out)
	}
}

// Control: the best-effort branch on the same host stays loud too, so the two
// outcomes are distinguishable in the log.
func TestBestEffortWithoutAMechanismStaysLoud(t *testing.T) {
	t.Setenv("PATH", "")
	m := &Manager{pub: &refusePub{}, sandboxMode: sandbox.ModeBestEffort}
	mf := Manifest{ID: "p", Dir: t.TempDir(), Server: &ServerSpec{Command: "/bin/true"}}

	buf := captureLogTo(t)
	_, _, run := m.sandboxSidecar(mf)
	if !run {
		t.Skip("best-effort refused on this host")
	}
	if !strings.Contains(buf.String(), "WITHOUT sandboxing") {
		t.Fatalf("best-effort lost its warning: %q", buf.String())
	}
}
