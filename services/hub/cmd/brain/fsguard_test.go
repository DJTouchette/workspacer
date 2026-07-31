// Path confinement for the brain's fs.* / search.project handlers.
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

// The config dir used to be a workspace root wholesale, and this test pinned
// that. It is the wrong shape: the same directory that holds library/, layouts/
// and sessions/ (the stores a client legitimately edits) also holds remote-token
// — the host bus credential. Reading it promotes the caller to a TRUSTED
// connection, and a trusted connection may call /plugins/install, which runs a
// command. So a plugin that declared any fs.read path resolving into the config
// dir had a two-step route from "read a file" to "execute anything".
//
// Now: only the three store subtrees are roots, and everything else in the
// config dir is refused by pathIsSecret even when another root re-admits it.
func TestConfigStoresAreTheOnlyConfigDirRoots(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)

	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	roots := reg.workspaceRoots(context.Background())
	for _, store := range []string{"library", "layouts", "sessions"} {
		want := filepath.Join(configDir(), store)
		found := false
		for _, r := range roots {
			if r == want {
				found = true
			}
		}
		if !found {
			t.Errorf("%s must stay a workspace root (the UI edits it through fs.*); got %v", want, roots)
		}
	}
	for _, r := range roots {
		if r == configDir() {
			t.Fatalf("the whole config dir must NOT be a workspace root; got %v", roots)
		}
	}
}

// The deny-list half, and the reason it is a SEPARATE test from the roots half:
// with no live agents the config dir is already outside every root, so denials
// there prove nothing about pathIsSecret — stub the second gate out and the
// assertions still pass. That is the failure mode this whole suite exists for —
// a guard the test never reaches — so this case gives the registry a live agent cwd one
// level ABOVE the config dir — the "user spawned an agent in $HOME" case — which
// makes the roots check say yes to everything below and leaves pathIsSecret as
// the only thing that can refuse.
func TestConfigDirIsRefusedEvenWhenAnAgentCwdReadmitsIt(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	// The agent cwd is the parent of <configDir>, so <configDir>/... is inside a
	// legitimate root and the roots check cannot be what refuses below.
	reg := registryWithCwd(t, dir)

	// Not just the credentials: config.yaml is here because updates.channel is
	// string-concatenated into the electron-updater feed URL, so a write to it
	// walks around config.save's host-trusted gate on updates.* and relocates the
	// updater; workspacer.db and the legacy plugin-settings.json overlay hold
	// session history and pre-migration plaintext plugin secrets.
	for _, name := range []string{
		"remote-token", "tokens.json", "remote-server.json", "vapid.json",
		"config.yaml", "claude-profiles.json", "plugin-settings.json", "workspacer.db",
	} {
		p := filepath.Join(configDir(), name)
		if err := os.WriteFile(p, []byte("s3cret"), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := reg.handle(context.Background(), "fs.read",
			json.RawMessage(`{"path":`+jsonStr(p)+`}`)); err == nil {
			t.Errorf("fs.read of %s must be denied even with the config dir inside an agent cwd", name)
		}
		if _, err := reg.handle(context.Background(), "fs.write",
			json.RawMessage(`{"path":`+jsonStr(p)+`,"contents":"pwned"}`)); err == nil {
			t.Errorf("fs.write of %s must be denied even with the config dir inside an agent cwd", name)
		}
		if got, err := os.ReadFile(p); err != nil || string(got) != "s3cret" {
			t.Fatalf("a denied write touched %s: %q (%v)", name, got, err)
		}
	}

	// A file the rule invents is as bad as one it misses: the three stores stay
	// writable through the same wide root.
	for _, store := range []string{"library", "layouts", "sessions"} {
		p := filepath.Join(configDir(), store, "item.yaml")
		if _, err := reg.handle(context.Background(), "fs.write",
			json.RawMessage(`{"path":`+jsonStr(p)+`,"contents":"ok"}`)); err != nil {
			t.Errorf("fs.write into %s/ must stay allowed: %v", store, err)
		}
	}
}

// A plugin's own credentials are denied by BASENAME, wherever they resolve: the
// roots can only be as narrow as the cwds agents run in, and `workspacer plugin
// dev` drops a .bus-token into whatever directory it is pointed at — including
// one inside a project another agent is working in.
func TestPluginCredentialsAreDeniedInsideAnAgentCwd(t *testing.T) {
	dir := t.TempDir()
	reg := registryWithCwd(t, dir)

	for _, name := range []string{".bus-token", ".settings.json"} {
		p := filepath.Join(dir, "plugin", name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte("s3cret"), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := reg.handle(context.Background(), "fs.read",
			json.RawMessage(`{"path":`+jsonStr(p)+`}`)); err == nil {
			t.Errorf("fs.read of %s must be denied even inside an agent cwd", name)
		}
	}
}

// With no live agents and no reachable claudemon, the allow-list must collapse to
// the config stores — not open up. A shape change in claudemon's /sessions payload
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
