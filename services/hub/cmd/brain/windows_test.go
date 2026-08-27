package main

import (
	"encoding/json"
	"testing"
)

// Cross-language context-window drift guard.
//
// contracts/model-context-windows.json is the SHARED fixture: a Rust test
// (services/claudemon/src/session/windows.rs) and a TypeScript test
// (apps/desktop/src/main/services/modelContextWindowsContract.test.ts) consume
// the exact same file. If the three tables — or, more importantly, the three
// RESOLVERS — ever disagree, one side's contract test fails.
//
// The fixture exists because there were five hand-maintained window tables in
// this repo and three of them disagreed for the same model id: a gpt-5-codex
// session reported 272000 through the provider status line and 200000 through
// usage, at the same instant, for the same session.

type windowContract struct {
	Windows []struct {
		Match  string `json:"match"`
		Kind   string `json:"kind"`
		Window uint64 `json:"window"`
		Note   string `json:"note"`
	} `json:"windows"`
	LookupCases []struct {
		Model    string  `json:"model"`
		Expected *uint64 `json:"expected"`
		Note     string  `json:"note"`
	} `json:"lookupCases"`
	MarkerCases []struct {
		Requested string  `json:"requested"`
		Expected  *uint64 `json:"expected"`
		Note      string  `json:"note"`
	} `json:"markerCases"`
	ResolutionCases []struct {
		Name           string  `json:"name"`
		Model          *string `json:"model"`
		RequestedModel *string `json:"requestedModel"`
		ReportedWindow *uint64 `json:"reportedWindow"`
		Override       *uint64 `json:"override"`
		PeakContext    uint64  `json:"peakContext"`
		Expected       *uint64 `json:"expected"`
		Note           string  `json:"note"`
	} `json:"resolutionCases"`
}

func loadWindowContract(t *testing.T) windowContract {
	t.Helper()
	raw := mustReadRepoFile(t, "contracts", "model-context-windows.json")
	var c windowContract
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatalf("parse contracts/model-context-windows.json: %v", err)
	}
	if len(c.Windows) == 0 || len(c.LookupCases) == 0 || len(c.MarkerCases) == 0 || len(c.ResolutionCases) == 0 {
		t.Fatal("the context-window contract decoded to an empty block; the corpus was gutted or a key was renamed")
	}
	return c
}

// The table itself, row for row and IN ORDER. A lookup corpus alone would not
// catch a reordering that happens to leave every sampled id where it was; the
// order is what makes `fable` beat `claude`.
func TestContextWindowTableMatchesTheContract(t *testing.T) {
	c := loadWindowContract(t)
	if len(contextWindows) != len(c.Windows) {
		t.Fatalf("the contract has %d rows and contextWindows has %d — a row was added on one side only", len(c.Windows), len(contextWindows))
	}
	for i, want := range c.Windows {
		got := contextWindows[i]
		if got.Match != want.Match || string(got.Kind) != want.Kind || got.Window != want.Window {
			t.Errorf("row %d: contract says {%q %s %d}, contextWindows says {%q %s %d}",
				i, want.Match, want.Kind, want.Window, got.Match, got.Kind, got.Window)
		}
	}
}

func TestContextWindowLookupCases(t *testing.T) {
	c := loadWindowContract(t)
	for _, tc := range c.LookupCases {
		w, ok := windowForModel(tc.Model)
		if tc.Expected == nil {
			if ok {
				t.Errorf("windowForModel(%q) = %d, want UNKNOWN — %s", tc.Model, w, tc.Note)
			}
			continue
		}
		if !ok || w != *tc.Expected {
			t.Errorf("windowForModel(%q) = %d (known=%v), want %d — %s", tc.Model, w, ok, *tc.Expected, tc.Note)
		}
	}
}

func TestContextWindowMarkerCases(t *testing.T) {
	c := loadWindowContract(t)
	for _, tc := range c.MarkerCases {
		w, ok := requestedWindowFor(tc.Requested)
		if tc.Expected == nil {
			if ok {
				t.Errorf("requestedWindowFor(%q) = %d, want SAYS-NOTHING — %s", tc.Requested, w, tc.Note)
			}
			continue
		}
		if !ok || w != *tc.Expected {
			t.Errorf("requestedWindowFor(%q) = %d (known=%v), want %d — %s", tc.Requested, w, ok, *tc.Expected, tc.Note)
		}
	}
}

// The block that actually pins the twins: it exercises the RESOLVER, so a stack
// that ports the table correctly but the hierarchy wrong still goes red.
func TestContextWindowResolutionCases(t *testing.T) {
	c := loadWindowContract(t)
	for _, tc := range c.ResolutionCases {
		sig := windowSignals{PeakContext: tc.PeakContext}
		if tc.RequestedModel != nil {
			sig.RequestedModel = *tc.RequestedModel
		}
		if tc.ReportedWindow != nil {
			sig.Reported, sig.HasReported = *tc.ReportedWindow, true
		}
		if tc.Override != nil {
			sig.Override, sig.HasOverride = *tc.Override, true
		}
		model := ""
		if tc.Model != nil {
			model = *tc.Model
		}
		w, ok := resolveContextWindow(model, sig)
		if tc.Expected == nil {
			if ok {
				t.Errorf("%s: resolved %d, want UNKNOWN — %s", tc.Name, w, tc.Note)
			}
			continue
		}
		if !ok || w != *tc.Expected {
			t.Errorf("%s: resolved %d (known=%v), want %d — %s", tc.Name, w, ok, *tc.Expected, tc.Note)
		}
	}
}

// The alarm must not be reachable only through the fixture: assert the boundary
// directly, because "off by one on the tolerance" is the way a full 200k
// session loses its meter at exactly the moment it matters.
func TestContextWindowDriftAlarmBoundary(t *testing.T) {
	at := func(peak uint64) (uint64, bool) {
		return resolveContextWindow("claude-opus-5", windowSignals{PeakContext: peak})
	}
	for _, tc := range []struct {
		peak uint64
		want uint64
		ok   bool
		why  string
	}{
		{200_000, 200_000, true, "exactly full is not over-full"},
		{204_000, 200_000, true, "1.02x exactly is the boundary"},
		{204_001, 0, false, "past it, the window is not believable"},
	} {
		got, ok := at(tc.peak)
		if ok != tc.ok || (ok && got != tc.want) {
			t.Errorf("peak %d → (%d, %v), want (%d, %v) — %s", tc.peak, got, ok, tc.want, tc.ok, tc.why)
		}
	}
}

// formatClaudeAliasWindow feeds the model picker's `context` badge. Pinned here
// so the badge cannot drift back into being its own opinion about a window.
func TestFormatClaudeAliasWindowBadges(t *testing.T) {
	for _, tc := range []struct{ alias, want string }{
		{"opus", "200K"},
		{"sonnet", "200K"},
		{"haiku", "200K"},
		{"opus[1m]", "1M"},
		{"sonnet[1m]", "1M"},
		{"fable", "1M"},
		// A concrete id the user has been seen running — the picker lists those
		// too, and they carry the family already.
		{"claude-opus-4-8-20260101", "200K"},
		{"claude-fable-5", "1M"},
	} {
		if got := formatClaudeAliasWindow(tc.alias); got != tc.want {
			t.Errorf("formatClaudeAliasWindow(%q) = %q, want %q", tc.alias, got, tc.want)
		}
	}
}
