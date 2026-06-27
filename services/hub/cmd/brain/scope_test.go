package main

import "testing"

func TestCatalogScopeIsDispatchableSubset(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	full := map[string]bool{}
	for _, m := range reg.methods() {
		full[m] = true
	}
	catalog := reg.catalogMethods()
	if len(catalog) == 0 || len(catalog) >= len(full) {
		t.Fatalf("catalog should be a non-empty proper subset: %d of %d", len(catalog), len(full))
	}
	for _, m := range catalog {
		if !full[m] {
			t.Errorf("catalog method %q is not in the full method set", m)
		}
	}
	// catalog must exclude the live/enriched caps the app keeps owning.
	for _, m := range []string{"agents.spawn", "sessions.transcript", "claude.approve", "notifications.post", "search.project"} {
		for _, c := range catalog {
			if c == m {
				t.Errorf("catalog must not include app-owned capability %q", m)
			}
		}
	}
}

func TestMethodsForScope(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	if len(reg.methodsForScope("full")) != len(reg.methods()) {
		t.Error("full scope should register the whole surface")
	}
	if len(reg.methodsForScope("catalog")) != len(reg.catalogMethods()) {
		t.Error("catalog scope should register the catalog subset")
	}
	// Unknown scope falls back to full.
	if len(reg.methodsForScope("bogus")) != len(reg.methods()) {
		t.Error("unknown scope should fall back to full")
	}
}
