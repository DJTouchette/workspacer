// Package layout owns the shared workspace layout document — the piece that
// makes the web remote mirror the desktop like tmux. claudemon owns the live
// sessions/PTYs; this owns the *window manager* state: which agent cards exist,
// their tabs/panes, the active tab, the view mode.
//
// The hub stores the document but does not interpret it. Its `Data` is the
// renderer's `AgentWorkspace[]` + globals, opaque here — the reducer that
// produces it lives in the renderer (one source of truth, no second Go copy to
// drift out of sync). The hub's job is narrow and authoritative: hold the
// latest document, version it, persist it, and broadcast every change so all
// connected clients converge.
//
// Concurrency is last-writer-wins: every accepted write bumps Version and is
// broadcast as `layout.changed`. With a single human driving at a time this is
// exactly right; simultaneous edits resolve to whichever write landed last and
// all clients reconcile to the broadcast.
package layout

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// ChangedTopic is published on every accepted write.
const ChangedTopic = "layout.changed"

// Document is the shared workspace layout. Data is opaque to the hub (the
// renderer's AgentWorkspace[] + globals). Version increments on every accepted
// write so clients can ignore stale echoes and detect when they're behind.
type Document struct {
	Version int             `json:"version"`
	Data    json.RawMessage `json:"data"`
}

// Service holds the document, persists it, and answers the layout.* RPCs.
type Service struct {
	mu   sync.RWMutex
	doc  Document
	b    *broker.Broker
	path string // persistence file; "" = memory only
}

// New builds a Service, seeding from the persisted file when present. The
// broker is used to broadcast changes; path is where the document is persisted
// across hub restarts (empty disables persistence).
func New(b *broker.Broker, path string) *Service {
	s := &Service{b: b, path: path, doc: Document{Version: 0, Data: json.RawMessage("null")}}
	s.load()
	return s
}

func (s *Service) load() {
	if s.path == "" {
		return
	}
	raw, err := os.ReadFile(s.path)
	if err != nil {
		return // no prior state; start empty
	}
	var d Document
	if err := json.Unmarshal(raw, &d); err != nil {
		return
	}
	if len(d.Data) == 0 {
		d.Data = json.RawMessage("null")
	}
	s.doc = d
}

// persist atomically writes the document to disk (best-effort; persistence
// failures must not break the live sync).
func (s *Service) persist(d Document) {
	if s.path == "" {
		return
	}
	if err := os.MkdirAll(filepath.Dir(s.path), 0o755); err != nil {
		return
	}
	raw, err := json.Marshal(d)
	if err != nil {
		return
	}
	tmp := s.path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o644); err != nil {
		return
	}
	_ = os.Rename(tmp, s.path)
}

// Get returns the current document. Params are ignored.
func (s *Service) Get(_ json.RawMessage) (any, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.doc, nil
}

// Set replaces the document with the caller's `data`, bumps the version,
// persists, and broadcasts layout.changed. Params: { "data": <layout> }.
func (s *Service) Set(params json.RawMessage) (any, error) {
	var in struct {
		Data json.RawMessage `json:"data"`
	}
	if err := json.Unmarshal(params, &in); err != nil {
		return nil, err
	}
	if len(in.Data) == 0 {
		return nil, errors.New("layout.set requires { data }")
	}

	s.mu.Lock()
	s.doc.Version++
	s.doc.Data = append(json.RawMessage(nil), in.Data...)
	d := s.doc
	// Persist while still holding the lock so writes are serialized: a higher
	// version can never be overwritten on disk by a slower, older-version
	// persist, and two goroutines can't clobber each other's shared .tmp file.
	s.persist(d)
	s.mu.Unlock()

	s.b.Publish(event.New(ChangedTopic, "hub", d))
	return d, nil
}
