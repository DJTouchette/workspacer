package main

// The agent-error marker, held to the SAME cross-language fixture as the Rust
// writer that produces it and the TypeScript reader on the desktop.
//
// The fixture (contracts/agent-error-marker-cases.json) is the contract: the
// non-failure cases are the load-bearing half, because a reader that fired on
// prose merely MENTIONING an error would relabel every landed dispatch as a
// crash — a new bug worse than the one this closes.

import (
	"encoding/json"
	"testing"
)

func TestWorkerFailureMatchesTheAgentErrorMarkerContract(t *testing.T) {
	var doc struct {
		Marker string `json:"marker"`
		Cases  []struct {
			Name         string `json:"name"`
			FinalMessage string `json:"finalMessage"`
			Failed       bool   `json:"failed"`
			Reason       string `json:"reason"`
			Why          string `json:"why"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(mustReadRepoFile(t, "contracts", "agent-error-marker-cases.json"), &doc); err != nil {
		t.Fatalf("parse the fixture: %v", err)
	}
	if doc.Marker != agentErrorMarker {
		t.Fatalf("the marker itself has drifted:\n  go:      %q\n  fixture: %q\n"+
			"claudemon WRITES this prefix; a reader spelling it differently cannot tell a dead worker from a finished one.",
			agentErrorMarker, doc.Marker)
	}
	if len(doc.Cases) < 8 {
		t.Fatalf("only %d cases loaded — the fixture is the whole point of this test", len(doc.Cases))
	}
	for _, c := range doc.Cases {
		reason, failed := workerFailureReason(false, c.FinalMessage)
		if failed != c.Failed {
			t.Errorf("%s: failed = %v, want %v\n  message: %q\n  why: %s", c.Name, failed, c.Failed, c.FinalMessage, c.Why)
			continue
		}
		if c.Failed && reason != c.Reason {
			t.Errorf("%s: reason = %q, want %q\n  why: %s", c.Name, reason, c.Reason, c.Why)
		}
	}
}

// Standing ACCOUNT state cannot CREATE a failure — only enrich one. Four
// completed, correctly-merged dispatches woke their manager as FAILED on
// 2026-08-22 purely because the account's overage window had no headroom.
func TestOutOfCreditsOnlyEnrichesAFailureItDidNotCreate(t *testing.T) {
	if _, failed := workerFailureReason(true, "All 42 tests pass. Merged as abc1234."); failed {
		t.Error("a clean finish on a no-headroom account was reported as a crash — the exact 2026-08-22 regression")
	}
	reason, failed := workerFailureReason(true, agentErrorMarker+"Credit balance is too low to access the Anthropic API.")
	if !failed {
		t.Fatal("a marked error turn was not reported as a failure")
	}
	want := "out of credits (overage disabled) - Credit balance is too low to access the Anthropic API."
	if reason != want {
		t.Errorf("reason = %q, want %q", reason, want)
	}
}

// The reason rides a wake BULLET, whose grammar splits on " — ". A reason
// carrying that separator would make the bullet unparseable — the card would
// degrade to a raw text blob on every client — so it is sanitized, not escaped.
func TestFailureReasonCannotForgeTheBulletSeparator(t *testing.T) {
	reason, failed := workerFailureReason(false, agentErrorMarker+"boom — FAILED: not really — last reply: nonsense")
	if !failed {
		t.Fatal("expected a failure")
	}
	if got := reason; got != "boom - FAILED: not really - last reply: nonsense" {
		t.Errorf("reason = %q — the bullet separator survived", got)
	}
	e := fleetEntry{Label: "w", SessionID: "s1", Cwd: "/p", Failed: reason}
	bullet := formatFleetEntry(e)
	if n := countSeparators(bullet); n != 1 {
		t.Errorf("bullet has %d %q separators, want exactly 1 (the FAILED tail's own):\n%s", n, " — ", bullet)
	}
}

func countSeparators(s string) int {
	n := 0
	for i := 0; i+len(" — ") <= len(s); i++ {
		if s[i:i+len(" — ")] == " — " {
			n++
		}
	}
	return n
}

// A reason longer than the cap is cut, not wrapped: a bullet must stay one line
// and must not crowd out the rest of the entry.
func TestFailureReasonIsCapped(t *testing.T) {
	long := ""
	for i := 0; i < 400; i++ {
		long += "x"
	}
	reason, failed := workerFailureReason(false, agentErrorMarker+long)
	if !failed {
		t.Fatal("expected a failure")
	}
	if len([]rune(reason)) != failureReasonMax+1 { // + the ellipsis
		t.Errorf("reason is %d runes, want %d + an ellipsis", len([]rune(reason)), failureReasonMax)
	}
}
