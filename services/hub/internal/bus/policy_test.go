package bus

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// withDeadline runs one guard call and fails the test rather than hanging on it.
// A goroutine spinning on a symlink cycle cannot be cancelled, so it is left to
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
		t.Fatalf("the walk did not return within 10s — a symlink cycle spun it (the hop counter is the only ELOOP bound this code has)")
		return zero, nil
	}
}

// mustCanon canonicalizes a root the way grant registration does, so tests
// compare canonical-to-canonical exactly like the live path.
func mustCanon(t *testing.T, p string) string {
	t.Helper()
	c, err := canonicalize(p)
	if err != nil {
		t.Fatalf("canonicalize(%q): %v", p, err)
	}
	return c
}

func TestPathWithinRoots(t *testing.T) {
	base := t.TempDir()
	root := filepath.Join(base, "project")
	sub := filepath.Join(root, "src")
	outside := filepath.Join(base, "secrets")
	for _, d := range []string{root, sub, outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(sub, "main.go"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(outside, "creds"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	canonRoot := mustCanon(t, root)

	cases := []struct {
		name   string
		target string
		want   bool
	}{
		{"existing file inside root", filepath.Join(sub, "main.go"), true},
		{"exact root", root, true},
		{"nested dir inside root", sub, true},
		{"new file under existing subdir (write target)", filepath.Join(sub, "new.txt"), true},
		{"new file via new nested dirs inside root", filepath.Join(root, "a", "b", "c.txt"), true},
		{"traversal escapes root", filepath.Join(root, "..", "secrets", "creds"), false},
		{"absolute path outside root", filepath.Join(outside, "creds"), false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, err := pathWithinRoots([]string{canonRoot}, c.target)
			if err != nil {
				t.Fatalf("pathWithinRoots error: %v", err)
			}
			if got != c.want {
				t.Errorf("pathWithinRoots(%q) = %v, want %v", c.target, got, c.want)
			}
		})
	}
}

// A symlink inside the root that points outside must not let a target reached
// through it escape — canonicalize resolves the link before the prefix check.
func TestPathWithinRoots_SymlinkEscape(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink semantics differ on Windows")
	}
	base := t.TempDir()
	root := filepath.Join(base, "project")
	outside := filepath.Join(base, "secrets")
	for _, d := range []string{root, outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(outside, "creds"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	// root/escape -> ../secrets
	link := filepath.Join(root, "escape")
	if err := os.Symlink(outside, link); err != nil {
		t.Fatal(err)
	}

	canonRoot := mustCanon(t, root)
	// Reading "inside" the root but through the symlink lands on the secret.
	got, err := pathWithinRoots([]string{canonRoot}, filepath.Join(link, "creds"))
	if err != nil {
		t.Fatalf("pathWithinRoots error: %v", err)
	}
	if got {
		t.Errorf("symlink escape was allowed: %q resolved inside %q", filepath.Join(link, "creds"), canonRoot)
	}
}

// A root whose name is a string prefix of a sibling must not capture the sibling.
func TestWithin_SiblingPrefixIsNotContained(t *testing.T) {
	root := filepath.FromSlash("/srv/foo")
	if within(root, filepath.FromSlash("/srv/foobar/x")) {
		t.Errorf("sibling /srv/foobar wrongly treated as inside /srv/foo")
	}
	if !within(root, filepath.FromSlash("/srv/foo/x")) {
		t.Errorf("/srv/foo/x should be inside /srv/foo")
	}
	if !within(root, root) {
		t.Errorf("root should contain itself")
	}
}

func TestPathWithinRoots_MultipleRoots(t *testing.T) {
	base := t.TempDir()
	a := filepath.Join(base, "a")
	b := filepath.Join(base, "b")
	other := filepath.Join(base, "c")
	for _, d := range []string{a, b, other} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	roots := []string{mustCanon(t, a), mustCanon(t, b)}

	if ok, _ := pathWithinRoots(roots, filepath.Join(b, "f.txt")); !ok {
		t.Errorf("target under second root should be allowed")
	}
	if ok, _ := pathWithinRoots(roots, filepath.Join(other, "f.txt")); ok {
		t.Errorf("target under an ungranted root should be denied")
	}
}

func TestParamString(t *testing.T) {
	cases := []struct {
		name   string
		params string
		field  string
		want   string
		ok     bool
	}{
		{"present", `{"path":"/a/b"}`, "path", "/a/b", true},
		{"cwd field", `{"cwd":"/proj","query":"x"}`, "cwd", "/proj", true},
		{"absent", `{"query":"x"}`, "path", "", false},
		{"empty string", `{"path":""}`, "path", "", false},
		{"wrong type", `{"path":123}`, "path", "", false},
		{"malformed", `{not json`, "path", "", false},
		{"empty params", ``, "path", "", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, ok := paramString(json.RawMessage(c.params), c.field)
			if got != c.want || ok != c.ok {
				t.Errorf("paramString(%s, %q) = (%q, %v), want (%q, %v)", c.params, c.field, got, ok, c.want, c.ok)
			}
		})
	}
}

// TestWithin_FilesystemRootContainsEverything covers a grant whose canonical
// root is the filesystem root ("/"): it declares the whole tree, so within()
// must treat every absolute path as contained. Concatenating an extra separator
// ("//") would match nothing, silently denying the grant. Covers idx 20.
func TestWithin_FilesystemRootContainsEverything(t *testing.T) {
	root := string(os.PathSeparator) // "/" on Unix, "\" on Windows
	target := filepath.Join(root, "home", "u", "f")
	if !within(root, target) {
		t.Errorf("root %q (whole filesystem) should contain %q", root, target)
	}
	if !within(root, filepath.Join(root, "etc", "x")) {
		t.Errorf("root %q should contain %q", root, filepath.Join(root, "etc", "x"))
	}
}

// --- contracts/path-containment-cases.json ----------------------------------
//
// The same fixture drives the brain (cmd/brain/fsguard_test.go) and the desktop
// (main/lib/pathConfinement.test.ts). Three implementations of one predicate:
// this loader is what stops them drifting apart case by case. Every case whose
// `group` this file owns must produce the fixture's verdict here, unchanged.

type containmentTree struct {
	Dirs     []string          `json:"dirs"`
	Files    map[string]string `json:"files"`
	Symlinks map[string]string `json:"symlinks"`
	// RelativeSymlinks values are written VERBATIM as the link body — the only
	// way to reach the walk's relative-link arm, which every absolutized
	// `Symlinks` case leaves unexecuted.
	RelativeSymlinks map[string]string `json:"relativeSymlinks"`
	Modes            map[string]string `json:"modes"`
}

type containmentCase struct {
	Name               string          `json:"name"`
	Group              string          `json:"group"`
	NeedsSymlinks      bool            `json:"needsSymlinks"`
	PosixOnly          bool            `json:"posixOnly"`
	NeedsUnreadableDir bool            `json:"needsUnreadableDir"`
	NeedsHome          bool            `json:"needsHome"`
	Tree               containmentTree `json:"tree"`
	// ConfigDirVia names a sandbox-relative symlink to the config HOME that the
	// implementation is pointed through, while ${CONFIG} keeps naming the real
	// path — so the case passes only if the gate resolves its own config dir.
	ConfigDirVia string   `json:"configDirVia"`
	Roots        []string `json:"roots"`
	Target       string   `json:"target"`
	Expect       string   `json:"expect"`
	// ResolvesTo is the token-substituted path canonicalize() must produce on an
	// allow. The bus never opens it (authorize only decides), but it computes
	// the same walk, and a copy whose ANSWER drifts is a copy whose verdict will
	// drift next. Mandatory on every allow case.
	ResolvesTo string `json:"resolvesTo"`
	Why        string `json:"why"`
}

type paramShapeCase struct {
	Name         string          `json:"name"`
	Field        string          `json:"field"`
	ParamsAbsent bool            `json:"paramsAbsent"`
	Params       json.RawMessage `json:"params"`
	Expect       string          `json:"expect"`
}

// methodCase is the fixture's second half: not "is the predicate right" but
// "does every path-bearing capability actually reach it". The bus owns the
// vocabulary side of that (capspec.PathParam) — which method is path-scoped and
// which params field carries the path. rootSet is the providers' half (spec
// 10.1/10.2); the bus never supplies workspace/browse roots, it uses the
// plugin's granted roots (spec 10.4), so nothing here asserts on it beyond it
// naming a set that exists.
type methodCase struct {
	Method    string   `json:"method"`
	Field     string   `json:"field"`
	RootSet   string   `json:"rootSet"`
	Providers []string `json:"providers"`
}

// asciiFoldCase is one in/out vector of the fixture's `asciiFold` block.
type asciiFoldCase struct {
	In  string `json:"in"`
	Out string `json:"out"`
}

type asciiFoldBlock struct {
	Cases []asciiFoldCase `json:"cases"`
}

type containmentFixture struct {
	Owners             map[string][]string `json:"owners"`
	SecretBasenames    []string            `json:"secretBasenames"`
	ConfigStoreSubdirs []string            `json:"configStoreSubdirs"`
	AsciiFold          asciiFoldBlock      `json:"asciiFold"`
	Cases              []containmentCase   `json:"cases"`
	Methods            []methodCase        `json:"methods"`
	ParamShapes        []paramShapeCase    `json:"paramShapes"`
}

// TestAsciiFoldMatchesTheFixture pins the bus copy of asciiLower against the
// same vectors the brain and the desktop are held to. The function's whole
// reason to exist is that the three copies fold IDENTICALLY, and until this
// block nothing distinguished it from strings.ToLower.
func TestAsciiFoldMatchesTheFixture(t *testing.T) {
	fx := loadContainmentFixture(t)
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

// thisOwner is the key this file answers to in the fixture's `owners` block.
const thisOwner = "services/hub/internal/bus/policy.go"

// containedByAny is the roots half of pathWithinRoots with the secret gate left
// out, so a test can say which of the two refusal arms a deny had to come from.
// It exists only for that assertion and is never a second opinion on the verdict.
func containedByAny(roots []string, canonicalTarget string) bool {
	for _, r := range roots {
		if within(r, canonicalTarget) {
			return true
		}
	}
	return false
}

// TestSecretGateConstantsMatchTheFixture pins the two parts of the `secrets`
// gate the CASES cannot reach: every secrets case names one of the two
// credential basenames and one of the three stores, so a THIRD basename, or a
// fourth store carve-out, keeps the whole corpus green while the copies drift.
// Carving out `plugins` in particular is a live widening — pathIsSecret flips
// from true to false for <configDir>/plugins/**, which is where every installed
// plugin keeps its .bus-token and settings.
//
// The fixture has carried both lists all along and only the TypeScript loader
// consumed them; this struct did not even declare the fields, so the bus copy
// could drift with the whole hub suite green.
func TestSecretGateConstantsMatchTheFixture(t *testing.T) {
	fx := loadContainmentFixture(t)
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
	// Order too: the brain joins these onto the config dir in this order and the
	// desktop pins the order with toEqual, so all three are one list.
	if strings.Join(configStoreSubdirs, ",") != strings.Join(fx.ConfigStoreSubdirs, ",") {
		t.Errorf("configStoreSubdirs drifted from the fixture\n  got:  %v\n  want: %v", configStoreSubdirs, fx.ConfigStoreSubdirs)
	}
}

// TestTheSecretGateCarvesOutExactlyTheFixturesStores is the BEHAVIOURAL half.
//
// The test above pins the CONTENTS of configStoreSubdirs. Nothing pinned that
// pathIsSecret iterates that same slice — the gate holds its own loop, so a
// hardcoded wider set there (a fourth entry for `plugins`) re-admits
// <configDir>/plugins/** with the constant, the fixture and every case green.
// Each secrets case names one of the three real stores, so a gate with FOUR
// carve-outs satisfies all of them.
func TestTheSecretGateCarvesOutExactlyTheFixturesStores(t *testing.T) {
	fx := loadContainmentFixture(t)
	if len(fx.ConfigStoreSubdirs) == 0 {
		t.Fatal("the fixture must carry configStoreSubdirs")
	}
	configHome := realpathOf(t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", configHome)
	if runtime.GOOS == "windows" {
		t.Setenv("APPDATA", configHome)
	}
	cfg := filepath.Join(configHome, "workspacer")
	if err := os.MkdirAll(cfg, 0o755); err != nil {
		t.Fatal(err)
	}

	exempt := map[string]bool{}
	for _, store := range fx.ConfigStoreSubdirs {
		exempt[store] = true
		if err := os.MkdirAll(filepath.Join(cfg, store), 0o755); err != nil {
			t.Fatal(err)
		}
		if pathIsSecret(filepath.Join(cfg, store, "item.md")) {
			t.Errorf("the fixture carves out %q but the gate still refuses it", store)
		}
	}
	for _, name := range []string{"plugins", "cache", "logs", "handoffs", "backups", "supervisor"} {
		if exempt[name] {
			continue
		}
		if err := os.MkdirAll(filepath.Join(cfg, name), 0o755); err != nil {
			t.Fatal(err)
		}
		if !pathIsSecret(filepath.Join(cfg, name, "anything.json")) {
			t.Errorf("the gate exempts <configDir>/%s, which the fixture does not list", name)
		}
	}
	if !pathIsSecret(filepath.Join(cfg, "remote-token")) {
		t.Error("remote-token is not refused — the gate is not running at all")
	}
}

// userHome / realpathOf feed the two out-of-sandbox tokens. The RESOLVED form
// matters: every other path in this loader is realpath'd, and on macOS /var ->
// /private/var alone would make the comparison wrong.
func userHome() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return home
}

func realpathOf(p string) string {
	if p == "" {
		return ""
	}
	if real, err := filepath.EvalSymlinks(p); err == nil {
		return real
	}
	return p
}

// TestEveryCorpusCaseBelongsToAGroupSOMEBODYOwns closes the complement of the
// `seen[g] == 0` floor below: every loader guards against running ZERO cases for
// a group it owns, and nothing guarded that every case belongs to a group
// somebody owns. Both Go loaders filter with `if !groups[c.Group] { continue }`,
// so a one-character typo in a case's `group` drops it from this copy and from
// the brain — the one that actually answers fs.*/library.* — with every suite
// green, while the TypeScript loader keeps running it. Twin of the brain's test
// of the same name.
func TestEveryCorpusCaseBelongsToAGroupSOMEBODYOwns(t *testing.T) {
	fx := loadContainmentFixture(t)
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
			t.Errorf("case %q is in group %q, which appears in NO owner's list — every Go loader skips it silently", c.Name, c.Group)
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

func loadContainmentFixture(t *testing.T) containmentFixture {
	t.Helper()
	path := filepath.Join("..", "..", "..", "..", "contracts", "path-containment-cases.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	var fx containmentFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	return fx
}

// materializeTree builds one case's sandbox: dirs, then files (creating missing
// parents), then symlinks, then modes — in exactly that order, so a 0-mode
// directory cannot block the rest of the setup. Symlink targets in the fixture
// are sandbox-relative and are made absolute here, so no case depends on
// relative-link resolution.
func materializeTree(t *testing.T, sandbox string, tr containmentTree, needsSymlinks bool) {
	t.Helper()
	for _, d := range tr.Dirs {
		if err := os.MkdirAll(filepath.Join(sandbox, filepath.FromSlash(d)), 0o755); err != nil {
			t.Fatalf("mkdir %s: %v", d, err)
		}
	}
	for p, body := range tr.Files {
		full := filepath.Join(sandbox, filepath.FromSlash(p))
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatalf("mkdir parent of %s: %v", p, err)
		}
		if err := os.WriteFile(full, []byte(body), 0o644); err != nil {
			t.Fatalf("write %s: %v", p, err)
		}
	}
	for link, target := range tr.Symlinks {
		full := filepath.Join(sandbox, filepath.FromSlash(link))
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatalf("mkdir parent of %s: %v", link, err)
		}
		if err := os.Symlink(filepath.Join(sandbox, filepath.FromSlash(target)), full); err != nil {
			if needsSymlinks {
				t.Skipf("cannot create symlinks here: %v", err)
			}
			t.Fatalf("symlink %s -> %s: %v", link, target, err)
		}
	}
	for link, target := range tr.RelativeSymlinks {
		full := filepath.Join(sandbox, filepath.FromSlash(link))
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatalf("mkdir parent of %s: %v", link, err)
		}
		// Verbatim: resolving a relative link body is the implementation's job.
		if err := os.Symlink(filepath.FromSlash(target), full); err != nil {
			if needsSymlinks {
				t.Skipf("cannot create symlinks here: %v", err)
			}
			t.Fatalf("symlink %s -> %s: %v", link, target, err)
		}
	}
	for p, mode := range tr.Modes {
		full := filepath.Join(sandbox, filepath.FromSlash(p))
		m, err := strconv.ParseUint(mode, 8, 32)
		if err != nil {
			t.Fatalf("bad mode %q for %s: %v", mode, p, err)
		}
		// Restored before the sandbox is torn down, or TempDir's cleanup cannot
		// remove a 0-mode directory. Cleanups run LIFO and TempDir registered
		// its own first, so this one runs before it.
		t.Cleanup(func() { _ = os.Chmod(full, 0o700) })
		if err := os.Chmod(full, os.FileMode(m)); err != nil {
			t.Fatalf("chmod %s: %v", p, err)
		}
	}
}

// TestPathContainmentContractCases runs every fixture case in a group this file
// owns. Roots are pre-canonicalized because on the bus they always are: the
// grant registration path (canonRoots) canonicalizes once and DISCARDS anything
// that cannot be — so a fixture root that fails to canonicalize is omitted from
// the list here rather than failing the test, which is exactly what the "empty
// root among valid ones" and "relative root" cases are asserting.
func TestPathContainmentContractCases(t *testing.T) {
	fx := loadContainmentFixture(t)

	groups := fx.Owners[thisOwner]
	if len(groups) == 0 {
		t.Fatalf("fixture owners has no entry for %s", thisOwner)
	}
	owned := map[string]bool{}
	for _, g := range groups {
		owned[g] = true
	}
	// Both groups are mandatory. If a future edit narrows the owners block, this
	// fails loudly instead of silently skipping half the corpus.
	for _, required := range []string{"containment", "secrets"} {
		if !owned[required] {
			t.Fatalf("owners[%s] must include %q (this file implements it); got %v", thisOwner, required, groups)
		}
	}

	seen := map[string]int{}
	for _, c := range fx.Cases {
		if !owned[c.Group] {
			continue
		}
		seen[c.Group]++
		t.Run(c.Group+"/"+c.Name, func(t *testing.T) {
			if c.PosixOnly && runtime.GOOS == "windows" {
				t.Skip("posixOnly: the '/' filesystem-root branch has no portable spelling")
			}
			if c.NeedsSymlinks && runtime.GOOS == "windows" {
				t.Skip("needsSymlinks: symlink creation needs developer mode on Windows")
			}
			if c.NeedsUnreadableDir && (runtime.GOOS == "windows" || os.Geteuid() == 0) {
				t.Skip("needsUnreadableDir: this process can read a 0o000 directory anyway")
			}
			// ${HOME} and ${PROCESS_CWD} deliberately leave the sandbox: they are
			// the two places a re-introduced tilde expansion, or a bad root
			// resolved against the process cwd, would actually LAND. Every case
			// keeping both root and target inside one temp dir is exactly what
			// made the tilde and bad-root cases vacuous here — a widened root
			// still contained nothing, so the deny verdict never moved. No case
			// using these tokens expects `allow`.
			home := realpathOf(userHome())
			if c.NeedsHome && home == "" {
				t.Skip("needsHome: this process has no resolvable home directory")
			}
			processCwd := ""
			if wd, err := os.Getwd(); err == nil {
				processCwd = realpathOf(wd)
			}

			sandbox, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatalf("realpath sandbox: %v", err)
			}
			root := filepath.Join(sandbox, "root")
			outside := filepath.Join(sandbox, "outside")
			configHome := filepath.Join(sandbox, "config")
			config := filepath.Join(configHome, "workspacer")
			for _, d := range []string{root, outside, config} {
				if err := os.MkdirAll(d, 0o755); err != nil {
					t.Fatalf("mkdir %s: %v", d, err)
				}
			}
			// A case may ask for the config dir to be REACHED through a symlink,
			// while ${CONFIG} keeps naming the real path: the two agree only if
			// the gate canonicalizes its own config dir.
			configEnv := configHome
			if c.ConfigDirVia != "" {
				link := filepath.Join(sandbox, filepath.FromSlash(c.ConfigDirVia))
				if err := os.Symlink(configHome, link); err != nil {
					t.Skipf("configDirVia: cannot create symlinks here: %v", err)
				}
				configEnv = link
			}
			// The secret gate resolves the config dir through authtoken.ConfigDir()
			// at call time, so pointing the environment at the sandbox is what puts
			// ${CONFIG} in scope.
			t.Setenv("XDG_CONFIG_HOME", configEnv)
			if runtime.GOOS == "windows" {
				t.Setenv("APPDATA", configEnv)
			}

			materializeTree(t, sandbox, c.Tree, c.NeedsSymlinks)

			replacer := strings.NewReplacer(
				"${SANDBOX}", sandbox,
				"${ROOT}", root,
				"${OUTSIDE}", outside,
				"${CONFIG}", config,
				"${HOME}", home,
				"${PROCESS_CWD}", processCwd,
			)
			// An UNRECOGNISED ${TOKEN} is passed through verbatim by every
			// substituter in every loader, and the result is then a relative
			// string that all three copies refuse for being non-absolute. So a
			// one-character typo turns a deny case into a case that passes while
			// exercising nothing — in all three languages at once. Fail on it.
			sub := func(in string) string {
				out := replacer.Replace(in)
				if i := strings.Index(out, "${"); i >= 0 {
					t.Fatalf("unsubstituted token in %q — the vocabulary is ${SANDBOX} ${ROOT} ${OUTSIDE} ${CONFIG} ${HOME} ${PROCESS_CWD}", out[i:])
				}
				return out
			}
			roots := make([]string, 0, len(c.Roots))
			for _, r := range c.Roots {
				cr, ok := canonicalizeRoot(sub(r))
				if !ok {
					continue // DISCARD: skip this root, never abort the check
				}
				roots = append(roots, cr)
			}
			target := sub(c.Target)

			// The fixture's verdict is allow/deny only: conn.authorize denies on
			// ANY non-nil error (unresolvable path, or the secret sentinel), so
			// `ok` is the whole of the shared contract. The error is inspected
			// below purely for the bus-local question the fixture cannot ask.
			// WATCHDOG: the walk is hand-rolled and never reaches the platform's
			// ELOOP, so a hop-counter regression hangs rather than fails. The two
			// relative-cycle cases would otherwise take down the whole package on
			// the 10-minute test timeout instead of naming themselves.
			ok, err := withDeadline(t, func() (bool, error) { return pathWithinRoots(roots, target) })
			want := c.Expect == "allow"
			if ok != want {
				t.Errorf("pathWithinRoots(%v, %q) = %v, want %v\ncase: %s\nwhy: %s",
					roots, target, ok, want, c.Name, c.Why)
			}
			// The ANSWER, not only the verdict. The corpus used to pin the
			// decision alone, which is half the predicate: a walk that produced
			// a textually-cleaned path instead of the resolved one (the thing
			// canonicalize()'s header forbids) satisfies every allow and every
			// deny here while naming a different file. On the bus that string is
			// never opened — but it is the same walk the brain opens with, and a
			// divergence in it is the drift this fixture exists to catch.
			if want {
				if c.ResolvesTo == "" {
					t.Fatalf("allow case %q carries no resolvesTo; every allow case must pin the path the walk produces", c.Name)
				}
				ct, err := canonicalize(target)
				if err != nil {
					t.Fatalf("allow case %q: canonicalize(%q) failed: %v", c.Name, target, err)
				}
				if expect := filepath.FromSlash(sub(c.ResolvesTo)); ct != expect {
					t.Errorf("canonicalize(%q) = %q, want %q\ncase: %s\nwhy: %s", target, ct, expect, c.Name, c.Why)
				}
			}
			// Bus-only strengthening, not part of the shared corpus. The bus
			// carries TWO refusals: the containment one may echo the plugin's own
			// requested path, the secret one must not (spec 7.5), and they are
			// told apart by the errSecretPath sentinel. Which arm a deny SHOULD
			// take is derivable without a second opinion on the verdict: if the
			// resolved target sits inside a granted root, containment passed and
			// only the gate can have refused it. Without this, a regression that
			// collapsed the two arms back into one path-echoing message would
			// leave every case in the corpus green.
			//
			// Note the fixture's `group` is NOT that discriminator: "a symlink
			// out of an allowed root into the config dir" is a secrets case that
			// resolves clean OUT of the only granted root, so on the bus it is
			// refused by containment before the gate is ever consulted.
			if !want {
				ct, cerr := canonicalize(target)
				switch {
				case cerr != nil:
					if err == nil {
						t.Errorf("target did not canonicalize (%v) but the refusal carried no error\ncase: %s", cerr, c.Name)
					}
				case containedByAny(roots, ct):
					if !errors.Is(err, errSecretPath) {
						t.Errorf("%q resolved inside a granted root, so only the secret gate can refuse it; "+
							"got err = %v, want errSecretPath (the non-echoing refusal)\ncase: %s", ct, err, c.Name)
					}
				default:
					if err != nil {
						t.Errorf("%q resolved outside every granted root, so this is the containment refusal "+
							"(which keeps its own path-echoing wording); got err = %v, want nil\ncase: %s", ct, err, c.Name)
					}
				}
			}
		})
	}

	for _, g := range []string{"containment", "secrets"} {
		if seen[g] == 0 {
			t.Errorf("fixture contributed no cases for owned group %q", g)
		}
	}
}

// TestFixtureMethodsMatchCapspec is the guard the fixture's header says exists:
// "its `method` set must equal capspec.PathParam exactly — a Go test asserts
// that". It lives here because capspec is the bus's vocabulary (the bus is what
// refuses to grant a path-scoped method to a plugin that declared no roots), and
// because nothing else in the repo reads the corpus's `methods` block: cmd/brain
// and internal/capspec have their own guards, but both compare code to code.
//
// Both directions matter. A new PathParam entry with no corpus entry ships a
// path capability nobody wrote a containment expectation for; a corpus entry
// with no PathParam entry describes confinement the bus does not actually apply.
func TestFixtureMethodsMatchCapspec(t *testing.T) {
	fx := loadContainmentFixture(t)
	if len(fx.Methods) == 0 {
		t.Fatal("fixture has no methods block")
	}

	inFixture := map[string]methodCase{}
	for _, m := range fx.Methods {
		if _, dup := inFixture[m.Method]; dup {
			t.Errorf("fixture lists %q twice in methods", m.Method)
		}
		inFixture[m.Method] = m
	}

	for method, m := range inFixture {
		field, scoped := capspec.PathParam[method]
		if !scoped {
			t.Errorf("fixture methods names %q, which capspec.PathParam does not consider path-scoped — "+
				"the bus grants it with no root confinement, so the corpus is describing a guard that never runs", method)
			continue
		}
		// The field is which params key the guard reads. Disagreeing on it is
		// the silent version of no guard at all: the bus would look up a key the
		// caller never sends, paramString would return ok=false, and every call
		// would deny — or worse, the corpus pins the wrong key and the real one
		// goes unchecked.
		if m.Field != field {
			t.Errorf("fixture says %q carries its path in %q; capspec.PathParam says %q", method, m.Field, field)
		}
		// rootSet is the providers' half of the contract (spec 10.1/10.2) and
		// the bus never supplies those lists — but a typo here would quietly
		// mean nothing to every loader, so pin the vocabulary at least.
		if m.RootSet != "workspace" && m.RootSet != "browse" {
			t.Errorf("fixture rootSet for %q is %q, want \"workspace\" or \"browse\"", method, m.RootSet)
		}
		if len(m.Providers) == 0 {
			t.Errorf("fixture entry for %q names no providers — nothing says which side must enforce it", method)
		}
	}

	for method := range capspec.PathParam {
		if _, ok := inFixture[method]; !ok {
			t.Errorf("capspec.PathParam has %q but the fixture's methods block does not — "+
				"a path-scoped capability shipped without a containment expectation", method)
		}
	}
}

// TestParamShapeContractCases drives the fixture's paramShapes block through
// paramString — the gate that runs BEFORE containment. Every failure mode must
// deny rather than fall through to an unrelated top-level key or to an empty
// string that would resolve against the daemon's working directory.
func TestParamShapeContractCases(t *testing.T) {
	fx := loadContainmentFixture(t)
	if len(fx.ParamShapes) == 0 {
		t.Fatal("fixture has no paramShapes cases")
	}
	for _, c := range fx.ParamShapes {
		t.Run(c.Name, func(t *testing.T) {
			params := c.Params // verbatim JSON, exactly as the fixture spells it
			if c.ParamsAbsent {
				params = nil // a call that carried no params at all
			}
			_, ok := paramString(params, c.Field)
			want := c.Expect == "accept"
			if ok != want {
				t.Errorf("paramString(%s, %q) ok = %v, want %v", string(params), c.Field, ok, want)
			}
		})
	}
}

// TestAuthorizeRefusalMessages pins the two refusal wordings apart (spec 7.5).
// The containment arm may name the plugin's own requested path — a grant's scope
// is the plugin's install-time consented data. The SECRET arm must not: it goes
// to a remote caller, and confirming that a denied path hit something worth
// protecting is a probe primitive.
func TestAuthorizeRefusalMessages(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	configHome := filepath.Join(sandbox, "config")
	config := filepath.Join(configHome, "workspacer")
	if err := os.MkdirAll(config, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config, "remote-token"), []byte("tok"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", configHome)
	if runtime.GOOS == "windows" {
		t.Setenv("APPDATA", configHome)
	}

	// A grant wide enough to re-admit the config dir — exactly what an absolute
	// manifest scope (or an agent cwd, on the brain side) can look like.
	canonRoot, ok := canonicalizeRoot(sandbox)
	if !ok {
		t.Fatalf("sandbox root should canonicalize")
	}
	cn := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{canonRoot}}}}

	secret := filepath.Join(config, "remote-token")
	params, _ := json.Marshal(map[string]string{"path": secret})
	err = cn.authorize("fs.read", params)
	if err == nil {
		t.Fatalf("reading the bus credential through a wide grant must be refused")
	}
	if want := "fs.read: path is outside the allowed workspace (agent cwds + config stores)"; err.Error() != want {
		t.Errorf("secret refusal = %q, want %q", err.Error(), want)
	}
	if strings.Contains(err.Error(), secret) || strings.Contains(err.Error(), config) {
		t.Errorf("secret refusal must not echo the path: %q", err.Error())
	}

	// The containment arm is unchanged and still names the requested path.
	outside := filepath.Join(t.TempDir(), "elsewhere.txt")
	params, _ = json.Marshal(map[string]string{"path": outside})
	err = cn.authorize("fs.read", params)
	if err == nil {
		t.Fatalf("a path outside every root must be refused")
	}
	if !strings.Contains(err.Error(), "outside the plugin's granted scope") {
		t.Errorf("containment refusal = %q, want the granted-scope wording", err.Error())
	}
}

