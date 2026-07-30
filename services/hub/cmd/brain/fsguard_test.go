// Path confinement for the brain's fs.* / search.project handlers
// (SECURITY.md #8).
//
// This suite exists because the confinement was documented as FIXED while being
// unreachable in the default configuration. The desktop registers these methods
// through `cat(...)`, a no-op when the catalog is delegated to this brain — which
// is the default — so the provider that actually answered the bus was this one,
// and it had no containment at all. The app-side test could not catch it: it
// mocks DELEGATE_CATALOG_TO_BRAIN = false, exercising only the kill-switch path.
//
// So: assert the deny here, against the handlers a real bus call reaches, and
// assert the filesystem is untouched after a denial — an error return that still
// wrote the file would be worse than useless.
package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// registryWithCwd returns a registry whose only live agent cwd is dir, which is
// what makes dir (and nothing above it) an allowed root.
func registryWithCwd(t *testing.T, dir string) *registry {
	t.Helper()
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1")) // never reached
	store := newSessionStore()
	store.set("s1", json.RawMessage(`{"session_id":"s1","cwd":`+jsonStr(dir)+`}`))
	reg.store = store
	return reg
}

func TestFsCallsInsideAnAgentCwdAreAllowed(t *testing.T) {
	dir := t.TempDir()
	reg := registryWithCwd(t, dir)
	p := filepath.Join(dir, "notes", "todo.txt") // parents do not exist yet

	if _, err := reg.handle(context.Background(), "fs.write",
		json.RawMessage(`{"path":`+jsonStr(p)+`,"contents":"hello"}`)); err != nil {
		t.Fatalf("write inside the agent cwd should be allowed: %v", err)
	}
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(p)+`}`)); err != nil {
		t.Fatalf("read inside the agent cwd should be allowed: %v", err)
	}
	if _, err := reg.handle(context.Background(), "fs.listEntries",
		json.RawMessage(`{"path":`+jsonStr(dir)+`}`)); err != nil {
		t.Fatalf("listEntries on the agent cwd should be allowed: %v", err)
	}
	// Missing parents are created, matching the desktop twin — plugins write to
	// <project>/.workspacer/plugins/<id>/ and depend on it. (The Go side did not,
	// so under brain delegation that write failed with ENOENT.)
	if _, err := os.Stat(p); err != nil {
		t.Fatalf("fs.write should have created missing parent dirs: %v", err)
	}
}

// The four shapes of escape, each against every path-bearing method.
func TestFsCallsOutsideTheWorkspaceAreDenied(t *testing.T) {
	dir := t.TempDir()
	outside := filepath.Join(t.TempDir(), "secret.txt")
	if err := os.WriteFile(outside, []byte("original"), 0o600); err != nil {
		t.Fatal(err)
	}

	// A symlink INSIDE the allowed cwd pointing out of it: the reason
	// containment has to resolve symlinks rather than string-prefix the input.
	link := filepath.Join(dir, "escape")
	if err := os.Symlink(filepath.Dir(outside), link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}

	cases := []struct {
		name string
		path string
	}{
		{"absolute path outside any root", outside},
		{"traversal out of the agent cwd", filepath.Join(dir, "..", "..", "etc", "passwd")},
		{"through a symlink planted inside the cwd", filepath.Join(link, "secret.txt")},
		{"a home-relative path outside the workspace", "~/.ssh/id_rsa"},
	}

	for _, tc := range cases {
		for _, method := range []string{"fs.read", "fs.listEntries"} {
			reg := registryWithCwd(t, dir)
			_, err := reg.handle(context.Background(), method,
				json.RawMessage(`{"path":`+jsonStr(tc.path)+`}`))
			if err == nil {
				t.Errorf("%s: %s should be denied", method, tc.name)
				continue
			}
			if !strings.Contains(err.Error(), "outside the allowed workspace") {
				t.Errorf("%s: %s denied for the wrong reason: %v", method, tc.name, err)
			}
		}
	}

	// fs.write gets its own assertion: the denial must also leave the target
	// alone. An error return that still wrote would be the worst outcome.
	for _, tc := range cases {
		reg := registryWithCwd(t, dir)
		if _, err := reg.handle(context.Background(), "fs.write",
			json.RawMessage(`{"path":`+jsonStr(tc.path)+`,"contents":"pwned"}`)); err == nil {
			t.Errorf("fs.write: %s should be denied", tc.name)
		}
	}
	if got, err := os.ReadFile(outside); err != nil || string(got) != "original" {
		t.Fatalf("a denied fs.write must not touch the file: contents=%q err=%v", got, err)
	}
}

func TestSearchProjectIsConfinedToTheWorkspace(t *testing.T) {
	dir := t.TempDir()
	reg := registryWithCwd(t, dir)
	_, err := reg.handle(context.Background(), "search.project",
		json.RawMessage(`{"query":"password","cwd":"/etc"}`))
	if err == nil || !strings.Contains(err.Error(), "outside the allowed workspace") {
		t.Fatalf("search.project outside the workspace should be denied, got %v", err)
	}
}

// fs.listDir is the folder picker, so it is allowed across the home tree — but
// not outside it. (Everything under $HOME is reachable by the user running the
// app anyway; /etc and other users' homes are not the picker's business.)
func TestFsListDirAllowsTheHomeTreeAndNothingElse(t *testing.T) {
	dir := t.TempDir()
	reg := registryWithCwd(t, dir)

	home, err := os.UserHomeDir()
	if err != nil {
		t.Skip("no home dir")
	}
	if _, err := reg.handle(context.Background(), "fs.listDir",
		json.RawMessage(`{"path":`+jsonStr(home)+`}`)); err != nil {
		t.Fatalf("listing the home dir should be allowed: %v", err)
	}

	reg = registryWithCwd(t, dir)
	if _, err := reg.handle(context.Background(), "fs.listDir",
		json.RawMessage(`{"path":"/etc"}`)); err == nil {
		t.Fatal("listing /etc should be denied")
	}
}

// The config dir is a root because library items, layouts and profiles live
// there and the web client edits them through fs.read/fs.write.
func TestConfigDirIsAWorkspaceRoot(t *testing.T) {
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	roots := reg.workspaceRoots(context.Background())
	if len(roots) == 0 || roots[len(roots)-1] != configDir() {
		t.Fatalf("config dir must be a workspace root; got %v", roots)
	}
}

// With no live agents and no reachable claudemon, the allow-list must collapse to
// the config dir — not open up. A shape change in claudemon's /sessions payload
// must fail closed for the same reason.
func TestNoLiveAgentsMeansNoWorkspaceRoots(t *testing.T) {
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1")) // refused
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":"/etc/passwd"}`)); err == nil {
		t.Fatal("with no live agent cwds, /etc/passwd must still be denied")
	}
}

// Drift guard: every method this brain dispatches that capspec considers
// path-bearing must actually go through the containment check. Catches the next
// fs.* handler added without one — the omission that produced this whole class of
// bug twice (fs.readImage in the app, all four fs.* here).
func TestEveryPathBearingBrainMethodIsConfined(t *testing.T) {
	src, err := os.ReadFile("handlers.go")
	if err != nil {
		t.Fatal(err)
	}
	body := string(src)
	for _, method := range []string{"fs.read", "fs.write", "fs.listEntries", "fs.listDir", "search.project"} {
		if !strings.Contains(body, `assertPathAllowed("`+method+`"`) {
			t.Errorf("%s is dispatched but never calls assertPathAllowed — it would serve arbitrary host paths", method)
		}
	}
}
