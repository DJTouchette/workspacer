package main

import (
	"os"
	"path/filepath"
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
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

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
