package layout

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/broker"
)

func TestSetGetRoundTrip(t *testing.T) {
	b := broker.New()
	s := New(b, "")

	got, err := s.Get(nil)
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if d := got.(Document); d.Version != 0 || string(d.Data) != "null" {
		t.Fatalf("initial doc = %+v, want version 0 / null", d)
	}

	res, err := s.Set(json.RawMessage(`{"data":{"agents":[{"id":"a1"}],"viewMode":"tabs"}}`))
	if err != nil {
		t.Fatalf("Set: %v", err)
	}
	if d := res.(Document); d.Version != 1 {
		t.Fatalf("after set, version = %d, want 1", d.Version)
	}

	got, _ = s.Get(nil)
	d := got.(Document)
	if d.Version != 1 {
		t.Fatalf("Get version = %d, want 1", d.Version)
	}
	var data struct {
		ViewMode string `json:"viewMode"`
	}
	if err := json.Unmarshal(d.Data, &data); err != nil || data.ViewMode != "tabs" {
		t.Fatalf("data round-trip failed: %s (%v)", d.Data, err)
	}
}

func TestSetBroadcasts(t *testing.T) {
	b := broker.New()
	s := New(b, "")
	sub := b.Subscribe([]string{ChangedTopic})
	defer b.Unsubscribe(sub)

	if _, err := s.Set(json.RawMessage(`{"data":{"x":1}}`)); err != nil {
		t.Fatalf("Set: %v", err)
	}

	select {
	case ev := <-sub.C:
		if ev.Type != ChangedTopic {
			t.Fatalf("event type = %q, want %q", ev.Type, ChangedTopic)
		}
		var d Document
		if err := json.Unmarshal(ev.Data, &d); err != nil {
			t.Fatalf("decode event: %v", err)
		}
		if d.Version != 1 {
			t.Fatalf("broadcast version = %d, want 1", d.Version)
		}
	case <-time.After(time.Second):
		t.Fatal("no layout.changed broadcast")
	}
}

func TestSetRejectsMissingData(t *testing.T) {
	s := New(broker.New(), "")
	if _, err := s.Set(json.RawMessage(`{}`)); err == nil {
		t.Fatal("expected error for missing data")
	}
}

// The layout document is shared with every connected client and persisted
// world-readable at 0644, while the per-plugin bus tokens that ride in plugin
// pane URLs come from a 0600 file. Set must blank them whatever it is handed —
// the renderer strips its own writes, but a stale document or a third-party
// writer must not be able to put a live capability token back into the shared
// state.
func TestSetRedactsBusTokensFromPaneURLs(t *testing.T) {
	b := broker.New()
	sub := b.Subscribe([]string{ChangedTopic})
	defer b.Unsubscribe(sub)
	s := New(b, "")

	res, err := s.Set(json.RawMessage(`{"data":{"agents":[{"panes":[
		{"id":"p1","type":"plugin","url":"http://127.0.0.1:7895/plugins/ui/editor/?busToken=sec-ret-123&pluginId=editor"},
		{"id":"p2","type":"plugin","url":"http://127.0.0.1:7895/plugins/ui/x/?pluginId=x&busToken=another-secret"}
	]}]}}`))
	if err != nil {
		t.Fatalf("Set: %v", err)
	}

	stored := string(res.(Document).Data)
	if strings.Contains(stored, "sec-ret-123") || strings.Contains(stored, "another-secret") {
		t.Fatalf("Set stored a live bus token: %s", stored)
	}
	// The param itself stays, so a client reading the URL sees an empty token
	// rather than a URL of a different shape.
	if n := strings.Count(stored, "busToken="); n != 2 {
		t.Fatalf("busToken= appears %d times, want both params kept (blanked): %s", n, stored)
	}
	// Everything else must round-trip untouched — the hub does not interpret
	// this document.
	if !strings.Contains(stored, "pluginId=editor") || !strings.Contains(stored, `"id":"p2"`) {
		t.Fatalf("redaction disturbed the rest of the document: %s", stored)
	}

	// The broadcast every client receives carries the redacted copy too.
	select {
	case ev := <-sub.C:
		if strings.Contains(string(ev.Data), "sec-ret-123") {
			t.Fatalf("layout.changed broadcast leaked the token: %s", ev.Data)
		}
	case <-time.After(time.Second):
		t.Fatal("no layout.changed broadcast")
	}

	// And so does what any later reader gets back.
	got, _ := s.Get(nil)
	if strings.Contains(string(got.(Document).Data), "sec-ret-123") {
		t.Fatalf("Get served the token back: %s", got.(Document).Data)
	}
}

// A document written before redaction existed still holds live tokens on disk.
// Loading it must clean them, or the first Get after the upgrade hands them out.
func TestLoadRedactsAPreexistingToken(t *testing.T) {
	path := filepath.Join(t.TempDir(), "layout.json")
	poisoned := `{"version":7,"data":{"panes":[{"url":"http://h/plugins/ui/e/?busToken=leaked-token"}]}}`
	if err := os.WriteFile(path, []byte(poisoned), 0o644); err != nil {
		t.Fatal(err)
	}
	s := New(broker.New(), path)
	got, _ := s.Get(nil)
	d := got.(Document)
	if d.Version != 7 {
		t.Fatalf("version = %d, want the persisted 7", d.Version)
	}
	if strings.Contains(string(d.Data), "leaked-token") {
		t.Fatalf("load served a token persisted before redaction: %s", d.Data)
	}
}

func TestPersistAndReload(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "layout.json")
	b := broker.New()
	s := New(b, path)
	if _, err := s.Set(json.RawMessage(`{"data":{"persisted":true}}`)); err != nil {
		t.Fatalf("Set: %v", err)
	}

	// A fresh service seeded from the same file recovers the document.
	s2 := New(broker.New(), path)
	got, _ := s2.Get(nil)
	d := got.(Document)
	if d.Version != 1 {
		t.Fatalf("reloaded version = %d, want 1", d.Version)
	}
	var data struct {
		Persisted bool `json:"persisted"`
	}
	if err := json.Unmarshal(d.Data, &data); err != nil || !data.Persisted {
		t.Fatalf("reloaded data wrong: %s (%v)", d.Data, err)
	}
}
