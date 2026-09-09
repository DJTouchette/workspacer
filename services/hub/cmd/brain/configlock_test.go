package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"time"
)

// The TS twin (apps/desktop/src/main/lib/configLock.test.ts) covers the same
// cases. Both assert against contracts/config-lock.json.

func lockOf(p string) string { return p + lockFileSuffix }

func tempConfig(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	p := filepath.Join(dir, "config.yaml")
	if err := os.WriteFile(p, []byte("ui: {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestWithConfigLockRunsAndReleases(t *testing.T) {
	cfg := tempConfig(t)
	heldDuringBody := false
	if err := withConfigLock(cfg, func() error {
		_, err := os.Stat(lockOf(cfg))
		heldDuringBody = err == nil
		return nil
	}); err != nil {
		t.Fatalf("lock: %v", err)
	}
	if !heldDuringBody {
		t.Error("the lock must be held FOR the body")
	}
	if _, err := os.Stat(lockOf(cfg)); !os.IsNotExist(err) {
		t.Error("the lock must be released after")
	}
}

func TestWithConfigLockReleasesOnError(t *testing.T) {
	cfg := tempConfig(t)
	boom := os.ErrInvalid
	if err := withConfigLock(cfg, func() error { return boom }); err != boom {
		t.Fatalf("the body's error must propagate, got %v", err)
	}
	if _, err := os.Stat(lockOf(cfg)); !os.IsNotExist(err) {
		t.Error("a failed write must not wedge config until the stale timeout")
	}
}

// The case the lock exists for: the other process is mid-write.
func TestWithConfigLockRefusesWhenHeld(t *testing.T) {
	cfg := tempConfig(t)
	if err := os.WriteFile(lockOf(cfg), []byte("9999 held\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	ran := false
	err := withConfigLock(cfg, func() error { ran = true; return nil })
	if err != errConfigLocked {
		t.Fatalf("want errConfigLocked, got %v", err)
	}
	if ran {
		t.Error("the body must NOT run — writing anyway is the bug this prevents")
	}
	if _, err := os.Stat(lockOf(cfg)); err != nil {
		t.Error("the other side's lock must be left alone")
	}
}

func TestWithConfigLockStealsADeadHolder(t *testing.T) {
	cfg := tempConfig(t)
	if err := os.WriteFile(lockOf(cfg), []byte("9999 crashed\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-(lockStaleMs*time.Millisecond + time.Second))
	if err := os.Chtimes(lockOf(cfg), old, old); err != nil {
		t.Fatal(err)
	}
	ran := false
	if err := withConfigLock(cfg, func() error { ran = true; return nil }); err != nil {
		t.Fatalf("lock: %v", err)
	}
	if !ran {
		t.Error("a dead holder must not wedge config forever")
	}
}

func TestConfigLockMatchesContract(t *testing.T) {
	raw := mustReadRepoFile(t, "contracts", "config-lock.json")
	var fixture struct {
		LockFileSuffix string `json:"lockFileSuffix"`
		StaleMs        int    `json:"staleMs"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	// staleMs is the correctness parameter: a side that expires locks sooner than
	// the other steals one the other still believes it holds.
	if fixture.StaleMs != lockStaleMs {
		t.Errorf("lockStaleMs = %d, contract says %d", lockStaleMs, fixture.StaleMs)
	}
	if fixture.LockFileSuffix != lockFileSuffix {
		t.Errorf("lockFileSuffix = %q, contract says %q", lockFileSuffix, fixture.LockFileSuffix)
	}
}

// A lock timeout is transient — an orphaned lockfile is stolen after staleMs.
// Latching persistBlocked on it turned a ten-second obstruction into a
// permanently write-only daemon that still reported every save as applied,
// because saveLocked's persistBlocked branch returns before writeConfigYAML and
// so can never clear the flag it set.
func TestSaveRecoversAfterALockTimeout(t *testing.T) {
	tempConfigHome(t)
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)

	c := &configService{}
	// Prime it so there is an on-disk file and a loaded cache.
	c.save(map[string]any{"ui": map[string]any{"theme": "one"}})

	// Somebody else holds the lock — a desktop force-quit mid-write.
	lock := configPath() + lockFileSuffix
	if err := os.WriteFile(lock, []byte("9999 held\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	got := mustSave(t, c, map[string]any{"ui": map[string]any{"theme": "two"}})
	if theme := themeOf(got); theme != "one" {
		t.Errorf("a refused save must report the UNCHANGED value, got %q", theme)
	}

	// The holder goes away (released, or stolen once stale).
	if err := os.Remove(lock); err != nil {
		t.Fatal(err)
	}
	got = mustSave(t, c, map[string]any{"ui": map[string]any{"theme": "three"}})
	if theme := themeOf(got); theme != "three" {
		t.Fatalf("the next save must work again, got %q — the daemon latched write-only", theme)
	}
	// ...and it must actually be on disk, not just in memory.
	raw, err := os.ReadFile(configPath())
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "three") {
		t.Error("reported success but wrote nothing")
	}
}

func themeOf(cfg map[string]any) string {
	ui, _ := cfg["ui"].(map[string]any)
	s, _ := ui["theme"].(string)
	return s
}

func TestLockContentionClassifiesWindowsDeletePending(t *testing.T) {
	for _, tc := range []struct {
		err  error
		want bool
	}{
		{os.ErrExist, true},
		{os.ErrPermission, runtime.GOOS == "windows"},
		{syscall.Errno(32), runtime.GOOS == "windows"},
		{os.ErrNotExist, false},
		{os.ErrInvalid, false},
	} {
		err := &os.PathError{Op: "open", Path: "brief.md.lock", Err: tc.err}
		if got := isLockContention(err); got != tc.want {
			t.Errorf("isLockContention(%v) = %v, want %v", err, got, tc.want)
		}
	}
}

// A stale lock can become unremovable (for example, a nonempty directory at
// the lock path). Failed cleanup must still obey the wait budget and must never
// enter the protected write. The old unconditional retry spun forever here.
func TestFileLocksBoundFailedStaleRemoval(t *testing.T) {
	for _, tc := range []struct {
		name string
		lock func(string, func() error) error
	}{
		{"config", withConfigLock},
		{"brief", func(path string, fn func() error) error {
			return withFileLock(path, 50*time.Millisecond, time.Second, fn)
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "document")
			lockPath := path + lockFileSuffix
			if err := os.Mkdir(lockPath, 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(lockPath, "child"), nil, 0o600); err != nil {
				t.Fatal(err)
			}
			stale := time.Now().Add(-time.Minute)
			if err := os.Chtimes(lockPath, stale, stale); err != nil {
				t.Fatal(err)
			}
			called := false
			started := time.Now()
			err := tc.lock(path, func() error { called = true; return nil })
			if err == nil || called {
				t.Fatalf("unremovable lock admitted write: err=%v called=%v", err, called)
			}
			if elapsed := time.Since(started); elapsed > 5*time.Second {
				t.Fatalf("lock exceeded bounded wait: %s", elapsed)
			}
		})
	}
}