// TestAuthorizeRefusesCaseVariantDuplicateOfTheScopedField is the grant-scoping
// half of the case-variant-key bypass, at the layer that actually decides.
//
// paramString's lookup is byte-exact. A Go provider's struct decode is not:
// encoding/json matches a field to a JSON key exactly if it can and
// CASE-INSENSITIVELY if it cannot, so the later "Path" overwrote the "path" this
// file had confined. authorize() therefore returned nil on
// `{"path":"<grantRoot>/ok.txt","Path":"<victim>/loot.txt"}` while the brain —
// the default answerer for fs.*/library.* — read and wrote the second one. That
// is the entire purpose of this file bypassed for every PathParam method a Go
// provider answers, with both suites green.
//
// The corpus pins the shape through paramShapes; this pins the CONSEQUENCE: the
// two-key form must be refused exactly as firmly as the honest one-key form of
// the same escape.
func TestAuthorizeRefusesCaseVariantDuplicateOfTheScopedField(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	grantRoot := filepath.Join(sandbox, "plugin")
	victim := filepath.Join(sandbox, "victim")
	for _, d := range []string{grantRoot, victim} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	canonRoot, ok := canonicalizeRoot(grantRoot)
	if !ok {
		t.Fatal("the grant root should canonicalize")
	}
	cn := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{canonRoot}}}}

	benign := filepath.Join(grantRoot, "ok.txt")
	loot := filepath.Join(victim, "loot.txt")

	// Control: the honest request for the same file is refused, so the two-key
	// form has something to smuggle.
	honest, _ := json.Marshal(map[string]string{"path": loot})
	if err := cn.authorize("fs.read", honest); err == nil {
		t.Fatal("the one-key form of this path must already be refused, or this test proves nothing")
	}
	// And the benign path on its own is allowed, so a refusal below cannot be
	// mistaken for "this conn is denied everything".
	fine, _ := json.Marshal(map[string]string{"path": benign})
	if err := cn.authorize("fs.read", fine); err != nil {
		t.Fatalf("the plugin's own file must stay readable: %v", err)
	}

	// json.Marshal of a map cannot produce two keys differing only by case, so
	// the attack shape is spelled out literally — as a caller would send it.
	for _, params := range []string{
		`{"path":` + strconv.Quote(benign) + `,"Path":` + strconv.Quote(loot) + `}`,
		`{"PATH":` + strconv.Quote(loot) + `,"path":` + strconv.Quote(benign) + `}`,
	} {
		if err := cn.authorize("fs.read", json.RawMessage(params)); err == nil {
			t.Errorf("authorize allowed an ambiguous params object: %s\n"+
				"the bus confined %q while a Go provider's decoder reads %q — one call must carry exactly one path",
				params, benign, loot)
		}
	}
}
