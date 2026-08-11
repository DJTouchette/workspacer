package main

// The FAILURE PLANE of config.save: what the caller is told when the bytes do
// not reach disk. The brain is the SOLE provider of config.save in the shipped
// default (DELEGATE_CATALOG_TO_BRAIN, renderer in bus mode), and whatever save()
// returns is what the Settings pane paints as the applied value — so returning
// a value that is not on disk IS the success report.
//
// Twinned with apps/desktop/src/main/services/configService.test.ts
// ("save failure plane"): every case here has a case there, because the two
// writers answer the same config.save in different configurations.

import (
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func seedConfig(t *testing.T, yamlText string) string {
	t.Helper()
	dir := tempConfigHome(t)
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(yamlText), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// A save whose write fails must report the OLD value and keep serving it. The
// mutant this kills: `_ = writeConfigYAML(merged)` (or dropping writeConfigYAML's
// error return), which reports a SUCCESSFUL config.save for a file that never
// reached disk and then serves the phantom for the life of the daemon.
func TestSaveWhoseWriteFailsReportsTheOldValueAndDoesNotAdoptIt(t *testing.T) {
	seedConfig(t, "ui:\n  theme: everforest\n")

	c := newConfigService()
	if got := themeOf(c.get()); got != "everforest" {
		t.Fatalf("seed did not load: theme = %q", got)
	}

	// ENOSPC on the atomic rename: the directory is writable (so the
	// cross-process lock still acquires) and config.yaml is readable (so the
	// pre-save re-read succeeds); only the write dies.
	restore := writeConfig
	writeConfig = func(map[string]any) error { return errors.New("write /…/config.yaml: no space left on device") }
	t.Cleanup(func() { writeConfig = restore })

	got := c.save(map[string]any{"ui": map[string]any{"theme": "nord"}})

	if themeOf(got) != "everforest" {
		t.Errorf("save() RETURNED the value it failed to write (%q): the client renders the setting as applied and it reverts on the next read",
			themeOf(got))
	}
	if themeOf(c.get()) != "everforest" {
		t.Errorf("the in-memory cache ADOPTED a value that is not on disk (%q): every later get() serves a phantom setting for the life of the process",
			themeOf(c.get()))
	}
}

// writeConfigYAML must RETURN its write error rather than logging it away: it is
// the only thing that tells saveLocked the bytes did not land.
func TestWriteConfigYAMLReturnsTheWriteError(t *testing.T) {
	// POSIX-only, and skipped rather than tagged so the rest of this file still
	// runs on Windows: the unwritable directory is made with a 0500 mode, which
	// Windows does not enforce for the owner, so the write would SUCCEED there
	// and the test would fail claiming the error was swallowed. Same reason
	// config_atomic_test.go carries //go:build !windows for its inode check.
	if runtime.GOOS == "windows" {
		t.Skip("a 0500 directory is still writable on Windows; the premise does not hold")
	}
	dir := tempConfigHome(t)
	// The config dir exists but is not writable: the temp file cannot be made.
	if err := os.MkdirAll(filepath.Join(dir, "workspacer"), 0o500); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(filepath.Join(dir, "workspacer"), 0o755) })
	if os.Geteuid() == 0 {
		t.Skip("running as root: a 0500 directory is still writable")
	}
	if err := writeConfigYAML(map[string]any{"ui": map[string]any{"theme": "nord"}}); err == nil {
		t.Fatal("writeConfigYAML swallowed a failed write: saveLocked would report a SUCCESSFUL config.save for a file that never reached disk")
	}
}

// A config.yaml that EXISTS but cannot be READ (EACCES/EBUSY) must latch
// persistBlocked, so one save never replaces a recoverable user file with
// defaults+partial. The unparseable half of that latch is pinned by
// TestConfigSaveDoesNotClobberUnparseableFile; this is the unreadable half.
func TestSaveDoesNotClobberAConfigItCouldNotRead(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("running as root: mode 0000 is still readable")
	}
	original := "ui:\n  theme: solarized\nterminal:\n  fontSize: 19\n"
	p := seedConfig(t, original)
	if err := os.Chmod(p, 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(p, 0o644) })

	c := newConfigService()
	c.save(map[string]any{"editor": map[string]any{"vim": true}})

	if err := os.Chmod(p, 0o644); err != nil {
		t.Fatal(err)
	}
	after, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("config.yaml disappeared: %v", err)
	}
	if string(after) != original {
		t.Fatalf("save() overwrote a config.yaml it could not READ, discarding every other setting.\n got: %q\nwant: %q", string(after), original)
	}
}

