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
	"strings"
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
			// ReasonWithOverage is what the reason must become with the standing
			// overage bit set. Empty means the bit changes nothing, which is the
			// answer for every wording that is not about usage or credits.
			ReasonWithOverage string `json:"reasonWithOverage"`
			// CreditBalance is whether the credit-balance remedy may attach.
			CreditBalance bool   `json:"creditBalance"`
			Why           string `json:"why"`
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
	// The two extra readers must be pinned non-vacuously: a fixture whose new
	// fields nobody set would leave the enrichment and the remedy tested by
	// nothing while every case still passed.
	var anyOverage, anyCredit bool
	for _, c := range doc.Cases {
		anyOverage = anyOverage || c.ReasonWithOverage != ""
		anyCredit = anyCredit || c.CreditBalance
	}
	if !anyOverage || !anyCredit {
		t.Fatalf("the fixture sets no reasonWithOverage (%v) or no creditBalance (%v), so both readers below would be vacuous", anyOverage, anyCredit)
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
		// With the standing overage bit set. It may only ever ADD to a failure
		// the marker already established, and only for a usage/credits wording:
		// an authentication failure relabelled "out of credits (overage
		// disabled) - Not logged in" is the mislabel this pins shut.
		wantEnriched := c.Reason
		if c.ReasonWithOverage != "" {
			wantEnriched = c.ReasonWithOverage
		}
		enriched, failedWithBit := workerFailureReason(true, c.FinalMessage)
		if failedWithBit != c.Failed {
			t.Errorf("%s (overage set): failed = %v, want %v\n  why: %s", c.Name, failedWithBit, c.Failed, c.Why)
		} else if c.Failed && enriched != wantEnriched {
			t.Errorf("%s (overage set): reason = %q, want %q\n  why: %s", c.Name, enriched, wantEnriched, c.Why)
		}
		// And the remedy gate. TWIN: isCreditBalanceFailureText's cases in
		// workerFailure.test.ts, which reads the same field.
		gotCredit := false
		if marker, ok := errorMarkerReason(c.FinalMessage); ok {
			gotCredit = creditBalanceTooLow(marker)
		}
		if gotCredit != c.CreditBalance {
			t.Errorf("%s: credit-balance remedy attaches = %v, want %v\n  why: %s", c.Name, gotCredit, c.CreditBalance, c.Why)
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

// The remedy has to reach the manager, not just exist as a constant: a headless
// `workspacer serve` fleet wake is the ONLY surface the brain has for it, and
// before this it carried the raw API text and none of the fix.
func TestFleetWakeCarriesTheCreditBalanceRemedy(t *testing.T) {
	reason, _ := workerFailureReason(false, agentErrorMarker+"Credit balance is too low to access the Anthropic API.")
	wake := buildFleetMessage(fleetWorkerFailedHeader, fleetWorkerFinishedTail, []fleetEntry{
		{Label: "worker", SessionID: "s1", Cwd: "/p", Failed: reason},
	})
	for _, want := range []string{fleetCreditBalanceNotePrefix, "/logout", "/login", "Anthropic Console"} {
		if !strings.Contains(wake, want) {
			t.Errorf("the wake does not carry %q, so the manager gets the raw API refusal and no fix:\n%s", want, wake)
		}
	}

	// And ONLY for that refusal. A 529 that collected a re-login instruction
	// would send the manager, and through it the user, down a credential path
	// for a retryable server hiccup.
	other, _ := workerFailureReason(false, agentErrorMarker+"API Error: 529 Overloaded. This is a server-side issue, usually temporary.")
	wake = buildFleetMessage(fleetWorkerFailedHeader, fleetWorkerFinishedTail, []fleetEntry{
		{Label: "worker", SessionID: "s1", Cwd: "/p", Failed: other},
	})
	if strings.Contains(wake, fleetCreditBalanceNotePrefix) {
		t.Errorf("a 529 overload collected the credit-balance remedy:\n%s", wake)
	}
	if !strings.Contains(wake, fleetFailedNote) {
		t.Error("the generic FAILED note went missing")
	}
}
