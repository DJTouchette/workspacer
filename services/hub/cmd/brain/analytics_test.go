package main

import (
	"context"
	"encoding/json"
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

// The headless stubs return well-formed empty results with an "unavailable"
// marker, so a web client degrades to an empty dashboard instead of erroring on
// a missing provider.
func TestAnalyticsStubsReturnEmptyMarkedResults(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))

	res, err := reg.handle(context.Background(), "analytics.summary", nil)
	if err != nil {
		t.Fatal(err)
	}
	var summary struct {
		Totals struct {
			Sessions int `json:"sessions"`
		} `json:"totals"`
		ByDay       []any  `json:"byDay"`
		ByProvider  []any  `json:"byProvider"`
		Unavailable string `json:"unavailable"`
	}
	if err := json.Unmarshal(res, &summary); err != nil {
		t.Fatalf("summary not valid JSON: %v", err)
	}
	if summary.Unavailable != "headless" {
		t.Errorf("summary should carry unavailable=headless, got %q", summary.Unavailable)
	}
	if summary.Totals.Sessions != 0 || summary.ByDay == nil || summary.ByProvider == nil {
		t.Errorf("summary should be empty-but-well-formed, got %+v", summary)
	}

	res, err = reg.handle(context.Background(), "analytics.recent", []byte(`{"limit":50}`))
	if err != nil {
		t.Fatal(err)
	}
	var recent []any
	if err := json.Unmarshal(res, &recent); err != nil {
		t.Fatalf("recent not valid JSON array: %v", err)
	}
	if len(recent) != 0 {
		t.Errorf("recent should be empty headless, got %d rows", len(recent))
	}
}
