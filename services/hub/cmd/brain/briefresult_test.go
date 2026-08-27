package main

import (
	"strings"
	"testing"
	"time"
)

var briefResultAt = time.Date(2026, 8, 26, 11, 0, 0, 0, time.Local)

// TestNormalizeSessionRefRefusesTheErrorClassThisFeatureExistsToKill pins the
// refusal by its real fixture: `session:6a-round2`, the line a live manager
// wrote into a brief and then had to repair by hand.
func TestNormalizeSessionRefRefusesTheErrorClassThisFeatureExistsToKill(t *testing.T) {
	for _, bad := range []string{
		"6a-round2", "session:6a-round2", "", "   ", "session:",
		"abc", "12345", "the-parser-worker", "ffff-2", "zzzzzzzz",
	} {
		if got, err := normalizeSessionRef(bad); err == nil {
			t.Errorf("normalizeSessionRef(%q) = %q, want a refusal", bad, got)
		}
	}
	// And it names the offending value, so the manager can see what it typed.
	if _, err := normalizeSessionRef("round2"); err == nil || !strings.Contains(err.Error(), `"round2"`) {
		t.Errorf("the refusal does not quote the bad value: %v", err)
	}
}

// TestNormalizeSessionRefCanonicalizes pins the short form the briefs and the
// board's REF_RE (`session:[0-9a-f]{6,}`) actually link on.
func TestNormalizeSessionRefCanonicalizes(t *testing.T) {
	for in, want := range map[string]string{
		"c03bd8ce-1f4a-4b2c-9d3e-0123456789ab": "c03bd8ce",
		"  session:C03BD8CE  ":                 "c03bd8ce",
		"a1b2c3d4e5f6a7b8":                     "a1b2c3d4",
		"a1b2c3":                               "a1b2c3",
	} {
		got, err := normalizeSessionRef(in)
		if err != nil {
			t.Fatalf("normalizeSessionRef(%q): %v", in, err)
		}
		if got != want {
			t.Errorf("normalizeSessionRef(%q) = %q, want %q", in, got, want)
		}
	}
}

// TestComposeResultLineHappyPath is the shape the desktop twin's own test pins,
// asserted here so the two providers cannot render the same result differently.
// TWIN: briefResultLine.test.ts "renders date, sentence, facts and reference".
func TestComposeResultLineHappyPath(t *testing.T) {
	got, err := composeResultLine(
		"the parser no longer allocates per token, which unblocks the mobile client",
		"c03bd8ce-1f4a-4b2c-9d3e-0123456789ab",
		map[string]any{
			"commit":       "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
			"filesChanged": []any{"src/parser.ts", "src/lexer.ts"},
			"checksRun":    []any{"vitest", "tsc"},
			"caveats":      []any{},
			"followUps":    []any{"delete the v1 path"},
		},
		briefResultAt,
	)
	if err != nil {
		t.Fatal(err)
	}
	want := "2026-08-26  the parser no longer allocates per token, which unblocks the mobile client — " +
		"commit: a1b2c3d4e5f6; filesChanged: src/parser.ts, src/lexer.ts; checksRun: vitest, tsc; " +
		"followUps: delete the v1 path (session:c03bd8ce)"
	if got != want {
		t.Errorf("composeResultLine:\n got %q\nwant %q", got, want)
	}
}

// TestComposeResultLineRequiresTheSignificanceSentence: a result object alone is
// never a brief line. Same rule renderDispatchTemplate enforces on {{task}}.
func TestComposeResultLineRequiresTheSignificanceSentence(t *testing.T) {
	for _, blank := range []string{"", "   ", "\n\t"} {
		_, err := composeResultLine(blank, "c03bd8ce", map[string]any{"commit": "abc1234"}, briefResultAt)
		if err == nil {
			t.Fatalf("composeResultLine(%q, …) produced a line from a result alone", blank)
		}
		if !strings.Contains(err.Error(), "one-sentence significance") {
			t.Errorf("the refusal does not say what is missing: %v", err)
		}
	}
}

