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
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// hostSkipReason names the host requirement that made caseSandbox skip a case,
// so the sweep floors below can say WHICH privilege turned the corpus off
// rather than just reporting a zero. Order matches caseSandbox's own gates.
func hostSkipReason(c contractCase) string {
	switch {
	case c.PosixOnly && runtime.GOOS == "windows":
		return "posixOnly"
	case c.NeedsUnreadableDir:
		return "needsUnreadableDir"
	case c.NeedsHome:
		return "needsHome"
	case c.ConfigDirVia != "":
		return "configDirVia (needs symlinks)"
	case c.NeedsSymlinks:
		return "needsSymlinks"
	}
	return "unexplained (the case declares no host requirement — a skip here is a bug in the loader, not a host limitation)"
}

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

// library.list and library.remove confine the caller's `cwd`, and for a long
// time that was ALL they confined. Everything they actually touch is DERIVED
// from that cwd — <cwd>/.workspacer/library/<name>.md, <cwd>/.claude/skills/
// <id>/SKILL.md — and those derived paths went straight to os.ReadFile /
// os.RemoveAll without ever being canonicalized. One symlink inside the allowed
// root (writing it is an ordinary permitted fs.write) therefore did what fs.read
// of the identical symlink refuses to do.
//
// libraryCwdWithConfigDir sets up that world once: a sandbox holding a config
// dir with a live remote-token, and an agent cwd BESIDE it (not above it, so the
// roots check is not what refuses — the derived-path guard has to be).
func libraryCwdWithConfigDir(t *testing.T) (cwd, token string) {
	t.Helper()
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	token = filepath.Join(configDir(), "remote-token")
	if err := os.WriteFile(token, []byte("SUPERSECRET-REMOTE-TOKEN"), 0o600); err != nil {
		t.Fatal(err)
	}
	cwd = filepath.Join(sandbox, "project")
	if err := os.MkdirAll(cwd, 0o755); err != nil {
		t.Fatal(err)
	}
	return cwd, token
}

