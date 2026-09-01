//go:build !windows

package routing

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDecisionLogUses0600ModeOnUnix(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	NewDecisionLog(path, 0).Decision(Decision{DecisionID: "rd_x"})
	st, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if perm := st.Mode().Perm(); perm != 0o600 {
		t.Errorf("decision log mode %v, want 0600", perm)
	}
}

func TestDecisionLogRepairsALooseUnixModeBeforeAppend(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	if err := os.WriteFile(path, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	if st, err := os.Stat(path); err != nil || st.Mode().Perm() != 0o644 {
		t.Fatalf("mode mutation did not make the guard red: stat=%v err=%v", st, err)
	}

	NewDecisionLog(path, 0).Decision(Decision{DecisionID: "rd_repaired"})
	st, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if perm := st.Mode().Perm(); perm != 0o600 {
		t.Fatalf("append did not repair mode %v to 0600", perm)
	}
	if entries := readEntries(t, path); len(entries) != 1 || entries[0].DecisionID != "rd_repaired" {
		t.Fatalf("repaired log entries = %+v", entries)
	}
}

func TestDecisionLogRepairsALooseUnixModeBeforeRotation(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-decisions.jsonl")
	if err := os.WriteFile(path, []byte("loose oversized generation\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// The process umask may narrow WriteFile's requested mode, so make the
	// precondition explicit: the retained bytes really are loose before the
	// production rotation path sees them.
	if err := os.Chmod(path, 0o644); err != nil {
		t.Fatal(err)
	}
	if st, err := os.Stat(path); err != nil || st.Mode().Perm() != 0o644 {
		t.Fatalf("mode mutation did not make the rotation guard red: stat=%v err=%v", st, err)
	}

	NewDecisionLog(path, 1).Decision(Decision{DecisionID: "rd_rotated_private"})
	rotated := path + ".1"
	st, err := os.Stat(rotated)
	if err != nil {
		t.Fatal(err)
	}
	if perm := st.Mode().Perm(); perm != 0o600 {
		t.Fatalf("rotated generation mode = %v, want 0600", perm)
	}
	if entries := readEntries(t, path); len(entries) != 1 || entries[0].DecisionID != "rd_rotated_private" {
		t.Fatalf("new live generation entries = %+v", entries)
	}
}
