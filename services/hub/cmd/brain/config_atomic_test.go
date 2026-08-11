//go:build !windows

package main

import (
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
)

// TestWriteConfigYAMLIsAtomic proves writeConfigYAML replaces config.yaml with a
// temp-file + rename (crash-safe), not a truncate-in-place os.WriteFile. A
// truncating write reuses the target's inode and leaves the file corrupt if the
// process is killed mid-write; an atomic rename swaps in a fresh, fully-written
// inode. So a differing inode across two writes is the fingerprint of the
// crash-safe path — the guarantee the desktop's atomicWriteFileSync gives.
func TestWriteConfigYAMLIsAtomic(t *testing.T) {
	dir := tempConfigHome(t)

	writeConfigYAML(map[string]any{"ui": map[string]any{"theme": "one"}})
	p := configPath()
	st1, err := os.Stat(p)
	if err != nil {
		t.Fatalf("first write did not produce config.yaml: %v", err)
	}
	ino1 := st1.Sys().(*syscall.Stat_t).Ino

	writeConfigYAML(map[string]any{"ui": map[string]any{"theme": "two"}})
	st2, err := os.Stat(p)
	if err != nil {
		t.Fatalf("second write did not produce config.yaml: %v", err)
	}
	ino2 := st2.Sys().(*syscall.Stat_t).Ino

	if ino1 == ino2 {
		t.Fatalf("writeConfigYAML reused inode %d across two writes — it truncates config.yaml in place instead of temp+rename, so a crash/power-loss mid-write corrupts the file", ino1)
	}

	// The atomic write must not leave temp files beside the target.
	entries, err := os.ReadDir(filepath.Join(dir, "workspacer"))
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if e.Name() != "config.yaml" {
			t.Errorf("leftover file in config dir after atomic write: %s", e.Name())
		}
	}
}

// TestFileBackedStoresWriteAtomically applies the same inode fingerprint to the
// other stores. It matters MORE here than for config.yaml: these methods are
// brain-delegated by default, so the desktop's atomicWriteFileSync — the
// implementation everyone points at when asked whether saves are crash-safe — is
// the one that never runs. A layout or saved session truncated in place by a
// kill mid-write comes back as unparseable YAML, and list() just skips it.
func TestFileBackedStoresWriteAtomically(t *testing.T) {
	tempConfigHome(t)

	ino := func(t *testing.T, path string) uint64 {
		t.Helper()
		st, err := os.Stat(path)
		if err != nil {
			t.Fatalf("write did not produce %s: %v", path, err)
		}
		return st.Sys().(*syscall.Stat_t).Ino
	}

	if _, err := saveSavedSession("Work", map[string]any{"name": "Work", "agents": []any{}}); err != nil {
		t.Fatal(err)
	}
	sessionPath := filepath.Join(sessionsDir(), "work.yaml")
	sessIno1 := ino(t, sessionPath)
	if _, err := saveSavedSession("Work", map[string]any{"name": "Work", "agents": []any{}}); err != nil {
		t.Fatal(err)
	}
	if sessIno2 := ino(t, sessionPath); sessIno1 == sessIno2 {
		t.Errorf("saveSavedSession reused inode %d — it truncates the session file in place instead of temp+rename", sessIno1)
	}

	if _, err := saveLayout(map[string]any{"name": "My Layout", "agents": []any{}}); err != nil {
		t.Fatal(err)
	}
	layoutPath := filepath.Join(layoutsDir(), "my-layout.yaml")
	layIno1 := ino(t, layoutPath)
	if _, err := saveLayout(map[string]any{"name": "My Layout", "agents": []any{}}); err != nil {
		t.Fatal(err)
	}
	if layIno2 := ino(t, layoutPath); layIno1 == layIno2 {
		t.Errorf("saveLayout reused inode %d — it truncates the layout file in place instead of temp+rename", layIno1)
	}

	// No temp files survive either write.
	for _, d := range []string{sessionsDir(), layoutsDir()} {
		entries, err := os.ReadDir(d)
		if err != nil {
			t.Fatal(err)
		}
		for _, e := range entries {
			if strings.Contains(e.Name(), ".tmp-") {
				t.Errorf("leftover temp file in %s: %s", d, e.Name())
			}
		}
	}
}