// The clobber the cross-process lock + re-read exist to prevent, in the shape
// the ORDERED mtime gate could not see: the other writer's save lands in the
// same filesystem timestamp tick (ext4/128-byte inodes, HFS+, NFSv3 have 1s
// granularity; FAT 2s), so `mtime.After(loadedAt)` is false and the next save
// merges onto the stale cache and renames over it.
func TestSaveFoldsInAnExternalWriteThatLandedInTheSameTimestampTick(t *testing.T) {
	p := seedConfig(t, "ui:\n  theme: everforest\n")

	c := newConfigService()
	// Our own save, which records the stamp of the file we just wrote.
	c.save(map[string]any{"claude": map[string]any{"seenModels": []any{"sonnet"}}})

	st, err := os.Stat(p)
	if err != nil {
		t.Fatal(err)
	}
	ours := st.ModTime()

	// The OTHER writer (the desktop, in its own process) changes the theme,
	// and its write lands in the same tick as ours.
	external := "ui:\n  theme: nord\n"
	if err := os.WriteFile(p, []byte(external), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(p, ours, ours); err != nil {
		t.Fatal(err)
	}

	// An UNRELATED partial from us (exactly what usageAccumulator sends).
	got := c.save(map[string]any{"claude": map[string]any{"seenModels": []any{"sonnet", "opus"}}})

	if themeOf(got) != "nord" {
		t.Errorf("save() returned theme %q: it merged onto a stale cache instead of re-reading", themeOf(got))
	}
	raw, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "theme: nord") {
		t.Fatalf("save() clobbered the other writer's setting — on-disk config no longer says theme: nord:\n%s", raw)
	}
}

// configStamp must see a same-mtime, different-length file as a change.
func TestConfigStampChangesWhenOnlyTheLengthDoes(t *testing.T) {
	p := seedConfig(t, "ui:\n  theme: everforest\n")
	before := configStamp()
	when := time.Now().Add(-time.Hour)
	if err := os.WriteFile(p, []byte("ui:\n  theme: nord\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(p, when, when); err != nil {
		t.Fatal(err)
	}
	first := configStamp()
	// Same mtime as `first`, different length.
	if err := os.WriteFile(p, []byte("ui:\n  theme: dracula\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(p, when, when); err != nil {
		t.Fatal(err)
	}
	second := configStamp()
	if first == second {
		t.Fatalf("configStamp() is blind to a same-mtime rewrite: %q == %q", first, second)
	}
	if before == "" {
		t.Fatal("configStamp() returned empty for a file that exists")
	}
}

// The same clobber in the shape no cheap stat can see AT ALL: the other writer's
// file has the same mtime AND the same length as ours. Only an unconditional
// re-read under the cross-process lock survives this.
func TestSaveFoldsInAnExternalWriteTheStampCannotSee(t *testing.T) {
	p := seedConfig(t, "ui:\n  theme: dark\n")

	c := newConfigService()
	c.save(map[string]any{"claude": map[string]any{"seenModels": []any{"sonnet"}}})

	st, err := os.Stat(p)
	if err != nil {
		t.Fatal(err)
	}
	ours, size := st.ModTime(), st.Size()

	// Pad the external write to exactly our length with a YAML comment, so
	// mtime AND size match.
	base := "ui:\n  theme: nord\n#"
	if int64(len(base)) > size {
		t.Fatalf("on-disk config (%d bytes) is shorter than the replacement (%d)", size, len(base))
	}
	external := base + strings.Repeat("#", int(size)-len(base))
	if err := os.WriteFile(p, []byte(external), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(p, ours, ours); err != nil {
		t.Fatal(err)
	}
	if got := configStamp(); got != c.loadedAt {
		t.Skipf("the stamp changed anyway (%q vs %q); this case needs an indistinguishable file", got, c.loadedAt)
	}

	got := c.save(map[string]any{"claude": map[string]any{"seenModels": []any{"sonnet", "opus"}}})
	if themeOf(got) != "nord" {
		t.Errorf("save() returned theme %q: it trusted the stamp gate instead of re-reading under the lock", themeOf(got))
	}
	raw, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "theme: nord") {
		t.Fatalf("save() clobbered an external write the stamp could not see:\n%s", raw)
	}
}
