package main

import (
	"os"
	"path/filepath"
	"testing"
)

// The brain only looks for the record where the node's entrypoint puts it, and
// only when WKS_DATA says there is a node volume at all. Unset — a desktop, a
// laptop, a dev box — and it looks nowhere.
func TestLastExitPathIsDerivedFromTheVolumeAndNothingElse(t *testing.T) {
	t.Setenv("WKS_DATA", "")
	if got := lastExitPath(); got != "" {
		t.Errorf("lastExitPath with no WKS_DATA = %q, want empty", got)
	}
	t.Setenv("WKS_DATA", "/data")
	if want := filepath.Join("/data", "state", "last-exit.json"); lastExitPath() != want {
		t.Errorf("lastExitPath = %q, want %q", lastExitPath(), want)
	}
}

// The record is read from the exact bytes the entrypoint writes.
func TestTheBrainReadsWhatTheEntrypointWrites(t *testing.T) {
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, "state"), 0o755); err != nil {
		t.Fatal(err)
	}
	// Byte-for-byte the entrypoint's printf format (deploy/fly/node/entrypoint.sh).
	body := `{"bootId":"20260824T210000Z-abc","reason":"claudemon-died","exitCode":1,` +
		`"at":"2026-08-24T21:00:00Z","machine":"17811944b12345"}` + "\n"
	if err := os.WriteFile(filepath.Join(dir, "state", "last-exit.json"), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("WKS_DATA", dir)

	// lastExit() caches per process, so exercise the parse directly rather
	// than fighting the sync.Once.
	rec := readExitRecord(lastExitPath())
	if rec == nil {
		t.Fatal("no record parsed from the entrypoint's own output format")
	}
	if rec.Reason != "claudemon-died" || rec.At != "2026-08-24T21:00:00Z" {
		t.Fatalf("parsed %+v", rec)
	}
	if rec.ExitCode == nil || *rec.ExitCode != 1 {
		t.Fatalf("exit code = %v, want 1", rec.ExitCode)
	}
}

// A missing, unreadable or nonsense file is "no record", never a fabricated
// one: "nobody knows" and "it ended cleanly" must stay different answers.
func TestAnUnreadableRecordIsNoRecord(t *testing.T) {
	dir := t.TempDir()
	if rec := readExitRecord(filepath.Join(dir, "absent.json")); rec != nil {
		t.Errorf("a missing file produced %+v", rec)
	}
	bad := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(bad, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := readExitRecord(bad); rec != nil {
		t.Errorf("a corrupt file produced %+v", rec)
	}
	empty := filepath.Join(dir, "empty.json")
	if err := os.WriteFile(empty, []byte(`{"reason":""}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := readExitRecord(empty); rec != nil {
		t.Errorf("a record with no reason produced %+v", rec)
	}
}
