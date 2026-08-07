package main

import (
	"sort"
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

// TestBrainMethodsAllClassified is the METHOD-level completeness check the
// package had no counterpart to.
//
// TestBrainMethodsAllScoped above asks capspec.MissingSpec, which is a name
// PREFIX heuristic over {fs., search., library., git.}: it returns false for
// every claude.*, sessions.*, config.*, layouts.*, app.*, analytics.*,
// providers.* and replay.* method no matter what that method does. Measured on
// the union of both providers' registries, 27 of 73 capabilities were classified
// NOWHERE — and six of them are the ones the app's own consent list marks
// sensitive:true, including claude.approve and claude.gate, an approval-override
// pair that composes with agents.sendMessage into arbitrary host command
// execution. A brand-new `claude.autoApprove`, registered and dispatched and
// byte-for-byte claude.approve under another name, passed the whole Go module.
//
// The repo already had this shape for the HUMAN-facing list —
// pluginPermissions.test.ts's "labels every capability the main process actually
// registers". The machine-enforced list had none.
func TestBrainMethodsAllClassified(t *testing.T) {
	r := newRegistry(newClaudemonClient("http://unused"))
	seen := map[string]bool{}
	for _, set := range [][]string{r.methods(), r.catalogMethods()} {
		for _, m := range set {
			seen[m] = true
		}
	}
	if len(seen) < 40 {
		t.Fatalf("the brain registry enumerated only %d methods — the method lists changed and this guard is guarding nothing", len(seen))
	}
	for _, m := range sortedMethodNames(seen) {
		if capspec.MissingClassification(m) {
			t.Errorf("the brain registers %q and capspec says nothing about it — not a PathParam entry, not an unscopedByDecision reason, not an inertMethods reason. That silence is what let claude.approve and claude.gate (an approval-override pair) ship unexamined: MissingSpec only ever asks about fs./search./library./git. names. Decide what it is and write it down.", m)
		}
	}
}

func sortedMethodNames(m map[string]bool) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