func TestLibraryListDoesNotReadThroughASymlinkOutOfTheRoots(t *testing.T) {
	cwd, token := libraryCwdWithConfigDir(t)

	// Two plants, because list() reaches the filesystem through two different
	// walkers: the .md sweep of the project library dir, and the per-skill
	// SKILL.md read under .claude/skills.
	projLib := filepath.Join(cwd, ".workspacer", "library")
	if err := os.MkdirAll(projLib, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(token, filepath.Join(projLib, "pwn.md")); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	skill := filepath.Join(cwd, ".claude", "skills", "x")
	if err := os.MkdirAll(skill, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(token, filepath.Join(skill, "SKILL.md")); err != nil {
		t.Fatal(err)
	}

	// The control: fs.read of the very same symlink is refused. If that ever
	// stops being true the leak below is not the finding this test describes.
	reg := registryWithCwd(t, cwd)
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(filepath.Join(projLib, "pwn.md"))+`}`)); err == nil {
		t.Fatal("fs.read of the planted symlink must be denied (the control for this test)")
	}

	reg = registryWithCwd(t, cwd)
	res, err := reg.handle(context.Background(), "library.list",
		json.RawMessage(`{"cwd":`+jsonStr(cwd)+`}`))
	if err != nil {
		t.Fatalf("library.list of a legitimate cwd must still succeed: %v", err)
	}
	if strings.Contains(string(res), "SUPERSECRET-REMOTE-TOKEN") {
		t.Fatalf("library.list returned the bus credential through a symlink planted in an allowed root: %s", res)
	}

	// The floor: a REAL item in the same directory is still listed, so the fix
	// is a guard and not "library.list stopped reading files".
	if err := os.WriteFile(filepath.Join(projLib, "ok.md"), []byte("---\ntitle: Fine\n---\n\nbody\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	reg = registryWithCwd(t, cwd)
	res, err = reg.handle(context.Background(), "library.list",
		json.RawMessage(`{"cwd":`+jsonStr(cwd)+`}`))
	if err != nil || !strings.Contains(string(res), "Fine") {
		t.Fatalf("an ordinary project library item must still be listed: %s (%v)", res, err)
	}
}

func TestLibraryRemoveDoesNotDeleteOutsideTheRootsThroughASymlink(t *testing.T) {
	cwd, token := libraryCwdWithConfigDir(t)

	// A DIRECTORY symlink: .claude/skills -> <configDir>. removeLibrary then
	// composes <cwd>/.claude/skills/remote-token and RemoveAll's it.
	if err := os.MkdirAll(filepath.Join(cwd, ".claude"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(configDir(), filepath.Join(cwd, ".claude", "skills")); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}

	reg := registryWithCwd(t, cwd)
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"claude","kind":"skill","id":"remote-token","cwd":`+jsonStr(cwd)+`}`)); err != nil {
		t.Fatalf("library.remove reported an error rather than silently skipping: %v", err)
	}
	if _, err := os.Stat(token); err != nil {
		t.Fatalf("library.remove destroyed the bus credential outside every allowed root: %v", err)
	}

	// The floor again: a real skill inside the project is still removable.
	realCwd := t.TempDir()
	realSkill := filepath.Join(realCwd, ".claude", "skills", "keeper")
	if err := os.MkdirAll(realSkill, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(realSkill, "SKILL.md"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	reg = registryWithCwd(t, realCwd)
	if _, err := reg.handle(context.Background(), "library.remove",
		json.RawMessage(`{"scope":"claude","kind":"skill","id":"keeper","cwd":`+jsonStr(realCwd)+`}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(realSkill); err == nil {
		t.Fatal("library.remove must still delete a skill that really is inside the agent cwd")
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
	// RelativeSymlinks values are written VERBATIM as the link body, which is
	// the only way to reach the walk's relative-link arm. `Symlinks` values are
	// sandbox-relative and absolutized by the loader.
	RelativeSymlinks map[string]string `json:"relativeSymlinks"`
	Modes            map[string]string `json:"modes"`
}

type contractCase struct {
	Name               string       `json:"name"`
	Group              string       `json:"group"`
	NeedsSymlinks      bool         `json:"needsSymlinks"`
	PosixOnly          bool         `json:"posixOnly"`
	NeedsUnreadableDir bool         `json:"needsUnreadableDir"`
	NeedsHome          bool         `json:"needsHome"`
	Tree               contractTree `json:"tree"`
	// ConfigDirVia names a sandbox-relative symlink to the config HOME that the
	// implementation's config dir is pointed through, while ${CONFIG} keeps
	// naming the real path.
	ConfigDirVia string   `json:"configDirVia"`
	Roots        []string `json:"roots"`
	Target       string   `json:"target"`
	Expect       string   `json:"expect"`
	// DeniedBy is the RIGHT-REASON half of a deny, named from the fixture's
	// `vocabulary.denyReasons`. `expect: deny` on its own is satisfied by a
	// refusal for ANY reason — including "the token did not substitute, so the
	// target was a relative literal and every copy refused it for that" — so a
	// deny case says which of the four outcomes it is exercising and
	// contractDenyReason has to land on it. Mandatory on a deny, forbidden on
	// an allow (which carries ResolvesTo instead).
	DeniedBy string `json:"deniedBy"`
	// ResolvesTo is the token-substituted path assertPathAllowed must RETURN on
	// an allow — the string BINDING DECISION 2 then hands to the filesystem.
	// Mandatory on every allow case; a deny returns no path.
	ResolvesTo string `json:"resolvesTo"`
	Why        string `json:"why"`
}

type contractMethod struct {
	Method  string         `json:"method"`
	Field   string         `json:"field"`
	Params  map[string]any `json:"params"`
	RootSet string         `json:"rootSet"`
	// DerivedRootSet names the SECOND, narrower allow-list a method's derived
	// paths are confined to when it has one — library.* compose
	// <cwd>/.workspacer/library/<slug>.md out of the cwd they were given, and
	// "item" means [<configDir>/library, cwd]. Empty for methods that open the
	// field they were handed. See TestLibraryDerivedRootSetIsTheItemRoots.
	DerivedRootSet string   `json:"derivedRootSet"`
	Providers      []string `json:"providers"`
}

func (m contractMethod) providedByBrain() bool {
	for _, p := range m.Providers {
		if p == "brain" {
			return true
		}
	}
	return false
}

// asciiFoldCase is one in/out vector of the fixture's `asciiFold` block: the
// ASCII-ONLY fold the secret gate runs, pinned as a primitive rather than
// through a containment verdict.
type asciiFoldCase struct {
	In  string `json:"in"`
	Out string `json:"out"`
}

type asciiFoldBlock struct {
	Cases []asciiFoldCase `json:"cases"`
}

// contractVocabulary is the fixture's declared vocabulary: the token names a
// loader may substitute, the `group` names a case may belong to, and the
// reasons a deny may be denied for. Every one of the three used to be validated
// by nothing at all.
type contractVocabulary struct {
	Tokens      map[string]string `json:"tokens"`
	Groups      map[string]string `json:"groups"`
	DenyReasons map[string]string `json:"denyReasons"`
}

type contractFixture struct {
	Vocabulary         contractVocabulary  `json:"vocabulary"`
	Owners             map[string][]string `json:"owners"`
	SecretBasenames    []string            `json:"secretBasenames"`
	ConfigStoreSubdirs []string            `json:"configStoreSubdirs"`
	AsciiFold          asciiFoldBlock      `json:"asciiFold"`
	CheckUse           []contractCheckUse  `json:"checkUse"`
	Cases              []contractCase      `json:"cases"`
	Methods            []contractMethod    `json:"methods"`
}

// contractCheckUse is one owner's record of BINDING DECISION 2's second half:
// the canonical path the guard returned is what the call site opens. `callSites`
// names every place that has to be true, and until this struct existed neither
// Go loader read the block at all.
type contractCheckUse struct {
	Owner       string   `json:"owner"`
	Requirement string   `json:"requirement"`
	CallSites   []string `json:"callSites"`
}

// TestAsciiFoldMatchesTheFixture pins asciiLower itself.
//
// All three copies carry the same comment — "deliberately not strings.ToLower /
// toLowerCase, because the three copies have to fold IDENTICALLY" — and nothing
// enforced it: replacing the body with strings.ToLower (or, on the desktop,
// s.toLowerCase()) kept the entire corpus and both full suites green, because
// every case-variant CASE in the corpus uses pure A-Z spellings that both folds
// agree on. The vectors carry code points where the folds disagree, including
// U+0130, where Go's Unicode fold and JavaScript's do not even agree with each
// other.
func TestAsciiFoldMatchesTheFixture(t *testing.T) {
	fx := loadContractFixture(t)
	if len(fx.AsciiFold.Cases) == 0 {
		t.Fatal("the fixture must carry asciiFold vectors, or this guard guards nothing")
	}
	sawNonASCII := false
	for _, c := range fx.AsciiFold.Cases {
		for _, r := range c.In {
			if r > 127 {
				sawNonASCII = true
			}
		}
		if got := asciiLower(c.In); got != c.Out {
			t.Errorf("asciiLower(%q) = %q, want %q", c.In, got, c.Out)
		}
	}
	if !sawNonASCII {
		t.Fatal("every asciiFold vector is pure ASCII, so strings.ToLower would pass them all — the block distinguishes nothing")
	}
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

// TestSecretGateConstantsMatchTheFixture pins the two parts of the `secrets`
// gate the CASES cannot reach.
//
// Every secrets case names one of the two credential basenames and one of the
// three stores, so adding a THIRD basename here — or a fourth store carve-out —
// keeps the whole corpus green while the copies silently drift apart. That is
// not hypothetical drift either: carving out `plugins` flips pathIsSecret from
// true to false for <configDir>/plugins/**, i.e. every plugin's stored files
// become readable and writable through fs.* the moment an agent cwd re-admits
// the config dir.
//
// The fixture carries `secretBasenames` and `configStoreSubdirs` for exactly
// this reason, and the desktop loader has consumed them since it was written —
// but neither Go loader's fixture struct even DECLARED the fields, so both Go
// copies could drift freely with every hub suite green. The lists are one list;
// all three owners have to be held to it.
func TestSecretGateConstantsMatchTheFixture(t *testing.T) {
	fx := loadContractFixture(t)
	if len(fx.SecretBasenames) == 0 || len(fx.ConfigStoreSubdirs) == 0 {
		t.Fatal("the fixture must carry both secretBasenames and configStoreSubdirs, or this guard guards nothing")
	}

	got := make([]string, 0, len(secretBasenames))
	for name := range secretBasenames {
		got = append(got, name)
	}
	want := append([]string(nil), fx.SecretBasenames...)
	sort.Strings(got)
	sort.Strings(want)
	if strings.Join(got, ",") != strings.Join(want, ",") {
		t.Errorf("secretBasenames drifted from the fixture\n  got:  %v\n  want: %v", got, want)
	}

	// configStoreRoots() is the same three names joined onto the config dir, in
	// the same order — the desktop twin asserts the identical shape, and the bus
	// asserts the bare list.
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	wantRoots := make([]string, 0, len(fx.ConfigStoreSubdirs))
	for _, store := range fx.ConfigStoreSubdirs {
		wantRoots = append(wantRoots, filepath.Join(configDir(), store))
	}
	gotRoots := configStoreRoots()
	if strings.Join(gotRoots, "\x00") != strings.Join(wantRoots, "\x00") {
		t.Errorf("configStoreRoots drifted from the fixture's configStoreSubdirs\n  got:  %v\n  want: %v", gotRoots, wantRoots)
	}
}

// TestTheSecretGateCarvesOutExactlyTheFixturesStores is the BEHAVIOURAL half of
// the test above, and the half that actually guards the escalation.
//
// TestSecretGateConstantsMatchTheFixture pins what configStoreRoots() RETURNS.
// Nothing pinned that pathIsSecretCanonical ITERATES that same list — the gate
// holds its own loop, so hardcoding a wider set there (say a fourth entry for
// `plugins`) re-admits <configDir>/plugins/** while the helper, the fixture and
// all 106 cases stay green. That directory is every installed plugin's manifest,
// cache and state, sitting next to the .bus-token and .settings.json the two
// basenames cover. The corpus cases cannot see it either: each one names one of
// the three real stores, so a gate with FOUR carve-outs satisfies every one.
//
// So: the carve-out set the gate applies must equal the fixture's list exactly,
// asserted in both directions on the gate itself.
func TestTheSecretGateCarvesOutExactlyTheFixturesStores(t *testing.T) {
	fx := loadContractFixture(t)
	if len(fx.ConfigStoreSubdirs) == 0 {
		t.Fatal("the fixture must carry configStoreSubdirs")
	}
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", sandbox)
	t.Setenv("APPDATA", sandbox)
	cfg := configDir()
	if err := os.MkdirAll(cfg, 0o755); err != nil {
		t.Fatal(err)
	}

	for _, store := range fx.ConfigStoreSubdirs {
		if err := os.MkdirAll(filepath.Join(cfg, store), 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(cfg, store, "item.md")
		if pathIsSecret(target) {
			t.Errorf("the fixture carves out %q but the gate still refuses %s — the web/remote UI cannot reach its own store", store, target)
		}
	}

	// The other direction. These are ordinary config-dir neighbours; every one of
	// them must stay refused, and `plugins` is the one whose exemption is the
	// documented escalation.
	exempt := map[string]bool{}
	for _, store := range fx.ConfigStoreSubdirs {
		exempt[store] = true
	}
	for _, name := range []string{"plugins", "cache", "logs", "handoffs", "backups", "supervisor"} {
		if exempt[name] {
			continue
		}
		if err := os.MkdirAll(filepath.Join(cfg, name), 0o755); err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(cfg, name, "anything.json")
		if !pathIsSecret(target) {
			t.Errorf("the gate exempts <configDir>/%s, which the fixture does not list — its carve-out set has drifted from configStoreSubdirs", name)
		}
	}
	// And the dir itself, which is neither a carve-out nor inside one.
	if !pathIsSecret(filepath.Join(cfg, "remote-token")) {
		t.Error("remote-token is not refused — the gate is not running at all")
	}
}

// TestEveryCorpusCaseBelongsToAGroupSOMEBODYOwns closes the complement of the
// `owned == 0` floors.
//
// Every loader guards against running ZERO cases for a group it owns. Nothing
// guarded the other direction: that every case in the corpus belongs to a group
// somebody owns. Both Go loaders filter with `if !groups[c.Group] { continue }`,
// so a ONE-CHARACTER TYPO in a case's `group` ("containmnet") drops it silently
// from the brain — the copy that actually answers fs.*/library.* under the
// default catalog delegation — and from the bus, leaving only a t.Logf behind.
// The TypeScript loader does not filter by group at all and keeps running it, so
// the three copies quietly stop being held to the same corpus, which is the one
// thing this fixture exists to prevent.
func TestEveryCorpusCaseBelongsToAGroupSOMEBODYOwns(t *testing.T) {
	fx := loadContractFixture(t)
	owned := map[string]bool{}
	for _, groups := range fx.Owners {
		for _, g := range groups {
			owned[g] = true
		}
	}
	if len(owned) == 0 {
		t.Fatal("the fixture's owners map lists no groups at all")
	}
	counts := map[string]int{}
	for _, c := range fx.Cases {
		if c.Group == "" {
			t.Errorf("case %q has no group, so no loader will run it", c.Name)
			continue
		}
		if !owned[c.Group] {
			t.Errorf("case %q is in group %q, which appears in NO owner's list — every Go loader skips it silently and the copies stop being held to the same corpus",
				c.Name, c.Group)
		}
		counts[c.Group]++
	}
	for g := range owned {
		if counts[g] == 0 {
			t.Errorf("group %q is owned by somebody but has no cases", g)
		}
	}
	t.Logf("corpus groups: %v", counts)
}

// contractTokenTable is this loader's substitution table, and the ONE place the
// token names it understands are written down. caseSandbox substitutes out of
// it and TestFixtureVocabularyIsClosed compares its key set against the
// fixture's `vocabulary.tokens`, which is what makes the declaration binding in
// both directions: a token the fixture declares and this loader cannot expand
// fails here, and so does a token this loader expands that the fixture does not
// declare. Legalizing a typo'd token by adding it to the fixture therefore
// fails in all three loaders, because not one of them substitutes it.
func contractTokenTable(sandbox, configHome, home, processCwd string) map[string]string {
	return map[string]string{
		"SANDBOX":     sandbox,
		"ROOT":        filepath.Join(sandbox, "root"),
		"OUTSIDE":     filepath.Join(sandbox, "outside"),
		"CONFIG":      filepath.Join(configHome, "workspacer"),
		"HOME":        home,
		"PROCESS_CWD": processCwd,
	}
}

// contractDenyReasonNames is contractDenyReason's declared range, pinned against
// the fixture's `vocabulary.denyReasons` so a reason can neither be declared
// without a classifier arm nor classified without being declared.
var contractDenyReasonNames = []string{"not-absolute", "unresolvable", "outside-roots", "secret"}

// contractDenyReason classifies a refusal by re-running assertPathAllowed's own
// three gates in assertPathAllowed's own order. The guard itself deliberately
// collapses all of them into one message (7.5), so the reason has to be
// recomputed from the exported predicates rather than parsed out of the error.
//
// "allowed" is returned when no gate fires; it is deliberately NOT a declared
// reason, so a deny case that reaches it fails with the mismatch spelled out.
func contractDenyReason(target string, roots []string) string {
	ct, err := canonicalizePath(target)
	switch {
	case errors.Is(err, errEmptyPath), errors.Is(err, errNotAbsolute):
		return "not-absolute"
	case err != nil:
		return "unresolvable"
	case !pathWithinRootsCanonical(roots, ct):
		return "outside-roots"
	case pathIsSecretCanonical(ct):
		return "secret"
	}
	return "allowed"
}

// contractTokenRefs collects every token reference in a string. `unterminated`
// reports a "${" with no closing brace, which the substituter leaves verbatim
// exactly like a mis-spelled name does.
func contractTokenRefs(s string) (names []string, unterminated bool) {
	for i := 0; i < len(s); {
		j := strings.Index(s[i:], "${")
		if j < 0 {
			break
		}
		start := i + j + 2
		end := strings.IndexByte(s[start:], '}')
		if end < 0 {
			return names, true
		}
		names = append(names, s[start:start+end])
		i = start + end + 1
	}
	return names, false
}

// walkContractStrings visits every string in the decoded fixture — map VALUES
// and map KEYS, prose `_comment` blocks included. Comments are in scope on
// purpose: a mis-spelling in the prose is how a mis-spelling in a case gets
// written, and the fixture's own vocabulary block says so.
func walkContractStrings(v any, where string, visit func(where, s string)) {
	switch t := v.(type) {
	case string:
		visit(where, t)
	case []any:
		for i, e := range t {
			walkContractStrings(e, fmt.Sprintf("%s[%d]", where, i), visit)
		}
	case map[string]any:
		for k, e := range t {
			visit(where+"."+k+" (key)", k)
			walkContractStrings(e, where+"."+k, visit)
		}
	}
}

// TestFixtureVocabularyIsClosed is the guard for the class of defect that made
// every deny case in this corpus individually unfalsifiable.
//
// A one-character typo in a ${TOKEN} name defangs a deny case in ALL THREE
// loaders at once and in silence: the name does not substitute, the target
// becomes a relative literal, every copy refuses it for not being absolute, and
// the case passes while exercising nothing. Applying that to all 64 deny
// targets left all three suites green. The sibling defect is a typo in a case's
// `group`: both Go loaders filter with `if !groups[c.Group] { continue }`, so
// the case silently stops running here and on the bus while TypeScript, which
// does not filter, keeps running it — the three copies quietly stop being held
// to the same corpus, which is the one thing the fixture exists to prevent.
//
// The per-case substituter's assertNoResidualToken is not enough on its own:
// it only fires for cases that actually RUN, so a typo in a case this platform
// skips (posixOnly, needsSymlinks, needsUnreadableDir, needsHome), or in a case
// whose `group` typo already dropped it, is never seen. This test is static —
// it reads the whole document, and every check below holds whether or not a
// single case executes.
//
// TWINS: internal/bus/policy_test.go and main/lib/pathConfinement.test.ts run
// the same checks. A check only ONE loader runs is how secretBasenames drifted.
func TestFixtureVocabularyIsClosed(t *testing.T) {
	fx := loadContractFixture(t)
	vocab := fx.Vocabulary
	if len(vocab.Tokens) == 0 || len(vocab.Groups) == 0 || len(vocab.DenyReasons) == 0 {
		t.Fatalf("the fixture must declare vocabulary.tokens, .groups and .denyReasons; got %d/%d/%d — an empty vocabulary makes every check below vacuous",
			len(vocab.Tokens), len(vocab.Groups), len(vocab.DenyReasons))
	}

	// 1. The declaration and this loader's substitution table are one list.
	table := contractTokenTable("/sandbox", "/sandbox/config", "/home/u", "/wd")
	for name := range vocab.Tokens {
		if _, ok := table[name]; !ok {
			t.Errorf("the fixture declares token %q but this loader's substitution table has no entry for it — every case using it would silently test a literal", name)
		}
	}
	for name := range table {
		if _, ok := vocab.Tokens[name]; !ok {
			t.Errorf("this loader substitutes token %q, which the fixture does not declare — vocabulary.tokens is supposed to be the whole set", name)
		}
	}

	// 2. Every token reference in the WHOLE document names a declared token.
	raw, err := os.ReadFile(contractFixtureRel)
	if err != nil {
		t.Fatalf("read the shared fixture: %v", err)
	}
	var doc any
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	walkContractStrings(doc, "", func(where, s string) {
		names, unterminated := contractTokenRefs(s)
		if unterminated {
			t.Errorf(`%s: a "${" with no closing "}" in %q — the substituter leaves it verbatim, exactly like a mis-spelled name`, where, s)
		}
		for _, name := range names {
			if _, ok := vocab.Tokens[name]; !ok {
				t.Errorf("%s: %q is not a declared token (vocabulary.tokens has %v) — it passes through verbatim and silently defangs whatever uses it\n  in: %q",
					where, "${"+name+"}", sortedVocabKeys(vocab.Tokens), s)
			}
		}
	})

	// 3. Every declared token is USED by a case field, not merely mentioned in
	// prose. A token no case substitutes is a token no loader is proved to
	// expand, and it is also the cheapest way to smuggle a typo in: declare it.
	used := map[string]bool{}
	for _, c := range fx.Cases {
		for _, s := range append(append([]string{}, c.Roots...), c.Target, c.ResolvesTo) {
			names, _ := contractTokenRefs(s)
			for _, name := range names {
				used[name] = true
			}
		}
	}
	for name := range vocab.Tokens {
		if !used[name] {
			t.Errorf("token %q is declared but no case's roots/target/resolvesTo uses it — nothing proves any loader substitutes it", name)
		}
	}

	// 4. Groups: every case's, and every layer any owner claims.
	caseCount := map[string]int{}
	for _, c := range fx.Cases {
		if _, ok := vocab.Groups[c.Group]; !ok {
			t.Errorf("case %q is in group %q, which vocabulary.groups does not declare — both Go loaders skip it silently and the three copies stop being held to the same corpus",
				c.Name, c.Group)
			continue
		}
		caseCount[c.Group]++
	}
	ownedGroups := map[string]bool{}
	for owner, layers := range fx.Owners {
		for _, g := range layers {
			if _, ok := vocab.Groups[g]; !ok {
				t.Errorf("owner %s claims group %q, which vocabulary.groups does not declare", owner, g)
			}
			ownedGroups[g] = true
		}
	}
	for g := range vocab.Groups {
		if caseCount[g] == 0 {
			t.Errorf("group %q is declared but no case belongs to it", g)
		}
		if !ownedGroups[g] {
			t.Errorf("group %q is declared but no owner implements it, so every loader skips its cases", g)
		}
	}

	// 5. deniedBy: present, declared, and exhaustive in both directions.
	reasonCount := map[string]int{}
	for _, c := range fx.Cases {
		switch c.Expect {
		case "deny":
			if c.DeniedBy == "" {
				t.Errorf("deny case %q names no deniedBy — `expect: deny` alone is satisfied by a refusal for ANY reason, which is exactly how a defanged case keeps passing", c.Name)
				continue
			}
			if _, ok := vocab.DenyReasons[c.DeniedBy]; !ok {
				t.Errorf("deny case %q claims reason %q, which vocabulary.denyReasons does not declare", c.Name, c.DeniedBy)
				continue
			}
			reasonCount[c.DeniedBy]++
		case "allow":
			if c.DeniedBy != "" {
				t.Errorf("allow case %q carries deniedBy %q; an allow is pinned by resolvesTo instead", c.Name, c.DeniedBy)
			}
		default:
			t.Errorf("case %q has expect %q, which is neither \"allow\" nor \"deny\"", c.Name, c.Expect)
		}
	}
	for r := range vocab.DenyReasons {
		if reasonCount[r] == 0 {
			t.Errorf("deny reason %q is declared but no case names it — an unexercised classification arm is one nothing holds to the other copies", r)
		}
	}

	// 6. The classifier's range is exactly the declared set.
	declared := sortedVocabKeys(vocab.DenyReasons)
	classifier := append([]string(nil), contractDenyReasonNames...)
	sort.Strings(classifier)
	if strings.Join(declared, ",") != strings.Join(classifier, ",") {
		t.Errorf("contractDenyReason's range drifted from vocabulary.denyReasons\n  classifier: %v\n  fixture:    %v", classifier, declared)
	}
}

func sortedVocabKeys(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
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
	home := realHome()
	if c.NeedsHome && home == "" {
		t.Skip("needsHome: this process has no resolvable home directory")
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
	// A case may ask for the config dir to be REACHED through a symlink. The
	// link is created here, before the tree, because the environment below has
	// to point at it; ${CONFIG} still substitutes to the real path, so the two
	// only agree if the implementation canonicalizes its own config dir.
	configEnv := configHome
	if c.ConfigDirVia != "" {
		link := filepath.Join(sandbox, filepath.FromSlash(c.ConfigDirVia))
		if err := os.Symlink(configHome, link); err != nil {
			t.Skipf("configDirVia: cannot create symlinks here: %v", err)
		}
		configEnv = link
	}
	// Read at call time by configDir(), so this redirects the secret gate's
	// config dir to ${CONFIG} = ${SANDBOX}/config/workspacer.
	t.Setenv("XDG_CONFIG_HOME", configEnv)
	t.Setenv("APPDATA", configEnv)

	// ${HOME} and ${PROCESS_CWD} deliberately leave the sandbox: they are the two
	// places a re-introduced tilde expansion, or a bad root resolved against the
	// process cwd, would actually LAND. No case using them expects `allow` (see
	// the fixture's TOKENS note).
	processCwd := ""
	if wd, err := os.Getwd(); err == nil {
		processCwd = wd
		if real, err := filepath.EvalSymlinks(wd); err == nil {
			processCwd = real
		}
	}
	// Driven by the ONE table (see contractTokenTable), so the set of names this
	// loader can expand is a value the vocabulary test can compare against the
	// fixture's declaration rather than a chain of literals nothing reads.
	// Sorted for determinism only: no token's VALUE can contain another token.
	table := contractTokenTable(sandbox, configHome, home, processCwd)
	names := make([]string, 0, len(table))
	for name := range table {
		names = append(names, name)
	}
	sort.Strings(names)
	sub := func(s string) string {
		for _, name := range names {
			s = strings.ReplaceAll(s, "${"+name+"}", table[name])
		}
		assertNoResidualToken(t, s)
		return s
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
		// Absolutized here; `relativeSymlinks` below is the arm that is not.
		if err := os.Symlink(filepath.Join(sandbox, dest), full); err != nil {
			if c.NeedsSymlinks {
				t.Skipf("needsSymlinks: cannot create symlinks here: %v", err)
			}
			t.Fatal(err)
		}
	}
	// Written verbatim: the link BODY stays relative, so resolving it is the
	// implementation's job and not the loader's.
	for rel, dest := range c.Tree.RelativeSymlinks {
		full := filepath.Join(sandbox, rel)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.Symlink(filepath.FromSlash(dest), full); err != nil {
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

// realHome is the process's home directory with symlinks resolved, or "" when
// there isn't one. Both the ${HOME} token and the browse-skip need the RESOLVED
// form: every other path in this file is realpath'd, and on macOS /var ->
// /private/var alone would make the comparison wrong.
func realHome() string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return ""
	}
	if real, err := filepath.EvalSymlinks(home); err == nil {
		return real
	}
	return home
}

// underHome reports whether an absolute target sits at or inside the home tree.
// Used only to skip browse-rootSet method cases whose target is legitimately
// browsable; it is never consulted for a verdict.
func underHome(target string) bool {
	home := realHome()
	if home == "" || !filepath.IsAbs(target) {
		return false
	}
	return target == home || strings.HasPrefix(target, home+string(filepath.Separator))
}

// withDeadline runs one guard call and fails the test rather than hanging on it.
// A goroutine that spins on a symlink cycle cannot be cancelled, so it is left to
// leak — the test binary is going down with a failure anyway.
func withDeadline[T any](t *testing.T, fn func() (T, error)) (T, error) {
	t.Helper()
	type result struct {
		val T
		err error
	}
	done := make(chan result, 1)
	go func() {
		v, err := fn()
		done <- result{v, err}
	}()
	select {
	case r := <-done:
		return r.val, r.err
	case <-time.After(10 * time.Second):
		var zero T
		t.Fatalf("the guard did not return within 10s — a symlink cycle spun the walk (the hop counter is the only ELOOP bound this code has)")
		return zero, nil
	}
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

	// `owned` counts cases the loop will REGISTER. Every one of them can still
	// skip itself inside caseSandbox (posixOnly, needsUnreadableDir, needsHome,
	// needsSymlinks, configDirVia), and on a host without symlink privilege that
	// is most of the corpus — a green package over an empty sweep. The tally
	// counts what actually asserted, and the floor at the bottom is what turns
	// "nothing ran" into red.
	var tally sweepguard.Tally

	for _, c := range fx.Cases {
		if !groups[c.Group] {
			continue
		}
		t.Run(c.Name, func(t *testing.T) {
			t.Cleanup(func() {
				if t.Skipped() {
					tally.Skip(hostSkipReason(c))
				}
			})
			_, sub := caseSandbox(t, c)
			// Past every skip gate: this case is going to assert.
			tally.Ran(c.Expect)
			roots := substituted(sub, c.Roots)
			target := sub(c.Target)

			// WATCHDOG. The walk is hand-rolled and never reaches the platform's
			// ELOOP, so the only thing between a symlink cycle and a spin is the
			// hop counter — and the two cycle cases that DO exercise the
			// relative-link arm would otherwise fail by hanging until the whole
			// package's 10-minute timeout, which is a red build nobody can read.
			// Bound it here so a counter regression is a fast, named failure.
			canonical, err := withDeadline(t, func() (string, error) {
				return assertPathAllowed("contract", target, roots)
			})
			allowed := err == nil
			want := c.Expect == "allow"
			if allowed != want {
				t.Fatalf("expected %s, got allowed=%v (err=%v)\n  target: %q\n  roots:  %q\n  why:    %s",
					c.Expect, allowed, err, target, roots, c.Why)
			}

			// The verdict must DECOMPOSE into the two named predicates this file
			// exports for a raw (not-yet-canonicalized) target. Both shipped with
			// zero callers and 0.0% coverage — while carrying the same names as
			// the bus's twins, which ARE live (bus.go authorize, policy.go). So
			// their fail-closed branches (`return false` and `return true` on an
			// unverifiable target) had never been executed by anything, and the
			// first future call site to reach for the obvious-looking name would
			// have got a predicate no case had ever run. Asserting the identity
			// rather than each half separately is what makes this non-vacuous:
			// it says these two ARE the gates assertPathAllowed applies, in the
			// same order, with the same posture on an unverifiable path.
			within := pathWithinRoots(roots, target)
			secret := pathIsSecret(target)
			if got := within && !secret; got != allowed {
				t.Fatalf("assertPathAllowed says allowed=%v but pathWithinRoots=%v && !pathIsSecret=%v decomposes to %v — the exported predicates and the guard disagree\n  target: %q\n  roots:  %q",
					allowed, within, !secret, got, target, roots)
			}
			if !allowed {
				// 7.5: one message for all three refusal reasons, echoing
				// neither the target, nor where it resolved, nor which gate
				// fired. Anything else is a probe primitive for a remote caller.
				if got, want := err.Error(), "contract: "+refusalText; got != want {
					t.Fatalf("refusal message drifted\n  got:  %q\n  want: %q", got, want)
				}
				// THE RIGHT REASON. A deny that happens for the wrong reason is
				// a case that tests nothing while reporting green — a mangled
				// ${TOKEN} makes the target a relative literal and every copy
				// refuses it for not being absolute, with the case's name still
				// claiming it exercises a symlink escape. deniedBy is the
				// fixture's independent statement of which gate must fire, and
				// it is NOT derivable from `group`: 'a symlink out of an allowed
				// root into the config dir' is a secrets case that containment
				// refuses first.
				if got := contractDenyReason(target, roots); got != c.DeniedBy {
					t.Fatalf("denied for the WRONG REASON: got %q, the fixture says %q\n  target: %q\n  roots:  %q\n  why:    %s",
						got, c.DeniedBy, target, roots, c.Why)
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
			// The VALUE, not just its shape. Shape alone is what let a
			// `return filepath.Clean(target)` — the forbidden whole-path helper,
			// named as such in fsguard.go's own header — pass every case in this
			// corpus while fs.read handed back a file outside the only allowed
			// root: Clean's answer is absolute and free of "." and ".." too, it
			// just points somewhere else. resolvesTo is mandatory on an allow so
			// a future case cannot be written without pinning the answer.
			if c.ResolvesTo == "" {
				t.Fatalf("allow case %q carries no resolvesTo; every allow case must pin the path the guard returns", c.Name)
			}
			if want := filepath.FromSlash(sub(c.ResolvesTo)); canonical != want {
				t.Fatalf("the guard returned a different path than it validated\n  got:  %q\n  want: %q\n  why:  %s", canonical, want, c.Why)
			}
		})
	}

	// Both classes, separately. A corpus that ran only allows says the guard
	// lets things through and nothing else; a corpus that ran only denies is
	// satisfied by a guard that refuses everything.
	if err := tally.RequireBoth("the fsguard containment corpus"); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
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
	ran := 0
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
			// RELOCATE $HOME before the sandbox is built, not after. A
			// browse-rootSet method also allows the whole home tree, so a target
			// under $HOME is refused by no root — and t.TempDir() lands under
			// $HOME whenever TMPDIR does (TMPDIR=~/tmp is an ordinary setup, and
			// the loader's own comment names it). The previous shape skipped
			// those cases instead, which on such a machine disarmed the probe
			// completely and SILENTLY: fs.listDir skipped 45 of its 45 deny
			// cases, library.list 44 of 45, and the package still printed ok,
			// because the only count assertion was over the method list and each
			// method's hand-written floor subtest still passed. That is the
			// self-disarming oracle this file's header says it was rewritten to
			// eliminate, re-introduced by a skip instead of a sandbox — so do
			// what the sibling sweep (TestPathBearingMethodRootSetsMatchTheCorpus)
			// already does and give the case a home of its own, a SIBLING of the
			// sandbox rather than an ancestor of it.
			//
			// The ${HOME} token still resolves to whatever is set here, so the
			// cases that deliberately probe the home tree keep working — they
			// just probe a home that contains no sandbox.
			fakeHome, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Setenv("HOME", fakeHome)
			t.Setenv("USERPROFILE", fakeHome)

			sandbox, sub := caseSandbox(t, c)
			// The relocation only works if the sandbox is a SIBLING of the fake
			// home rather than a child of it. If that ever stops holding, every
			// browse case goes back to skipping and the sweep goes quiet, so
			// assert it rather than discover it in a hunter's report.
			if underHome(sandbox) {
				t.Fatalf("the case sandbox %s is inside the relocated $HOME %s — the browse skip below would swallow every case", sandbox, fakeHome)
			}
			target := sub(c.Target)
			if why := blankTargetIsAnsweredBeforeTheGuard(m, target); why != "" {
				t.Skip(why)
			}
			// What remains after the relocation: the handful of cases that AIM at
			// the home tree on purpose (${HOME}, and ${PROCESS_CWD} when the
			// daemon runs from inside it). A browse-rootSet method legitimately
			// browses those, so they cannot be deny cases for it. This skip now
			// covers only them.
			if m.RootSet == "browse" && underHome(target) {
				t.Skipf("target %s is inside $HOME, which %s legitimately browses", target, m.Method)
			}
			ran++

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
	// The per-method floor. TestEveryPathBearingBrainMethodIsConfined counts
	// METHODS, not cases, so a method that contributed zero cases was invisible.
	if ran == 0 {
		t.Fatalf("%s ran ZERO corpus deny cases — this subtest asserted nothing", m.Method)
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
//
// THE HOME IS A SANDBOX, AND THAT IS THE WHOLE POINT OF THIS PARAGRAPH. This
// sweep used to probe with a FIXED name in the developer's REAL home
// ($HOME/wks-contract-probe-not-an-agent-cwd) and t.Skipf if that path already
// existed — "the probe must be a path nothing owns". But the sweep drives the
// REAL handlers, so the fs.write subtest CREATES that exact file the moment
// fs.write admits a $HOME path. The test therefore disarmed itself, permanently
// and for every OTHER method too, the first time it detected the defect it
// exists to detect: five later subtests were already skipped within that same
// run, and every run afterwards skipped all eight and reported PASS. It had
// already happened on the developer machine (a 5-byte file containing "hello",
// which is verbatim the corpus's fs.write params), so fs.read and
// search.project could each be widened to browseRoots with the entire Go suite
// green. Any process running as the user could arm it with one `touch`.
//
// So HOME is redirected at a fresh t.TempDir() per subtest: the probe cannot
// pre-exist, cannot be squatted, and a write that lands there is thrown away
// with the sandbox instead of poisoning the next run. A probe that somehow
// exists anyway is a FATAL, never a skip — the one thing this test must not do
// is treat "I could not run" as "I passed" — and the loop asserts it ran.
func TestPathBearingMethodRootSetsMatchTheCorpus(t *testing.T) {
	fx := loadContractFixture(t)
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	dispatched := map[string]bool{}
	for _, set := range [][]string{reg.methods(), reg.catalogMethods()} {
		for _, m := range set {
			dispatched[m] = true
		}
	}

	ran := 0
	for _, m := range fx.Methods {
		if !m.providedByBrain() || !dispatched[m.Method] {
			continue
		}
		ran++
		t.Run(m.Method, func(t *testing.T) {
			// A config dir of its own, so the config stores cannot be what
			// admits (or refuses) the probe.
			t.Setenv("XDG_CONFIG_HOME", t.TempDir())
			// A home of its own, for the reasons in the header. USERPROFILE is
			// what os.UserHomeDir reads on Windows, HOME everywhere else; set
			// both so the sandbox holds on either.
			fakeHome, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Setenv("HOME", fakeHome)
			t.Setenv("USERPROFILE", fakeHome)
			home, err := os.UserHomeDir()
			if err != nil || home == "" {
				t.Fatalf("os.UserHomeDir must follow the sandboxed home (%q): %v", fakeHome, err)
			}
			if home != fakeHome {
				t.Fatalf("os.UserHomeDir returned %q, not the sandbox %q — the probe below would land in the real home", home, fakeHome)
			}
			cwd, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			// Under $HOME, does not exist, and is nobody's cwd: inside the
			// browse roots and outside the workspace roots, by construction.
			probe := filepath.Join(home, "wks-contract-probe-not-an-agent-cwd")
			if _, err := os.Lstat(probe); err == nil {
				t.Fatalf("%s exists inside a freshly created sandbox home — the sandbox is not fresh, and the probe no longer proves anything", probe)
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

			// The UPPER boundary of `browse`, which nothing probed. browse is
			// "the home tree plus the workspace roots"; the deny probes are all
			// outside $HOME's PARENT as well, so widening browseRoots to
			// filepath.Dir(home) — every other user's home directory — changed no
			// assertion anywhere. A sibling of $HOME is inside the widened set and
			// outside the real one, so it must be refused whatever the rootSet is.
			sibling := filepath.Join(filepath.Dir(home), "wks-contract-probe-sibling-of-home")
			if _, err := os.Lstat(sibling); err == nil {
				t.Fatalf("%s already exists — the sandbox is not fresh and this probe proves nothing", sibling)
			}
			sibParams := map[string]any{}
			for k, v := range m.Params {
				sibParams[k] = v
			}
			sibParams[m.Field] = sibling
			sibBody, _ := json.Marshal(sibParams)
			if _, sibErr := reg.handle(context.Background(), m.Method, json.RawMessage(sibBody)); sibErr == nil ||
				!strings.Contains(sibErr.Error(), refusalText) {
				t.Fatalf("%s accepted a SIBLING of $HOME (%s) — neither root set reaches outside the home tree (err=%v)", m.Method, sibling, sibErr)
			}

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
	// Zero subtests is the failure mode this whole family of tests keeps
	// re-learning: TestPathContainmentContractCases fails hard on an empty owned
	// set for the same reason. Without this, a corpus that stopped naming
	// "brain" as a provider — or a registry that stopped dispatching — would
	// report a green PASS having asserted nothing at all.
	if ran == 0 {
		t.Fatal("no brain-provided path-bearing method was swept — the corpus, the registry and this loop have drifted out of overlap and the rootSet column is pinned by nothing")
	}
	t.Logf("swept the rootSet column for %d brain-provided methods", ran)
}

// ---------------------------------------------------------------------------
// checkUse: the OTHER half of BINDING DECISION 2.
//
// The fixture's `checkUse` block requires that assertPathAllowed's RETURN VALUE
// is what reaches the filesystem, and lists the call sites — but a corpus case
// can only ever exercise the predicate, so every one of those call sites could
// go back to opening the caller's raw string with the whole corpus green. The
// ordinary symlink cases cannot catch it either: the kernel resolves a symlink
// the same way this walk does, so raw and canonical name the SAME file.
//
// One input tells them apart. `<root>/nope/../notes.txt` canonicalizes to
// `<root>/notes.txt` — the walk appends a component that does not exist and lets
// a later ".." pop back onto ground that does (the corpus's "'..' after a
// non-existent component pops lexically and lands inside", an ALLOW case) —
// while the kernel refuses the raw string with ENOENT, because `nope` is not
// there to walk through. So a handler that re-opens the caller's string fails
// every assertion below, and one that opens the guard's answer passes.
func TestGuardedHandlersOpenTheCanonicalPathTheyValidated(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "notes.txt"), []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, d := range []string{"sub", "real", "other"} {
		if err := os.MkdirAll(filepath.Join(dir, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	// `<dir>/sub/link` -> `<dir>/real`, so the link's own parent (`<dir>/sub`)
	// and its target's parent (`<dir>`) differ — the only shape that separates a
	// per-component walk from a textual clean.
	if err := os.Symlink(filepath.Join(dir, "real"), filepath.Join(dir, "sub", "link")); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	// The probe every subtest uses, carrying BOTH defects at once. It resolves to
	// `<dir>/<name>` and nothing else does:
	//
	//   raw string            ENOENT — `nope` is not there to walk through, so a
	//                         handler that re-opens the caller's string fails.
	//   textual clean         `<dir>/sub/<name>` — `link/..` collapses to `<dir>/sub`
	//                         before the link is read, so a handler that Cleans (or a
	//                         guard that RETURNS a Clean) names a different file.
	//   per-component walk    `<dir>/<name>`, which is what must be opened.
	//
	// Four of these six subtests used the `nope/..`-only probe, for which a clean
	// and the walk agree — so they stayed green while fs.read was handing back a
	// file outside the root.
	via := func(name string) string {
		return filepath.Join(dir, "sub", "link") + "/../nope/../" + name
	}

	// Every subtest is named for the exact `callSites` entry it covers, and the
	// set is compared against the fixture at the end. The fixture's checkUse
	// block used to be read by NO Go loader at all — neither Go fixture struct
	// even declared the field — so its nine call sites were a comment, and the
	// hand-maintained table here silently covered six of them.
	covered := map[string]bool{}
	run := func(site string, fn func(t *testing.T)) {
		covered[site] = true
		t.Run(site, fn)
	}

	run("handlers.go fs.read -> readTextFile", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "fs.read",
			json.RawMessage(`{"path":`+jsonStr(via("notes.txt"))+`}`))
		if err != nil {
			t.Fatalf("fs.read must open the path the guard returned, not the caller's string: %v", err)
		}
		var res struct {
			Path     string `json:"path"`
			Contents string `json:"contents"`
		}
		if err := json.Unmarshal(raw, &res); err != nil {
			t.Fatal(err)
		}
		if res.Contents != "hello" {
			t.Fatalf("fs.read returned %q", res.Contents)
		}
		if res.Path != filepath.Join(dir, "notes.txt") {
			t.Errorf("fs.read reported %q, want the canonical %q — a caller round-tripping this "+
				"path must stay inside the guard", res.Path, filepath.Join(dir, "notes.txt"))
		}
	})

	run("handlers.go fs.write -> writeHostFile", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		if _, err := reg.handle(context.Background(), "fs.write",
			json.RawMessage(`{"path":`+jsonStr(via("out.txt"))+`,"contents":"written"}`)); err != nil {
			t.Fatalf("fs.write must write the path the guard returned: %v", err)
		}
		body, err := os.ReadFile(filepath.Join(dir, "out.txt"))
		if err != nil || string(body) != "written" {
			t.Fatalf("fs.write did not land on the canonical path: %q %v", body, err)
		}
		if _, err := os.Stat(filepath.Join(dir, "nope")); err == nil {
			t.Error("fs.write materialized the unresolved component instead of using the canonical path")
		}
		// writeHostFile creates missing parents, so a cleaned path does not fail
		// — it lands one directory over, silently. Name that outcome.
		if _, err := os.Stat(filepath.Join(dir, "sub", "out.txt")); err == nil {
			t.Error("fs.write landed on the TEXTUALLY CLEANED path <dir>/sub/out.txt instead of the canonical <dir>/out.txt")
		}
	})

	// The two directory listers get `<dir>/other`, an EXISTING directory the
	// cleaned form (`<dir>/sub/other`) does not name.
	run("handlers.go fs.listEntries -> listEntries", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "fs.listEntries",
			json.RawMessage(`{"path":`+jsonStr(via("other"))+`}`))
		if err != nil {
			t.Fatalf("fs.listEntries must list the path the guard returned: %v", err)
		}
		var res listEntriesResult
		if err := json.Unmarshal(raw, &res); err != nil {
			t.Fatal(err)
		}
		if res.Path != filepath.Join(dir, "other") {
			t.Errorf("fs.listEntries reported %q, want the canonical %q", res.Path, filepath.Join(dir, "other"))
		}
	})

	run("handlers.go fs.listDir -> listHostDir", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "fs.listDir",
			json.RawMessage(`{"path":`+jsonStr(via("other"))+`}`))
		if err != nil {
			t.Fatalf("fs.listDir must list the path the guard returned: %v", err)
		}
		var res listDirResult
		if err := json.Unmarshal(raw, &res); err != nil {
			t.Fatal(err)
		}
		if res.Path != filepath.Join(dir, "other") {
			t.Errorf("fs.listDir reported %q, want the canonical %q", res.Path, filepath.Join(dir, "other"))
		}
	})

	run("handlers.go search.project -> searchProject(opts.Cwd)", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "search.project",
			json.RawMessage(`{"cwd":`+jsonStr(filepath.Join(dir, "sub", "link")+"/../nope/..")+`,"query":"hello"}`))
		if err != nil {
			t.Fatalf("search.project must search the path the guard returned: %v", err)
		}
		// notes.txt lives in <dir>, not in the cleaned <dir>/sub.
		if !strings.Contains(string(raw), "notes.txt") {
			t.Errorf("search.project found nothing under the canonical cwd: %s", raw)
		}
	})

	run("library.go saveLibrary -> writeFileAtomic", func(t *testing.T) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "library.save",
			json.RawMessage(`{"scope":"project","id":"n","title":"N","kind":"prompt","body":"b","cwd":`+
				jsonStr(filepath.Join(dir, "sub", "link")+"/../nope/..")+`}`))
		if err != nil {
			t.Fatalf("library.save must write under the path the guard returned: %v", err)
		}
		var item libraryItem
		if err := json.Unmarshal(raw, &item); err != nil {
			t.Fatal(err)
		}
		want := filepath.Join(dir, ".workspacer", "library", "n.md")
		if item.Path != want {
			t.Errorf("library.save reported %q, want %q", item.Path, want)
		}
		if _, err := os.Stat(want); err != nil {
			t.Errorf("library.save did not land on the canonical path: %v", err)
		}
	})

	// The claude scope is a SECOND write leg with its own destination, its own
	// guard call and its own derived path.
	run("library.go saveLibraryClaude -> writeFileAtomic", func(t *testing.T) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "library.save",
			json.RawMessage(`{"scope":"claude","id":"s","title":"S","kind":"skill","body":"b","cwd":`+
				jsonStr(filepath.Join(dir, "sub", "link")+"/../nope/..")+`}`))
		if err != nil {
			t.Fatalf("library.save(claude) must write under the path the guard returned: %v", err)
		}
		var item libraryItem
		if err := json.Unmarshal(raw, &item); err != nil {
			t.Fatal(err)
		}
		want := filepath.Join(dir, ".claude", "skills", "s", "SKILL.md")
		if item.Path != want {
			t.Errorf("library.save(claude) reported %q, want %q", item.Path, want)
		}
		if _, err := os.Stat(want); err != nil {
			t.Errorf("library.save(claude) did not land on the canonical path: %v", err)
		}
		if _, err := os.Stat(filepath.Join(dir, "sub", ".claude")); err == nil {
			t.Error("library.save(claude) used the TEXTUALLY CLEANED cwd <dir>/sub")
		}
	})

	run("handlers.go library.list -> listLibrary", func(t *testing.T) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
		item := filepath.Join(dir, ".workspacer", "library", "listed.md")
		if err := os.MkdirAll(filepath.Dir(item), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(item, []byte("---\ntitle: ListedFromCanonical\n---\n\nb\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "library.list",
			json.RawMessage(`{"cwd":`+jsonStr(filepath.Join(dir, "sub", "link")+"/../nope/..")+`}`))
		if err != nil {
			t.Fatalf("library.list must list under the path the guard returned: %v", err)
		}
		// The cleaned cwd is <dir>/sub, which has no library at all.
		if !strings.Contains(string(raw), "ListedFromCanonical") {
			t.Errorf("library.list read from a different directory than the one the guard validated: %s", raw)
		}
	})

	run("handlers.go library.remove -> removeLibrary", func(t *testing.T) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
		victim := filepath.Join(dir, ".workspacer", "library", "gone.md")
		if err := os.MkdirAll(filepath.Dir(victim), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(victim, []byte("---\ntitle: Gone\n---\n\nb\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		reg := registryWithCwd(t, dir)
		if _, err := reg.handle(context.Background(), "library.remove",
			json.RawMessage(`{"scope":"project","id":"gone","cwd":`+
				jsonStr(filepath.Join(dir, "sub", "link")+"/../nope/..")+`}`)); err != nil {
			t.Fatalf("library.remove must unlink under the path the guard returned: %v", err)
		}
		if _, err := os.Stat(victim); err == nil {
			t.Error("library.remove did not unlink the item under the CANONICAL cwd")
		}
	})

	// The fixture's checkUse list is the contract; this table is the proof. If
	// they disagree in either direction, one of them is lying about what is
	// covered.
	fx := loadContractFixture(t)
	var want []string
	for _, e := range fx.CheckUse {
		if e.Owner == fsguardOwnerKey {
			want = append(want, e.CallSites...)
		}
	}
	if len(want) == 0 {
		t.Fatalf("the fixture carries no checkUse callSites for %s — BINDING DECISION 2's record has gone missing", fsguardOwnerKey)
	}
	got := make([]string, 0, len(covered))
	for site := range covered {
		got = append(got, site)
	}
	sort.Strings(got)
	sort.Strings(want)
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("the checkUse call sites and the subtests here have drifted\n  covered: %v\n  fixture: %v", got, want)
	}
}

// TestGuardedHandlersDoNotRenormalizeTheCanonicalPath is the other direction of
// BINDING DECISION 2, and the one that shipped broken: not "does the handler use
// the guard's answer" but "does it use it UNCHANGED".
//
// Every helper in fsops.go used to re-normalize its argument. listHostDir ran
// strings.TrimSpace, then expandTilde, then filepath.Abs (which Cleans);
// listEntries ran Abs too; readTextFile and writeHostFile ran expandTilde. Each
// one is a second, different opinion about what the caller's string means, and
// the guard's whole contract is that there is only one.
//
// A single trailing space was enough. `.. ` is an ordinary component: it does
// not exist, it resolves INSIDE the root, and the guard correctly allows it —
// then TrimSpace turned it back into `..` and Abs collapsed that, so
// `fs.listDir {"path": "<root>/.. "}` listed the root's PARENT. On the real
// browse roots that is `$HOME/.. ` -> /home (every other user's home directory
// name) and `<configDir>/layouts/.. ` -> the config dir itself, the directory
// holding remote-token — while both parents named directly are refused. The same
// trim re-attached a space to a symlink name the guard had just denied.
//
// The corpus pins the guard's half of this (the two trailing-space cases and
// their resolvesTo); this pins the handlers'.
func TestGuardedHandlersDoNotRenormalizeTheCanonicalPath(t *testing.T) {
	dir := t.TempDir()
	outside := t.TempDir()
	if err := os.MkdirAll(filepath.Join(outside, "HIDDEN"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "notes.txt"), []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(dir, "link")); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	parent := filepath.Dir(dir)

	// Each probe is an ALLOWED path (it resolves inside the agent cwd) that
	// names nothing on disk. The handler must therefore fail with ENOENT — the
	// one thing it must never do is trim the space and succeed.
	for _, tc := range []struct {
		method string
		params string
		note   string
	}{
		{"fs.listDir", `{"path":` + jsonStr(dir+"/.. ") + `}`,
			"trimming makes this the parent of the only allowed root"},
		{"fs.listDir", `{"path":` + jsonStr(dir+"/link ") + `}`,
			"trimming makes this the symlink out of the root, which the guard denies by name"},
		{"fs.listEntries", `{"path":` + jsonStr(dir+"/link ") + `}`,
			"same trim, other lister"},
		{"fs.read", `{"path":` + jsonStr(dir+"/notes.txt ") + `}`,
			"trimming makes this an existing file the caller did not name"},
	} {
		t.Run(tc.method+" "+tc.note, func(t *testing.T) {
			reg := registryWithCwd(t, dir)
			raw, err := reg.handle(context.Background(), tc.method, json.RawMessage(tc.params))
			if err == nil {
				t.Fatalf("%s succeeded on a path that does not exist — the handler re-normalized the guard's answer and opened something else: %s", tc.method, raw)
			}
			if strings.Contains(err.Error(), refusalText) {
				t.Fatalf("%s was refused by the GUARD (%v); the point of this case is that the guard allows the string and the handler must then fail to open it", tc.method, err)
			}
		})
	}

	// fs.write is the one that must SUCCEED, on the literal name with the space.
	t.Run("fs.write keeps the trailing space instead of clobbering the neighbour", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		if _, err := reg.handle(context.Background(), "fs.write",
			json.RawMessage(`{"path":`+jsonStr(dir+"/w.txt ")+`,"contents":"spaced"}`)); err != nil {
			t.Fatalf("writing a filename that ends in a space is legal: %v", err)
		}
		if body, err := os.ReadFile(filepath.Join(dir, "w.txt ")); err != nil || string(body) != "spaced" {
			t.Errorf("fs.write did not land on the canonical %q: %q %v", dir+"/w.txt ", body, err)
		}
		if _, err := os.Stat(filepath.Join(dir, "w.txt")); err == nil {
			t.Error("fs.write trimmed the space and wrote a DIFFERENT file")
		}
	})

	// The escalation the trim actually bought: fs.listDir runs on browseRoots, so
	// "<store>/.. " is the config dir — which is denied when it is named directly.
	t.Run("fs.listDir cannot reach the config dir through a store", func(t *testing.T) {
		cfgHome := t.TempDir()
		t.Setenv("XDG_CONFIG_HOME", cfgHome)
		t.Setenv("APPDATA", cfgHome)
		cfg := configDir()
		if err := os.MkdirAll(filepath.Join(cfg, "layouts"), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(cfg, "remote-token"), []byte("tok"), 0o600); err != nil {
			t.Fatal(err)
		}

		reg := registryWithCwd(t, dir)
		if _, err := reg.handle(context.Background(), "fs.listDir",
			json.RawMessage(`{"path":`+jsonStr(cfg)+`}`)); err == nil {
			t.Fatal("naming the config dir directly must be refused — the premise of this case")
		}
		reg = registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "fs.listDir",
			json.RawMessage(`{"path":`+jsonStr(filepath.Join(cfg, "layouts")+"/.. ")+`}`))
		if err == nil {
			t.Fatalf("fs.listDir listed the config dir through a trailing space: %s", raw)
		}
	})

	// And the plain statement of the whole test, in case a future refactor makes
	// the calls above fail for some other reason.
	t.Run("no handler ever reports the parent of a root", func(t *testing.T) {
		reg := registryWithCwd(t, dir)
		raw, err := reg.handle(context.Background(), "fs.listDir",
			json.RawMessage(`{"path":`+jsonStr(dir+"/.. ")+`}`))
		if err != nil {
			return
		}
		var res listDirResult
		if err := json.Unmarshal(raw, &res); err != nil {
			t.Fatal(err)
		}
		if res.Path == parent {
			t.Fatalf("fs.listDir returned the parent %q of the only allowed root %q", parent, dir)
		}
	})
}

// TestBrainRefusesCaseVariantDuplicateParamKeys pins the decoder half of the
// case-variant-key bypass.
//
// encoding/json matches a struct field to a JSON key exactly if it can and
// CASE-INSENSITIVELY if it cannot, so `{"path":a,"Path":b}` decodes to b — while
// the bus's grant confinement reads the map key "path" byte-exactly and sees a.
// One request, two paths, and the plugin's fsRoots scope confined the wrong one:
// a plugin granted only its own directory got read and write over every live
// agent cwd and over <configDir>/library|layouts|sessions.
//
// The bus refuses the shape (contracts paramShapes), and so does this decoder,
// which is what makes it hold for a TRUSTED connection and for every params
// field rather than only the one capspec classified.
func TestBrainRefusesCaseVariantDuplicateParamKeys(t *testing.T) {
	dir := t.TempDir()
	victim := t.TempDir()
	if err := os.WriteFile(filepath.Join(victim, "loot.txt"), []byte("SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "ok.txt"), []byte("benign"), 0o644); err != nil {
		t.Fatal(err)
	}

	// Both directories are live agent cwds, so containment alone would allow
	// either one: the question here is purely which STRING the handler reads.
	reg := registryWithCwds(t, dir, victim)
	raw, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(filepath.Join(dir, "ok.txt"))+
			`,"Path":`+jsonStr(filepath.Join(victim, "loot.txt"))+`}`))
	if err == nil {
		t.Fatalf("an ambiguous params object must be refused, got %s", raw)
	}
	if !strings.Contains(err.Error(), "differ only by case") {
		t.Errorf("refused for the wrong reason: %v", err)
	}

	// The unambiguous request is unaffected.
	reg = registryWithCwds(t, dir, victim)
	if _, err := reg.handle(context.Background(), "fs.read",
		json.RawMessage(`{"path":`+jsonStr(filepath.Join(dir, "ok.txt"))+`}`)); err != nil {
		t.Fatalf("the ordinary single-key call must still work: %v", err)
	}
}

// assertNoResidualToken is the fixture's own spell-checker, and it is not
// paranoia: every substituter in every loader passes an UNRECOGNISED ${TOKEN}
// through verbatim, and the result is then a RELATIVE string, which every copy
// refuses because it is not absolute. So a one-character typo in a token —
// ${CONFI} for ${CONFIG} — turns a deny case into a case that passes while
// exercising nothing, silently and in all three languages at once. Applying that
// to the config-dir-canonicalization case defanged the only guard on that axis
// and a lexical config dir went green everywhere; applying it to all 64 deny
// targets left every suite 100% green. Allow cases are immune (a bogus target
// fails resolvesTo), so this is the negative half's only protection.
func assertNoResidualToken(t *testing.T, s string) {
	t.Helper()
	if i := strings.Index(s, "${"); i >= 0 {
		end := strings.Index(s[i:], "}")
		tok := s[i:]
		if end >= 0 {
			tok = s[i : i+end+1]
		}
		t.Fatalf("unsubstituted token %s in %q — the token set is DECLARED in the fixture's `vocabulary.tokens` block and closed by TestFixtureVocabularyIsClosed; an undeclared one passes through verbatim and silently defangs the case", tok, s)
	}
}
