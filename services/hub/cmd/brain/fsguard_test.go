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
	"runtime"
	"sort"
	"strconv"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// refusalText is the single, non-echoing denial (spec 7.5). Tests match on it
// rather than on "an error happened", because a handler that fails for an
// unrelated reason (ENOENT, a missing param) proves nothing about confinement.
const refusalText = "path is outside the allowed workspace (agent cwds + config stores)"

// registryWithCwd returns a registry whose only live agent cwd is dir, which is
// what makes dir (and nothing above it) an allowed root.
func registryWithCwd(t *testing.T, dir string) *registry {
	t.Helper()
	return registryWithCwds(t, dir)
}

// registryWithCwds is the same thing for N live agents, which is what the
// fixture's multi-root cases ("the second root in the list matches", "an empty
// root among valid ones") need: each root becomes one session's cwd, so the
// allow-list the handler assembles is the case's root list plus the config
// stores. Zero cwds is a real state too (no live agents), and the store must be
// non-nil for it or agentCwds falls back to the claudemon client.
func registryWithCwds(t *testing.T, dirs ...string) *registry {
	t.Helper()
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1")) // never reached
	store := newSessionStore()
	for i, dir := range dirs {
		id := "s" + strconv.Itoa(i)
		store.set(id, json.RawMessage(`{"session_id":`+jsonStr(id)+`,"cwd":`+jsonStr(dir)+`}`))
	}
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

// ---------------------------------------------------------------------------
// contracts/path-containment-cases.json — the cross-language corpus.
//
// Everything above this line is hand-written and stays that way: it pins brain
// SHAPES (missing parents are created, a denied write leaves the file alone, the
// config stores are roots but the config dir is not) that the corpus does not
// model. Everything below is the corpus itself — the predicate, case by case,
// and then the second half of the contract: that every path-bearing method the
// brain answers actually calls the predicate.
// ---------------------------------------------------------------------------

// contractFixtureRel is relative to this package dir (services/hub/cmd/brain).
const contractFixtureRel = "../../../../contracts/path-containment-cases.json"

// fsguardOwnerKey is this implementation's key in the fixture's `owners` map;
// the groups listed there are the ones this copy must satisfy.
const fsguardOwnerKey = "services/hub/cmd/brain/fsguard.go"

type contractTree struct {
	Dirs     []string          `json:"dirs"`
	Files    map[string]string `json:"files"`
	Symlinks map[string]string `json:"symlinks"`
	Modes    map[string]string `json:"modes"`
}

type contractCase struct {
	Name               string       `json:"name"`
	Group              string       `json:"group"`
	NeedsSymlinks      bool         `json:"needsSymlinks"`
	PosixOnly          bool         `json:"posixOnly"`
	NeedsUnreadableDir bool         `json:"needsUnreadableDir"`
	Tree               contractTree `json:"tree"`
	Roots              []string     `json:"roots"`
	Target             string       `json:"target"`
	Expect             string       `json:"expect"`
	Why                string       `json:"why"`
}

type contractMethod struct {
	Method    string         `json:"method"`
	Field     string         `json:"field"`
	Params    map[string]any `json:"params"`
	RootSet   string         `json:"rootSet"`
	Providers []string       `json:"providers"`
}

func (m contractMethod) providedByBrain() bool {
	for _, p := range m.Providers {
		if p == "brain" {
			return true
		}
	}
	return false
}

type contractFixture struct {
	Owners  map[string][]string `json:"owners"`
	Cases   []contractCase      `json:"cases"`
	Methods []contractMethod    `json:"methods"`
}

// ownedGroups is the set of case groups fsguard.go is on the hook for. A copy
// that quietly dropped itself out of `owners` would otherwise run zero cases and
// report success, so the loader fails loudly instead.
func (fx contractFixture) ownedGroups(t *testing.T) map[string]bool {
	t.Helper()
	groups := map[string]bool{}
	for _, g := range fx.Owners[fsguardOwnerKey] {
		groups[g] = true
	}
	if !groups["containment"] || !groups["secrets"] {
		t.Fatalf("%s must own both `containment` and `secrets` in the fixture's owners map; got %v",
			fsguardOwnerKey, fx.Owners[fsguardOwnerKey])
	}
	return groups
}

func loadContractFixture(t *testing.T) contractFixture {
	t.Helper()
	raw, err := os.ReadFile(contractFixtureRel)
	if err != nil {
		t.Fatalf("read the shared fixture: %v", err)
	}
	var fx contractFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	if len(fx.Cases) == 0 || len(fx.Methods) == 0 {
		t.Fatalf("%s decoded to %d cases and %d methods — a silently empty corpus guards nothing",
			contractFixtureRel, len(fx.Cases), len(fx.Methods))
	}
	return fx
}

// caseSandbox builds one case's world per the fixture's sandbox model: a fresh
// temp dir per case, REALPATH'd (macOS hands out /var/... which is a symlink to
// /private/var — without this every case would pass or fail for the wrong
// reason), with root/ outside/ config/workspacer/ pre-created, then `tree`
// materialized in the mandated order (dirs, files, symlinks, modes) and the
// implementation's config dir pointed at ${SANDBOX}/config.
//
// It returns the sandbox and the token substituter. Substitution applies to
// `roots` and `target` and to nothing else — a root written without a token
// ("/", "", "~", "root") is passed through literally on purpose.
func caseSandbox(t *testing.T, c contractCase) (string, func(string) string) {
	t.Helper()
	if c.PosixOnly && runtime.GOOS == "windows" {
		t.Skip("posixOnly: the '/' filesystem-root branch has no portable spelling on Windows")
	}
	if c.NeedsUnreadableDir && (runtime.GOOS == "windows" || os.Geteuid() == 0) {
		t.Skip("needsUnreadableDir: this process can read a 0o000 directory anyway")
	}

	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatalf("realpath the sandbox: %v", err)
	}
	configHome := filepath.Join(sandbox, "config")
	for _, d := range []string{"root", "outside", filepath.Join("config", "workspacer")} {
		if err := os.MkdirAll(filepath.Join(sandbox, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	// Read at call time by configDir(), so this redirects the secret gate's
	// config dir to ${CONFIG} = ${SANDBOX}/config/workspacer.
	t.Setenv("XDG_CONFIG_HOME", configHome)
	t.Setenv("APPDATA", configHome)

	sub := func(s string) string {
		s = strings.ReplaceAll(s, "${ROOT}", filepath.Join(sandbox, "root"))
		s = strings.ReplaceAll(s, "${OUTSIDE}", filepath.Join(sandbox, "outside"))
		s = strings.ReplaceAll(s, "${CONFIG}", filepath.Join(configHome, "workspacer"))
		return strings.ReplaceAll(s, "${SANDBOX}", sandbox)
	}

	for _, d := range c.Tree.Dirs {
		if err := os.MkdirAll(filepath.Join(sandbox, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	for rel, body := range c.Tree.Files {
		full := filepath.Join(sandbox, rel)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	for rel, dest := range c.Tree.Symlinks {
		full := filepath.Join(sandbox, rel)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		// Absolute by construction: no case depends on relative-link resolution.
		if err := os.Symlink(filepath.Join(sandbox, dest), full); err != nil {
			if c.NeedsSymlinks {
				t.Skipf("needsSymlinks: cannot create symlinks here: %v", err)
			}
			t.Fatal(err)
		}
	}
	// Modes LAST, so a 0-mode directory does not block the rest of the setup,
	// and restored before teardown or t.TempDir's cleanup cannot remove it.
	for rel, mode := range c.Tree.Modes {
		full := filepath.Join(sandbox, rel)
		bits, err := strconv.ParseUint(strings.TrimPrefix(strings.TrimPrefix(mode, "0o"), "0"), 8, 32)
		if err != nil && mode != "0" {
			t.Fatalf("unparseable mode %q on %s: %v", mode, rel, err)
		}
		t.Cleanup(func() { _ = os.Chmod(full, 0o700) })
		if err := os.Chmod(full, os.FileMode(bits)); err != nil {
			t.Fatal(err)
		}
	}
	return sandbox, sub
}

func substituted(sub func(string) string, in []string) []string {
	out := make([]string, 0, len(in))
	for _, s := range in {
		out = append(out, sub(s))
	}
	return out
}

// TestPathContainmentContractCases is the predicate half: assertPathAllowed
// itself, against every case fsguard.go owns. This is the copy that ACTUALLY
// answers fs.*/library.* under the default DELEGATE_CATALOG_TO_BRAIN, and until
// this fixture existed nothing kept it in agreement with the desktop's copy or
// the bus's — all three hand-rolled the same walk and disagreed about tilde
// expansion, symlink-plus-"..", and whether "/" contains anything.
func TestPathContainmentContractCases(t *testing.T) {
	fx := loadContractFixture(t)
	groups := fx.ownedGroups(t)

	owned := 0
	for _, c := range fx.Cases {
		if groups[c.Group] {
			owned++
		}
	}
	if owned == 0 {
		t.Fatal("no owned cases: a corpus this implementation is not on the hook for guards nothing")
	}
	t.Logf("%d owned cases of %d", owned, len(fx.Cases))

	for _, c := range fx.Cases {
		if !groups[c.Group] {
			continue
		}
		t.Run(c.Name, func(t *testing.T) {
			_, sub := caseSandbox(t, c)
			roots := substituted(sub, c.Roots)
			target := sub(c.Target)

			canonical, err := assertPathAllowed("contract", target, roots)
			allowed := err == nil
			want := c.Expect == "allow"
			if allowed != want {
				t.Fatalf("expected %s, got allowed=%v (err=%v)\n  target: %q\n  roots:  %q\n  why:    %s",
					c.Expect, allowed, err, target, roots, c.Why)
			}
			if !allowed {
				// 7.5: one message for all three refusal reasons, echoing
				// neither the target, nor where it resolved, nor which gate
				// fired. Anything else is a probe primitive for a remote caller.
				if got, want := err.Error(), "contract: "+refusalText; got != want {
					t.Fatalf("refusal message drifted\n  got:  %q\n  want: %q", got, want)
				}
				return
			}
			// 7.4/8.1: what comes back is what the handler must open.
			if !filepath.IsAbs(canonical) {
				t.Fatalf("allowed but the canonical path is not absolute: %q", canonical)
			}
			for _, comp := range strings.Split(canonical, string(filepath.Separator)) {
				if comp == ".." || comp == "." {
					t.Fatalf("canonical path still carries a %q component: %q", comp, canonical)
				}
			}
		})
	}
}

// TestCorpusMethodsMatchCapspec pins the two halves of the method contract onto
// each other: capspec.PathParam is what the BUS confines, the corpus's `methods`
// block is what the PROVIDERS are tested against, and a method in one and not
// the other is exactly the gap this whole file exists to close. A new fs.*
// capability added to capspec now fails here until someone writes its corpus
// entry — and a corpus entry naming a method the bus does not consider
// path-scoped fails here too, because that method would be granted to plugins
// with no confinement at all.
func TestCorpusMethodsMatchCapspec(t *testing.T) {
	fx := loadContractFixture(t)
	byMethod := map[string]contractMethod{}
	for _, m := range fx.Methods {
		if _, dup := byMethod[m.Method]; dup {
			t.Fatalf("the corpus lists %s twice", m.Method)
		}
		byMethod[m.Method] = m
		field, ok := capspec.IsPathScoped(m.Method)
		if !ok {
			t.Errorf("the corpus models %s as path-bearing but capspec.PathParam has no entry — the bus would grant it unconfined", m.Method)
			continue
		}
		if field != m.Field {
			t.Errorf("%s: the corpus injects into %q but capspec confines %q — the bus would guard a different field than the provider", m.Method, m.Field, field)
		}
	}
	for _, method := range sortedMethods(capspec.PathParam) {
		if _, ok := byMethod[method]; !ok {
			t.Errorf("capspec.PathParam has %s with no corpus entry — nothing asserts that its provider actually calls the guard", method)
		}
	}
}

func sortedMethods(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// TestEveryPathBearingBrainMethodIsConfined is the drift guard, and it is
// BEHAVIOURAL. It used to grep handlers.go for the string `assertPathAllowed("`
// followed by a hardcoded list of five method names, which is two lies in one:
// the list was maintained by hand (library.* was path-bearing for months and not
// on it), and a textual match proves only that the call was WRITTEN, not that it
// runs — a guard behind an `if cwd != ""`, or one whose result is discarded,
// passes a grep and confines nothing.
//
// Now: iterate capspec.PathParam — the bus's own definition of "this method
// touches the filesystem" — take every entry the brain actually dispatches, and
// drive the real handler with every deny case in the corpus. A new fs.* handler
// cannot ship unguarded, because it must be in capspec (TestBrainMethodsAllScoped
// forces that), which puts it in the corpus (TestCorpusMethodsMatchCapspec
// forces that), which lands it here.
func TestEveryPathBearingBrainMethodIsConfined(t *testing.T) {
	fx := loadContractFixture(t)
	groups := fx.ownedGroups(t)

	byMethod := map[string]contractMethod{}
	for _, m := range fx.Methods {
		byMethod[m.Method] = m
	}

	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	dispatched := map[string]bool{}
	for _, set := range [][]string{reg.methods(), reg.catalogMethods()} {
		for _, m := range set {
			dispatched[m] = true
		}
	}

	exercised := 0
	for _, method := range sortedMethods(capspec.PathParam) {
		m, ok := byMethod[method]
		if !ok {
			continue // reported by TestCorpusMethodsMatchCapspec
		}
		if !dispatched[method] {
			if m.providedByBrain() {
				t.Errorf("the corpus says the brain provides %s, but neither methods() nor catalogMethods() dispatches it — one of the two is stale", method)
			}
			continue
		}
		if !m.providedByBrain() {
			t.Errorf("the brain dispatches %s but the corpus does not list it as a brain provider, so nothing would have tested this side of it", method)
		}
		exercised++
		t.Run(method, func(t *testing.T) { assertMethodRejectsCorpus(t, fx, groups, m) })
	}
	if exercised == 0 {
		t.Fatal("no path-bearing brain method was exercised — capspec, the corpus and the registry have drifted out of overlap and this guard is guarding nothing")
	}
	t.Logf("%d of capspec's %d path-bearing methods are answered by this brain", exercised, len(capspec.PathParam))
}

// assertMethodRejectsCorpus drives one real handler with every deny case the
// brain owns, using the case's roots as the live agent cwds. The handler must
// refuse with the containment message and not with, say, a decode error that
// would mask a missing guard.
func assertMethodRejectsCorpus(t *testing.T, fx contractFixture, groups map[string]bool, m contractMethod) {
	t.Helper()
	for _, c := range fx.Cases {
		if !groups[c.Group] || c.Expect != "deny" {
			continue
		}
		// library.save guards the DERIVED destination (<cwd>/.workspacer/library/
		// <id>.md), not the raw cwd, so only the containment group transfers: a
		// cwd outside the roots always yields a destination outside them, but a
		// cwd that IS a credential file yields a destination whose basename is
		// <id>.md and whose location can be a store carve-out — the secret gate
		// is answering about a path the caller never named. The corpus's own note
		// scopes the equivalence to the roots ("either is confined by the same
		// roots"). The config-dir denials for save are covered by
		// TestConfigDirIsRefusedEvenWhenAnAgentCwdReadmitsIt.
		if derivesDestination(m) && c.Group != "containment" {
			continue
		}
		t.Run(c.Name, func(t *testing.T) {
			sandbox, sub := caseSandbox(t, c)
			target := sub(c.Target)
			if why := blankTargetIsAnsweredBeforeTheGuard(m, target); why != "" {
				t.Skip(why)
			}
			// A browse-rootSet method also allows the whole home tree, so a
			// sandbox that happens to live under $HOME (TMPDIR=~/tmp) would make
			// a deny case allow for a legitimate reason.
			if m.RootSet == "browse" {
				if home, err := os.UserHomeDir(); err == nil && home != "" &&
					(sandbox == home || strings.HasPrefix(sandbox, home+string(filepath.Separator))) {
					t.Skipf("sandbox %s is inside $HOME, which %s legitimately browses", sandbox, m.Method)
				}
			}

			roots := substituted(sub, c.Roots)
			reg := registryWithCwds(t, roots...)
			params := map[string]any{}
			for k, v := range m.Params {
				params[k] = v
			}
			params[m.Field] = target
			body, err := json.Marshal(params)
			if err != nil {
				t.Fatal(err)
			}
			res, err := reg.handle(context.Background(), m.Method, json.RawMessage(body))

			if derivationCleansTheEscape(m, c) {
				// library.save composes its destination with filepath.Join,
				// which Cleans — so for a cwd that escapes ONLY by following a
				// symlink and then popping "..", the escape is collapsed away
				// textually before the guard ever runs, and the destination is
				// pulled back inside the root. Refusing is fine and accepting is
				// fine; writing outside is not. Since the check and the use are
				// the same collapsed string, the question the corpus is really
				// asking here is "where did the bytes land", so ask that
				// directly — with EvalSymlinks, an oracle that shares no code
				// with the thing under test.
				if err != nil {
					return
				}
				assertWroteInsideARoot(t, res, roots)
				return
			}

			if err == nil {
				t.Fatalf("%s with %s=%q must be refused\n  roots: %q\n  why:   %s",
					m.Method, m.Field, target, roots, c.Why)
			}
			// A blank field is refused by the handler's own "requires a path"
			// before the guard is reached. That is a denial and it echoes
			// nothing, so it satisfies the case; requiring the guard's uniform
			// wording there would be asserting the params gate, which the corpus
			// declares bus-only. Everything the GUARD refuses must use the one
			// message — see divergences: this is the brain's one non-uniform
			// denial class.
			if strings.TrimSpace(target) == "" {
				return
			}
			if !strings.Contains(err.Error(), refusalText) {
				t.Fatalf("%s with %s=%q was rejected for the wrong reason: %v\n  why: %s",
					m.Method, m.Field, target, err, c.Why)
			}
		})
	}

	// The floor. Every assertion above is a denial, and a handler that refuses
	// unconditionally satisfies all of them — so one legitimate in-root call must
	// NOT produce the containment refusal. (It may still fail for its own
	// reasons: fs.read of a directory, library.remove of an absent item. Those
	// are not this test's business.)
	t.Run("a path inside a live agent cwd is not refused", func(t *testing.T) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
		dir, err := filepath.EvalSymlinks(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		reg := registryWithCwds(t, dir)
		params := map[string]any{}
		for k, v := range m.Params {
			params[k] = v
		}
		params[m.Field] = dir
		body, _ := json.Marshal(params)
		if _, err := reg.handle(context.Background(), m.Method, json.RawMessage(body)); err != nil &&
			strings.Contains(err.Error(), refusalText) {
			t.Fatalf("%s refused its own agent cwd — the guard denies everything, which passes every deny case above: %v", m.Method, err)
		}
	})
}

// blankTargetIsAnsweredBeforeTheGuard returns the reason to skip a blank-target
// case for one method, or "" to run it. The corpus's empty/whitespace targets
// live in the `containment` group but the METHOD-level version of them is the
// params gate, which the corpus declares bus-only — and three brain methods
// substitute a default for a blank field before the guard is reached, on
// purpose. Every other method gets the case: fs.read/fs.write/fs.listEntries
// reject "" with their own "requires a path" and refuse "   " through the guard,
// and library.save composes from mustCwd() and is refused because the daemon's
// own working directory is not a root. Naming the three exceptions one by one
// (rather than skipping every blank target everywhere) keeps the other five
// methods actually covered.
func blankTargetIsAnsweredBeforeTheGuard(m contractMethod, target string) string {
	switch m.Method {
	case "fs.listDir":
		// The picker opens on $HOME when it has nowhere to start from, and that
		// default sits deliberately BEFORE the guard: "" is otherwise
		// unverifiable (it would absolutize to the daemon's own cwd), so the
		// substitution has to happen while there is still a decision to make.
		if strings.TrimSpace(target) == "" {
			return "fs.listDir substitutes $HOME for a blank path before the guard runs, by design"
		}
	case "library.list", "library.remove":
		// An absent cwd means the GLOBAL store, not a project one. The corpus
		// records library.* with no cwd at all as DELIBERATELY ABSENT because Go
		// resolves it against its own process cwd and TypeScript skips the guard
		// entirely — pinning either here would bless one side of a live
		// divergence. (Whitespace is NOT exempt: "   " is a non-empty cwd and
		// must be refused, and this case asserts that.)
		if target == "" {
			return "an absent cwd means the global store; the corpus leaves library.* with no cwd deliberately unpinned"
		}
	}
	return ""
}

// derivesDestination names the methods whose injected field is not the path
// they open: library.save takes a cwd and composes <cwd>/.workspacer/library/
// <id>.md (or the .claude/ equivalent) from it, and guards THAT. The corpus's
// verdict is about the field; for these it transfers to the composition, which
// is exact for containment and not for the secret gate.
func derivesDestination(m contractMethod) bool { return m.Method == "library.save" }

// derivationCleansTheEscape marks the cases where composing the destination with
// filepath.Join destroys the very thing the case is testing. Join Cleans, and
// Clean collapses "link/.." textually — so a target that escapes only by
// following a symlink and then popping is folded back inside the root before the
// guard sees it. The condition is deliberately textual (a symlink case whose
// target carries a "..") rather than "ask the guard whether it still escapes":
// a broken guard must not be able to talk this test out of running.
func derivationCleansTheEscape(m contractMethod, c contractCase) bool {
	return derivesDestination(m) && c.NeedsSymlinks && strings.Contains(c.Target, "..")
}

// assertWroteInsideARoot checks the landing site of a write with EvalSymlinks
// rather than with fsguard's own walk, so the assertion cannot be satisfied by
// the same bug it is looking for. `res` is a library item; its `path` is the
// file the handler says it wrote.
func assertWroteInsideARoot(t *testing.T, res json.RawMessage, roots []string) {
	t.Helper()
	var item struct {
		Path string `json:"path"`
	}
	if err := json.Unmarshal(res, &item); err != nil || item.Path == "" {
		t.Fatalf("library.save reported no path (err=%v, body=%s)", err, res)
	}
	if _, err := os.Lstat(item.Path); err != nil {
		t.Fatalf("library.save reported %s but nothing is there: %v", item.Path, err)
	}
	real, err := filepath.EvalSymlinks(item.Path)
	if err != nil {
		t.Fatalf("realpath %s: %v", item.Path, err)
	}
	for _, root := range roots {
		rr, err := filepath.EvalSymlinks(root)
		if err != nil {
			continue
		}
		if real == rr || strings.HasPrefix(real, strings.TrimSuffix(rr, string(filepath.Separator))+string(filepath.Separator)) {
			return
		}
	}
	t.Fatalf("library.save accepted a cwd whose escape its own filepath.Join collapsed, and then wrote OUTSIDE every root: %s (roots %q)", real, roots)
}

// TestPathBearingMethodRootSetsMatchTheCorpus pins the OTHER half of the guard:
// not "is there a check" but "which allow-list does it consult". The two are
// separate bugs — library.list called assertPathAllowed all along, with the
// workspace roots, so it passed every containment case and still broke the New
// Agent dialog's project-MCP picker for every directory no agent was running in
// (the caller's `.catch(() => {})` turned the refusal into an empty list).
//
// browse = workspace roots + $HOME. So: a directory under $HOME that is not any
// agent's cwd must be accepted by a browse method and refused by a workspace one.
func TestPathBearingMethodRootSetsMatchTheCorpus(t *testing.T) {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		t.Skip("no home dir: browse and workspace roots are identical here")
	}
	fx := loadContractFixture(t)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	dispatched := map[string]bool{}
	for _, set := range [][]string{reg.methods(), reg.catalogMethods()} {
		for _, m := range set {
			dispatched[m] = true
		}
	}

	for _, m := range fx.Methods {
		if !m.providedByBrain() || !dispatched[m.Method] {
			continue
		}
		t.Run(m.Method, func(t *testing.T) {
			// A config dir of its own, so the config stores cannot be what
			// admits (or refuses) the probe.
			t.Setenv("XDG_CONFIG_HOME", t.TempDir())
			cwd, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			// Under $HOME, does not exist, and is nobody's cwd: inside the
			// browse roots and outside the workspace roots, by construction.
			probe := filepath.Join(home, "wks-contract-probe-not-an-agent-cwd")
			if _, err := os.Lstat(probe); err == nil {
				t.Skipf("%s exists on this machine; the probe must be a path nothing owns", probe)
			}
			reg := registryWithCwds(t, cwd)
			params := map[string]any{}
			for k, v := range m.Params {
				params[k] = v
			}
			params[m.Field] = probe
			body, _ := json.Marshal(params)
			_, err = reg.handle(context.Background(), m.Method, json.RawMessage(body))
			refused := err != nil && strings.Contains(err.Error(), refusalText)

			switch m.RootSet {
			case "browse":
				if refused {
					t.Fatalf("the corpus says %s browses (%s), but it refused a $HOME path that is not a live agent cwd: %v", m.Method, m.RootSet, err)
				}
			case "workspace":
				if !refused {
					t.Fatalf("the corpus confines %s to the workspace roots, but it accepted a $HOME path no agent is running in (err=%v)", m.Method, err)
				}
			default:
				t.Fatalf("unknown rootSet %q in the corpus for %s", m.RootSet, m.Method)
			}
		})
	}
}
