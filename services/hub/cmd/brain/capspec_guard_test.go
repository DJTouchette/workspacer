package main

import (
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// TestBrainMethodsAllScoped cross-checks the headless brain's registered
// capability surface (both scopes) against capspec: any fs.*/search.* method it
// exposes must have a PathParam entry, or the bus would grant it to plugins with
// no filesystem confinement. This is the Go half of the capspec allowlist guard
// (the desktop half lives in internal/capspec's hubCapabilities.ts cross-check),
// and it fails at build time if someone adds a path capability here without
// scoping it.
func TestBrainMethodsAllScoped(t *testing.T) {
	r := newRegistry(newClaudemonClient("http://unused"))
	seenPathCap := false
	for _, set := range [][]string{r.methods(), r.catalogMethods()} {
		for _, m := range set {
			if capspec.LooksPathBearing(m) {
				seenPathCap = true
			}
			if capspec.MissingSpec(m) {
				t.Errorf("brain registers %q, which is filesystem-scoped by name but has no capspec.PathParam entry — it would be grantable to plugins with no path confinement", m)
			}
		}
	}
	if !seenPathCap {
		t.Error("expected the brain to register at least one fs.*/search.* capability; found none — the method lists likely changed")
	}
}

// TestSpawnStaysDeliberatelyUnscoped pins the capspec stance on agents.spawn
// now that it grew the provider/transport/effort/permissionMode params: it
// still carries only `cwd` as a path-ish field, and spawning is deliberately
// NOT path-scoped (starting an agent is a separate authz decision — see the
// vocabulary tests in internal/capspec). If capspec ever gains a PathParam
// entry for agents.spawn, the brain's spawn dispatch (PTY *and* spawn-managed)
// must learn root confinement first — this test forces that conversation.
func TestSpawnStaysDeliberatelyUnscoped(t *testing.T) {
	if _, ok := capspec.IsPathScoped("agents.spawn"); ok {
		t.Fatal("agents.spawn became path-scoped in capspec, but the brain's spawn handlers do no root confinement — teach spawn/spawnManagedSession to confine cwd before scoping it")
	}
	if capspec.MissingSpec("agents.spawn") {
		t.Fatal("agents.spawn now looks path-bearing by name to capspec — align this guard and the spawn handlers")
	}
}
