package routing

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func readEntries(t *testing.T, path string) []Entry {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	var out []Entry
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		if line == "" {
			continue
		}
		var e Entry
		if err := json.Unmarshal([]byte(line), &e); err != nil {
			t.Fatalf("line is not JSON (%v): %s", err, line)
		}
		out = append(out, e)
	}
	return out
}

// THE JOIN is the whole point of the decisionId: one decision row and one spawn
// row that a reader can put together without the ticket store, on a node that
// records no analytics of its own.
func TestDecisionLogJoinsADecisionToTheSpawnItProduced(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	l := NewDecisionLog(path, DefaultDecisionLogMaxBytes)

	id := NewDecisionID()
	if id == "" || !strings.HasPrefix(id, "rd_") {
		t.Fatalf("NewDecisionID gave %q", id)
	}
	l.Decision(Decision{DecisionID: id, Role: "implementer", Capability: "frontier", Provider: "codex", Model: "gpt-5.6-sol"})
	l.Spawn(id, SpawnEntry{Role: "implementer", Capability: "frontier", Cwd: "/w", Provider: "codex", Model: "gpt-5.6-sol"})

	entries := readEntries(t, path)
	if len(entries) != 2 {
		t.Fatalf("wrote %d rows, want 2", len(entries))
	}
	if entries[0].Kind != KindDecision || entries[1].Kind != KindSpawn {
		t.Fatalf("kinds %q/%q", entries[0].Kind, entries[1].Kind)
	}
	if entries[0].DecisionID != id || entries[1].DecisionID != id {
		t.Errorf("the two rows do not join: %q vs %q", entries[0].DecisionID, entries[1].DecisionID)
	}
	if entries[0].Decision == nil || entries[0].Decision.Model != "gpt-5.6-sol" {
		t.Errorf("the decision row lost its payload: %+v", entries[0])
	}
	if entries[1].Spawn == nil || entries[1].Spawn.Cwd != "/w" {
		t.Errorf("the spawn row lost its payload: %+v", entries[1])
	}
	if entries[0].At.IsZero() || entries[1].At.IsZero() {
		t.Error("a row with no timestamp cannot be used to calibrate anything")
	}

	// Two ids in a row must differ, or two decisions a second apart would join a
	// spawn to whichever came first.
	if NewDecisionID() == NewDecisionID() {
		t.Error("NewDecisionID repeated itself")
	}
}

// Mode 0600, like routing.yaml and jobs.json beside it: the log names project
// directories, the models spent in them, and which credential asked.
func TestDecisionLogIsWrittenPrivate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "sub", "routing-decisions.jsonl")
	NewDecisionLog(path, 0).Decision(Decision{DecisionID: "rd_x"})
	st, err := os.Stat(path)
	if err != nil {
		t.Fatalf("the log was not created (the directory should have been): %v", err)
	}
	if perm := st.Mode().Perm(); perm != 0o600 {
		t.Errorf("decision log mode %v, want 0600", perm)
	}
}

// Rotation, not trimming: the point of an append-only file is that growing it
// never means reading and rewriting it.
func TestDecisionLogRotatesRatherThanTrimming(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "routing-decisions.jsonl")
	// Comfortably above one row (a decision row runs a few hundred bytes) so the
	// cap bounds a HISTORY rather than a single line, which is the case the
	// rotation is actually for.
	const cap = 4000
	l := NewDecisionLog(path, cap)

	for i := 0; i < 40; i++ {
		l.Decision(Decision{DecisionID: "rd_" + strings.Repeat("a", 20), Role: "scout"})
	}
	st, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if st.Size() > cap {
		t.Errorf("the live log is %d bytes against a %d-byte cap — the check must run BEFORE the write, or the cap is the point it is already past", st.Size(), cap)
	}
	if _, err := os.Stat(path + ".1"); err != nil {
		t.Errorf("no previous generation at %s.1 — rotation dropped the history instead of keeping one generation", path)
	}
	// Whatever is in the live file is still whole lines.
	readEntries(t, path)
}

// A disabled log and a nil one are both no-ops, so no caller has to ask whether
// logging is on before recording anything.
func TestDecisionLogDisabledIsSilent(t *testing.T) {
	var nilLog *DecisionLog
	nilLog.Decision(Decision{DecisionID: "x"})
	nilLog.Spawn("x", SpawnEntry{})
	if nilLog.Path() != "" {
		t.Error("a nil log claims a path")
	}

	off := NewDecisionLog("", 0)
	off.Decision(Decision{DecisionID: "x"})
	off.Spawn("x", SpawnEntry{})
}

func TestDecisionLogPathSitsBesideTheMatrix(t *testing.T) {
	if got := DecisionLogPathFor("/home/u/.config/workspacer-hub/routing.yaml"); got != "/home/u/.config/workspacer-hub/routing-decisions.jsonl" {
		t.Errorf("DecisionLogPathFor = %q", got)
	}
	if got := DecisionLogPathFor(""); got != "" {
		t.Errorf("a disabled matrix file must disable the log too, got %q", got)
	}
}
