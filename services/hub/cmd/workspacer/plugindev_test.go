package main

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

// write a file with a mtime bumped comfortably past any prior scan, so the test
// doesn't depend on the filesystem's mtime resolution.
func touch(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	future := time.Now().Add(time.Hour)
	if err := os.Chtimes(path, future, future); err != nil {
		t.Fatal(err)
	}
}

func TestDevWatchIgnore(t *testing.T) {
	ignored := []string{".git", "node_modules", ".bus-token", ".settings.json", ".install-source", ".disabled"}
	for _, n := range ignored {
		if !devWatchIgnore(n) {
			t.Errorf("devWatchIgnore(%q) = false, want true", n)
		}
	}
	for _, n := range []string{"main.go", "plugin.json", "src", "index.ts", ".gitignore"} {
		if devWatchIgnore(n) {
			t.Errorf("devWatchIgnore(%q) = true, want false", n)
		}
	}
}

func TestScanTreeDetectsRealEditsIgnoresLoaderFiles(t *testing.T) {
	dir := t.TempDir()
	touch(t, filepath.Join(dir, "plugin.json"), `{"id":"x","apiVersion":"1"}`)
	touch(t, filepath.Join(dir, "index.ts"), "console.log(1)")

	base := scanTree(dir)

	// A real source edit is a change.
	touch(t, filepath.Join(dir, "index.ts"), "console.log(2)")
	if after := scanTree(dir); !base.changed(after) {
		t.Fatal("editing a source file was not detected as a change")
	}

	// Re-baseline, then write the loader's own sidecar markers + VCS/dep noise:
	// none of these must register as a change (a reload rewrites .bus-token, so
	// watching it would loop).
	base = scanTree(dir)
	touch(t, filepath.Join(dir, ".bus-token"), "TOKEN")
	touch(t, filepath.Join(dir, ".install-source"), "owner/repo")
	touch(t, filepath.Join(dir, ".settings.json"), "{}")
	touch(t, filepath.Join(dir, ".disabled"), "")
	if err := os.MkdirAll(filepath.Join(dir, ".git"), 0o755); err != nil {
		t.Fatal(err)
	}
	touch(t, filepath.Join(dir, ".git", "HEAD"), "ref: refs/heads/main")
	if err := os.MkdirAll(filepath.Join(dir, "node_modules", "left-pad"), 0o755); err != nil {
		t.Fatal(err)
	}
	touch(t, filepath.Join(dir, "node_modules", "left-pad", "index.js"), "module.exports={}")

	if after := scanTree(dir); base.changed(after) {
		t.Fatalf("loader/VCS/dep files were treated as a change:\n base=%+v\n after=%+v", base, after)
	}

	// Adding a genuine new file under the watched tree IS a change (count bump).
	base = scanTree(dir)
	touch(t, filepath.Join(dir, "new.ts"), "export const x = 1")
	if after := scanTree(dir); !base.changed(after) {
		t.Fatal("adding a new source file was not detected as a change")
	}
}

func TestWatchLoopDebouncesAndFires(t *testing.T) {
	dir := t.TempDir()
	touch(t, filepath.Join(dir, "index.ts"), "v0")

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	fired := make(chan struct{}, 8)
	go watchLoop(ctx, dir, 10*time.Millisecond, func() { fired <- struct{}{} })

	// Give the loop a scan cycle to record the baseline, then make a change.
	time.Sleep(25 * time.Millisecond)
	touch(t, filepath.Join(dir, "index.ts"), "v1")

	select {
	case <-fired:
	case <-time.After(2 * time.Second):
		t.Fatal("watchLoop never fired onChange after a file change")
	}

	// Once the burst settled and fired, a quiet tree must NOT keep firing.
	select {
	case <-fired:
		t.Fatal("watchLoop fired again with no further changes (debounce broken)")
	case <-time.After(60 * time.Millisecond):
	}
}

func TestMakeDevPluginsDirSymlinksAndLoads(t *testing.T) {
	// A real, valid plugin dir the developer would point `plugin dev` at.
	plugDir := t.TempDir()
	touch(t, filepath.Join(plugDir, "plugin.json"), `{"id":"acme.dev","name":"Acme","apiVersion":"1"}`)

	tmp, cleanup, err := makeDevPluginsDir(plugDir)
	if err != nil {
		t.Fatal(err)
	}

	// The dev plugins dir contains exactly one entry: a symlink to plugDir.
	link := filepath.Join(tmp, filepath.Base(plugDir))
	target, err := os.Readlink(link)
	if err != nil {
		t.Fatalf("expected a symlink at %s: %v", link, err)
	}
	if target != plugDir {
		t.Fatalf("symlink points at %q, want %q", target, plugDir)
	}

	// The hub's loader (which follows the dir symlink) finds exactly this plugin.
	manifests, errs := plugin.LoadDir(tmp)
	if len(errs) != 0 {
		t.Fatalf("LoadDir errors: %v", errs)
	}
	if len(manifests) != 1 || manifests[0].ID != "acme.dev" {
		t.Fatalf("LoadDir found %+v, want exactly acme.dev", manifests)
	}

	// cleanup removes the temp dir but never the developer's real plugin dir.
	cleanup()
	if _, err := os.Stat(tmp); !os.IsNotExist(err) {
		t.Fatalf("cleanup did not remove temp dir %s (err=%v)", tmp, err)
	}
	if _, err := os.Stat(filepath.Join(plugDir, "plugin.json")); err != nil {
		t.Fatalf("cleanup damaged the developer's plugin dir: %v", err)
	}
}
