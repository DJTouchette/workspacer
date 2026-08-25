package nodes

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestOnlyASignalCountsAsACleanEnding(t *testing.T) {
	clean := []string{"signal-TERM", "signal-INT"}
	dirty := []string{"claudemon-died", "brain-died", "boot-failure", "unknown", "", "something-new"}
	for _, r := range clean {
		if !(&ExitRecord{Reason: r}).Clean() {
			t.Errorf("%q should be a clean ending", r)
		}
	}
	for _, r := range dirty {
		if (&ExitRecord{Reason: r}).Clean() {
			t.Errorf("%q must NOT count as a clean ending", r)
		}
	}
	// A record the hub does not hold at all is not evidence of a clean stop.
	if (*ExitRecord)(nil).Clean() {
		t.Error("a nil record reported itself clean — the direction must err toward telling the user to look")
	}
}

// THE CASE THE CLOUD API CANNOT SEE. A node that ran fine, crashed, and left
// its machine stopped is `stopped` in the cloud API exactly like one somebody
// put to sleep. The node's own record is the only account of the difference,
// and the hub can only read it once the node is back up — so it must SAY so
// when it does, rather than quietly moving on.
func TestANodeThatComesBackFromACrashSaysSo(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	code := 1
	h.bus.setExit(&ExitRecord{Reason: "claudemon-died", ExitCode: &code, At: "2026-08-24T21:00:00Z"})
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())

	v := h.view(t, "den")
	if v.State != string(StateAvailable) {
		t.Fatalf("state = %q, want available — the node IS working now", v.State)
	}
	if v.LastExit == nil || v.LastExit.Reason != "claudemon-died" {
		t.Fatalf("lastExit = %+v, want the crash record", v.LastExit)
	}
	if !strings.Contains(strings.ToLower(v.Detail), "did not end cleanly") {
		t.Errorf("detail = %q — a node that came back from a crash must say so, or the crash is invisible", v.Detail)
	}
}

// A node whose previous run ended on a signal is unremarkable and must not be
// dressed up as a problem.
func TestACleanPreviousEndingAddsNoAlarm(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.bus.setExit(&ExitRecord{Reason: "signal-TERM", At: "2026-08-24T21:00:00Z"})
	h.bus.set("den", nil, true)
	h.sup.Reconcile(context.Background())
	v := h.view(t, "den")
	if v.Detail != "" {
		t.Errorf("detail = %q, want empty for a clean previous ending", v.Detail)
	}
	if v.LastExit == nil || !v.LastExit.Clean() {
		t.Errorf("lastExit = %+v, want the clean record still reported", v.LastExit)
	}
}

// The exit record is a projection too: the file on the node's disk also
// carries a bootId and the Fly machine id, and a client needs neither.
func TestTheExitRecordDisclosesNoMachineIdentifiers(t *testing.T) {
	// What the entrypoint actually writes.
	onDisk := `{"bootId":"20260824T210000Z-abc123","reason":"claudemon-died","exitCode":1,` +
		`"at":"2026-08-24T21:00:00Z","machine":"17811944b12345"}`
	var rec ExitRecord
	if err := json.Unmarshal([]byte(onDisk), &rec); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(rec)
	for _, leak := range []string{"bootId", "20260824T210000Z-abc123", "machine", "17811944b12345"} {
		if strings.Contains(string(raw), leak) {
			t.Errorf("ExitRecord carried %q through to the wire: %s", leak, raw)
		}
	}
	if rec.Reason != "claudemon-died" || rec.At != "2026-08-24T21:00:00Z" || rec.ExitCode == nil || *rec.ExitCode != 1 {
		t.Errorf("parsed %+v — the three fields a person reads must survive", rec)
	}
}

// The record is never invented. A hub that has never seen a node up reports no
// record at all rather than an empty one, so a client can tell "it ended
// cleanly" from "nobody knows".
func TestNoExitRecordIsReportedForANodeTheHubHasNeverSeenUp(t *testing.T) {
	h := newHarness(t, []Node{flyNode("den")}, Tunables{})
	h.sup.Reconcile(context.Background())
	v := h.view(t, "den")
	if v.LastExit != nil {
		t.Errorf("lastExit = %+v for a node the hub has never seen up", v.LastExit)
	}
	raw, _ := json.Marshal(v)
	if strings.Contains(string(raw), "lastExit") {
		t.Errorf("an absent record still appeared on the wire: %s", raw)
	}
}