// TestComposeResultLineRefusesAMalformedSessionIDBeforeComposing: nothing is
// written and nothing is guessed at.
func TestComposeResultLineRefusesAMalformedSessionID(t *testing.T) {
	if _, err := composeResultLine("round two landed", "6a-round2", nil, briefResultAt); err == nil {
		t.Fatal("a malformed session id composed a line")
	}
}

// TestResultFactsNeverDropACaveat is the one honesty promise: everything else
// may be capped; a caveat may not be dropped, capped or elided.
func TestResultFactsNeverDropACaveat(t *testing.T) {
	caveats := []any{
		"the migration is not reversible",
		"the fixture still uses the old schema",
		"CI is red on windows for an unrelated reason",
		"the flag defaults to on",
		"nobody has run this against prod data",
	}
	out := renderResultFacts(map[string]any{"commit": "abc1234", "caveats": caveats})
	for _, c := range caveats {
		if !strings.Contains(out, c.(string)) {
			t.Errorf("caveat %q was dropped from %q", c, out)
		}
	}
	if strings.Contains(out, "more") {
		t.Errorf("caveats were capped: %q", out)
	}
	// A very long caveat survives where any other field would be cut.
	long := strings.Repeat("x", 600)
	if !strings.Contains(renderResultFacts(map[string]any{"caveats": []any{long}}), long) {
		t.Error("a long caveat was truncated")
	}
	if !strings.Contains(renderResultFacts(map[string]any{"notes": long}), "chars)") {
		t.Error("a long non-caveat field was NOT capped — the cap is not applying at all")
	}
}

// TestResultFactsAreLossyButHonest: a capped list says how many it kept back,
// and an empty value is not reported as a fact.
func TestResultFactsAreLossyButHonest(t *testing.T) {
	out := renderResultFacts(map[string]any{
		"filesChanged": []any{"a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts"},
	})
	if out != "filesChanged: a.ts, b.ts, c.ts, +3 more" {
		t.Errorf("capped list rendered as %q", out)
	}
	// An EMPTY caveats list is a truthful "none", not a caveat.
	empty := renderResultFacts(map[string]any{
		"commit": "abc1234", "caveats": []any{}, "followUps": nil, "notes": "",
	})
	if empty != "commit: abc1234" {
		t.Errorf("empty values leaked into %q", empty)
	}
	// Arbitrary JSON: an unknown key is kept, not dropped, and sorts after the
	// known ones.
	arb := renderResultFacts(map[string]any{
		"zzz": "last", "followUps": []any{"f"}, "commit": "abc1234", "filesChanged": []any{"a"},
	})
	if arb != "commit: abc1234; filesChanged: a; followUps: f; zzz: last" {
		t.Errorf("arbitrary-key ordering: %q", arb)
	}
	// json.Unmarshal hands every number back as float64; an integer must not
	// arrive in the brief as "4".
	if n := renderResultFacts(map[string]any{"benchmarkMs": float64(12)}); n != "benchmarkMs: 12" {
		t.Errorf("integer rendering: %q", n)
	}
}

// TestComposeResultLineDoesNotStackADateOrARef: the caller's own date and an
// already-present reference are left alone.
func TestComposeResultLineDoesNotStackADateOrARef(t *testing.T) {
	got, err := composeResultLine("2026-08-24  backfilled from the handoff", "c03bd8ce", nil, briefResultAt)
	if err != nil {
		t.Fatal(err)
	}
	// The double space is the doctrine's dated-log format and must survive: a
	// \s+ collapse here would have re-spaced the one thing this tool writes.
	if got != "2026-08-24  backfilled from the handoff (session:c03bd8ce)" {
		t.Errorf("caller's date was restacked or re-spaced: %q", got)
	}
	dup, err := composeResultLine("landed, see session:c03bd8ce", "c03bd8ce-1f4a-4b2c-9d3e-0123456789ab", nil, briefResultAt)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Count(dup, "session:c03bd8ce") != 1 {
		t.Errorf("the reference was duplicated: %q", dup)
	}
}
