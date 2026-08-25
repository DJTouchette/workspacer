package main

// `terminals.open` — the VISIBLE-terminal tool, headless.
//
// TWIN: the terminals.open registration in hubCapabilities.ts.
//
// THE DISTINCTION THIS CAPABILITY EXISTS FOR. terminals.create (already
// provided here) makes a HEADLESS, driveable PTY: an agent can run things in it
// and read the bytes back, and the user never sees it. terminals.open asks the
// CLIENT to open a real terminal PANE, so the process is watchable — "bring up
// the dev server so the user can see it". On a remote node driven from a
// browser that is the whole point of the feature, and both halves were missing:
// the method had no headless provider, and the web client's
// onFacadeOpenTerminal was a no-op.
//
// PROVIDING ONLY THE METHOD WOULD HAVE BEEN WORSE THAN THE GAP. The desktop
// implementation is emitToRenderer(FACADE_OPEN_TERMINAL) — it does not start a
// process, it asks a UI to. Registering a handler here that returned {ok:true}
// and told nobody would answer an agent's "open a terminal the user can see"
// with a success and no terminal: a silent wrong answer, which is precisely the
// class of failure this whole port is about. So the request goes onto the bus as
// `facade.openTerminal`, and webBackend.ts subscribes to it and opens the pane —
// the same event shape the preload delivers over IPC on the desktop.
//
// capspec classifies the topic TopicGuardedBy terminals.open (eventtopics.go):
// the payload is a host command line, so receiving it costs the same capability
// as sending it. A view or triage token holds neither.
//
// CONFINEMENT. `cwd` gets the same treatment terminals.create's does — the
// shared normalizeSpawnCwd, and nothing more. That is a decision on the record
// in capspec, not an oversight: a process working directory is not confined on
// either provider, and holding the capability is the gate. The other string,
// `command`, is NOT argv[0]: the pane opens the HOST'S default login shell (the
// caller names no shell) and the command runs inside it under that shell's own
// tool/PTY rules, exactly as a line the user typed would. There is therefore no
// argv[0] to hold against shellallow.go's login-shell allowlist here, which is
// the one thing that would look missing next to terminals.create.

import (
	"context"
	"encoding/json"
	"errors"
)

// terminalsOpen asks the client to open a visible terminal pane.
func (r *registry) terminalsOpen(_ context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd             string `json:"cwd"`
		Command         string `json:"command"`
		Label           string `json:"label"`
		ParentSessionID string `json:"parentSessionId"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if r.publish == nil {
		// A brain with no bus connection (catalog scope, or a unit test that did
		// not wire one) cannot ask anything to open a pane. Refuse rather than
		// report success: the caller's whole request is that something becomes
		// VISIBLE, and there is nothing here that could make it so.
		return nil, errors.New("terminals.open: this provider has no bus connection to ask a client for a terminal pane")
	}
	// One normalization, shared with terminals.create and with agents.spawn
	// (spawncwd.go explains why `if exists { cwd } else { home }` had to go).
	payload, err := json.Marshal(map[string]any{
		"cwd":             normalizeCwd(p.Cwd),
		"command":         p.Command,
		"label":           p.Label,
		"parentSessionId": p.ParentSessionID,
	})
	if err != nil {
		return nil, err
	}
	r.publish("facade.openTerminal", payload)
	return okResult()
}
