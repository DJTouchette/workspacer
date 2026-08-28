package main

import (
	"context"
	"testing"
)

// A codex spawn that names no transport must land HEADLESS — the same shape the
// desktop picks (apps/desktop/src/main/lib/spawnTransport.ts). This is the leg
// most of the fleet actually takes: `agents.spawn` over the bus, from the MCP
// facade or a dispatched worker, names no transport at all. Before the shared
// resolver, an absent key meant "hybrid" here and "hybrid" in claudemon, so a
// bus-dispatched codex worker came up as a TUI + viewer pair while a locally
// spawned one came up GUI-only.
func TestCodexSpawnDefaultsToStreamOnTheWire(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one spawn-managed call, got %d", len(managed))
	}
	if got := managed[0].body["transport"]; got != "stream" {
		t.Errorf("codex spawn with no transport must state 'stream', got %v", got)
	}
}

// ...and an explicit 'pty' still reaches the wire, or the hybrid would be
// unreachable the moment the default flipped. The whole point of forwarding
// BOTH values is that "the user chose hybrid" and "the user said nothing" stop
// being the same request.
func TestCodexExplicitPtyStatesItselfOnTheWire(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one spawn-managed call, got %d", len(managed))
	}
	if got := managed[0].body["transport"]; got != "pty" {
		t.Errorf("an explicit hybrid request must ride the wire as 'pty', got %v", got)
	}
}

// The resolver itself: caller > config > shipped fallback, per harness. The
// per-harness fallbacks differ (claude's headless transport is opt-in per
// install, codex's is the default), which is why one shared "?? 'pty'" was
// wrong for codex.
func TestTransportDefaultResolutionOrder(t *testing.T) {
	for _, tc := range []struct {
		name, provider, requested, want string
		cfg                             map[string]any
	}{
		{name: "explicit wins over config", provider: "codex", requested: "pty", want: "pty",
			cfg: map[string]any{"codex": map[string]any{"transport": "stream"}}},
		{name: "config wins over fallback", provider: "codex", want: "pty",
			cfg: map[string]any{"codex": map[string]any{"transport": "pty"}}},
		// A registry with no config file still SEEDS config_defaults.json, so
		// these two read the shipped values rather than the in-code fallback —
		// which is the point: the JSON and this map must agree.
		{name: "codex ships headless", provider: "codex", want: "stream", cfg: map[string]any{}},
		{name: "claude ships headless too", provider: "claude", want: "stream", cfg: map[string]any{}},
		// An unparseable value is the only way to reach the in-code fallback
		// from here, and it must not become a third session shape.
		{name: "junk codex value falls back to stream", provider: "codex", want: "stream",
			cfg: map[string]any{"codex": map[string]any{"transport": "sideways"}}},
		{name: "junk claude value falls back to pty", provider: "claude", want: "pty",
			cfg: map[string]any{"claude": map[string]any{"transport": "sideways"}}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			// A fresh registry (and config home) per case: save() MERGES, so a
			// value set for one case would survive into the next and quietly
			// turn a fallback assertion into a config one.
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newSpawnTestRegistry(t, srv.URL)
			if len(tc.cfg) > 0 {
				reg.cfg.save(tc.cfg)
			}
			if got := reg.transportDefault(tc.provider, tc.requested); got != tc.want {
				t.Errorf("transportDefault(%q, %q) = %q, want %q", tc.provider, tc.requested, got, tc.want)
			}
		})
	}
}
