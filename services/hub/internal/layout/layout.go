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
	"log"
	"os"
	"path/filepath"
	"regexp"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// ChangedTopic is published on every accepted write.
const ChangedTopic = "layout.changed"

// busTokenQuery matches a `busToken=<value>` query param anywhere in the opaque
// document. Plugin pane URLs carry the per-plugin bus token that way, and the
// layout document is a *shared*, world-readable (0644) file that every connected
// client also receives on layout.changed — while the token file it came from is
// 0600. A capability token has no business riding along in it.
//
// The match stops at the JSON string terminator, a query separator, or a
// backslash escape, so only the value is consumed. Redaction is done on the raw
// bytes rather than by decoding and re-encoding: `Data` is the renderer's state
// and the hub deliberately doesn't interpret it, so everything but the token
// must round-trip byte for byte.
var busTokenQuery = regexp.MustCompile(`busToken=[^"&\\]*`)

// redactBusTokens blanks any bus token in the document, leaving the param in
// place so a client reading the URL still sees an (empty) token rather than a
// differently-shaped URL. The renderer strips the token before writing too;
// this is the belt-and-braces half, so a stale document on disk or a
// third-party writer can't reintroduce the leak.
func redactBusTokens(data json.RawMessage) json.RawMessage {
	if !busTokenQuery.Match(data) {
		return data
	}
	return busTokenQuery.ReplaceAll(data, []byte("busToken="))
}

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
	// A document written before Set started redacting still holds live tokens;
	// clean it on the way in so the first Get doesn't serve them.
	d.Data = redactBusTokens(d.Data)
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

// spawnEscalationKeys are the per-agent fields of the shared layout document
// that STOP BEING DESCRIPTION on the desktop's next launch and become arguments
// to a spawn.
//
// The document is opaque to the hub by design — the reducer that produces it
// lives in the renderer, and the hub's job is to hold, version, persist and
// broadcast it. That is still true of everything not named here. But the
// document is not only read: App.tsx hardcodes adoptSharedLayout, so on
// hydration the desktop adopts it, useSessionLifecycle runs
// reconcileAgents{respawnStopped:true} over it, and every agent whose sessionId
// is not live — guaranteed after a restart — goes to respawnFromRecord, which
// hands these fields straight to window.electronAPI.spawnClaude. That is the
// LOCAL IPC spawn door, which does no scrubbing.
//
// The bus's own agents.spawn refuses exactly these four: skipPermissions is
// forced off, mcpItemIds is dropped, the profile is run through
// scrubProfileBypass and an escalating permissionMode is discarded — "remote
// spawns never auto-bypass approvals". A caller that may not spawn at all could
// nevertheless write them here and have the desktop spawn them verbatim one
// launch later. Neither call is wrong alone: layout.set writes an opaque blob,
// and the local spawn door is trusted by construction.
//
// This is the same belt-and-braces posture redactBusTokens already takes on this
// document, and for the same reason: "a stale document on disk or a third-party
// writer can't reintroduce the leak".
var spawnEscalationKeys = []string{"skipPermissions", "permissionMode", "profileId", "mcpItemIds"}

// scrubAdoptedSpawnFields removes spawnEscalationKeys from every entry of the
// document's `agents` array, returning the (re-encoded) data and the keys it
// dropped.
//
// Structural, not textual: these are JSON object keys at a known depth, and a
// regex over the raw bytes would also hit a pane title that happened to contain
// the word. Anything that does not decode as the expected shape is returned
// UNCHANGED — the hub does not interpret this document, and a write it cannot
// parse is a write it must not silently rewrite either. That is safe here
// because the desktop reads the same bytes with the same expectations: a
// document whose `agents` is not an array of objects is one respawnFromRecord
// gets nothing out of.
func scrubAdoptedSpawnFields(data json.RawMessage) (json.RawMessage, []string) {
	var doc map[string]json.RawMessage
	if err := json.Unmarshal(data, &doc); err != nil {
		return data, nil
	}
	rawAgents, ok := doc["agents"]
	if !ok {
		return data, nil
	}
	var agents []map[string]json.RawMessage
	if err := json.Unmarshal(rawAgents, &agents); err != nil {
		return data, nil
	}
	var dropped []string
	for _, a := range agents {
		for _, k := range spawnEscalationKeys {
			if _, present := a[k]; present {
				delete(a, k)
				dropped = append(dropped, k)
			}
		}
	}
	if len(dropped) == 0 {
		return data, nil
	}
	reAgents, err := json.Marshal(agents)
	if err != nil {
		return data, nil
	}
	doc["agents"] = reAgents
	out, err := json.Marshal(doc)
	if err != nil {
		return data, nil
	}
	return out, dropped
}

// Set replaces the document with the caller's `data`, bumps the version,
// persists, and broadcasts layout.changed. Params: { "data": <layout> }.
//
// Trusted-caller entry point, kept so tests and in-process callers that already
// hold host authority read the same as before. The BUS registers SetAs.
func (s *Service) Set(params json.RawMessage) (any, error) {
	return s.setScrubbed(params, false)
}

// SetAs is Set with the calling connection's identity, which decides whether the
// spawn-escalation fields survive. A trusted (host / operator) writer is the
// desktop mirroring its own state and keeps them; anything else — a scoped user
// token, a plugin token — has them dropped, because for those callers this
// document is an unguarded second door onto agents.spawn's clamps.
func (s *Service) SetAs(caller busIdentity, params json.RawMessage) (any, error) {
	return s.setScrubbed(params, !caller.IsTrusted())
}

// busIdentity is the slice of bus.CallerIdentity this package needs, declared as
// an interface so internal/layout does not import internal/bus (which imports
// this package's siblings). One method, one question.
type busIdentity interface{ IsTrusted() bool }

func (s *Service) setScrubbed(params json.RawMessage, scrub bool) (any, error) {
	var in struct {
		Data json.RawMessage `json:"data"`
	}
	if err := json.Unmarshal(params, &in); err != nil {
		return nil, err
	}
	if len(in.Data) == 0 {
		return nil, errors.New("layout.set requires { data }")
	}

	data := append(json.RawMessage(nil), in.Data...)
	if scrub {
		var dropped []string
		data, dropped = scrubAdoptedSpawnFields(data)
		if len(dropped) > 0 {
			// Logged rather than refused, matching how the spawn path itself
			// handles the same values: the write is honoured, the escalation is
			// not. Refusing would let a hostile writer deny the whole shared
			// layout to the operator by writing one bad key.
			log.Printf("SECURITY: layout.set: dropping spawn-escalation field(s) %v from a non-trusted bus client — the desktop respawns this document's agents verbatim on its next launch", dropped)
		}
	}

	s.mu.Lock()
	s.doc.Version++
	s.doc.Data = redactBusTokens(data)
	d := s.doc
	// Persist while still holding the lock so writes are serialized: a higher
	// version can never be overwritten on disk by a slower, older-version
	// persist, and two goroutines can't clobber each other's shared .tmp file.
	s.persist(d)
	s.mu.Unlock()

	s.b.Publish(event.New(ChangedTopic, "hub", d))
	return d, nil
}
