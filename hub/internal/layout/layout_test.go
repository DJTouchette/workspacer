package layout

import (
	"encoding/json"
	"path/filepath"
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
