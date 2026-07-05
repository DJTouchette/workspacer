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
