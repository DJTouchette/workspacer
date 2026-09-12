package main

import (
	"context"
	"testing"
)

// Analytics is registered only in the full (headless) scope — when the brain runs
// alongside the desktop app, the app owns analytics.* via registerCapability and
// the brain must not collide with it on the single-owner router.
func TestAnalyticsRegisteredFullScopeOnly(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	full := map[string]bool{}
	for _, m := range reg.methods() {
		full[m] = true
	}
	for _, m := range []string{"analytics.summary", "analytics.recent"} {
		if !full[m] {
			t.Errorf("full scope should register %q for headless clients", m)
		}
	}
	for _, m := range reg.catalogMethods() {
		if m == "analytics.summary" || m == "analytics.recent" {
			t.Errorf("catalog scope must not register %q — the desktop owns analytics", m)
		}
	}
}

// Source failure must not masquerade as a measured empty history.
func TestAnalyticsUnavailableSourceReturnsError(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	for _, method := range []string{"analytics.summary", "analytics.recent"} {
		if _, err := reg.handle(context.Background(), method, nil); err == nil {
			t.Fatalf("%s concealed unavailable history", method)
		}
	}
}
