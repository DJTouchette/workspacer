package bus

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

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
	Modes    map[string]string `json:"modes"`
}

type containmentCase struct {
	Name               string          `json:"name"`
	Group              string          `json:"group"`
	NeedsSymlinks      bool            `json:"needsSymlinks"`
	PosixOnly          bool            `json:"posixOnly"`
	NeedsUnreadableDir bool            `json:"needsUnreadableDir"`
	Tree               containmentTree `json:"tree"`
	Roots              []string        `json:"roots"`
	Target             string          `json:"target"`
	Expect             string          `json:"expect"`
	Why                string          `json:"why"`
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

type containmentFixture struct {
	Owners      map[string][]string `json:"owners"`
	Cases       []containmentCase   `json:"cases"`
	Methods     []methodCase        `json:"methods"`
	ParamShapes []paramShapeCase    `json:"paramShapes"`
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
			// The secret gate resolves the config dir through authtoken.ConfigDir()
			// at call time, so pointing the environment at the sandbox is what puts
			// ${CONFIG} in scope.
			t.Setenv("XDG_CONFIG_HOME", configHome)
			if runtime.GOOS == "windows" {
				t.Setenv("APPDATA", configHome)
			}

			materializeTree(t, sandbox, c.Tree, c.NeedsSymlinks)

			sub := strings.NewReplacer(
				"${SANDBOX}", sandbox,
				"${ROOT}", root,
				"${OUTSIDE}", outside,
				"${CONFIG}", config,
			)
			roots := make([]string, 0, len(c.Roots))
			for _, r := range c.Roots {
				cr, ok := canonicalizeRoot(sub.Replace(r))
				if !ok {
					continue // DISCARD: skip this root, never abort the check
				}
				roots = append(roots, cr)
			}
			target := sub.Replace(c.Target)

			// The fixture's verdict is allow/deny only: conn.authorize denies on
			// ANY non-nil error (unresolvable path, or the secret sentinel), so
			// `ok` is the whole of the shared contract. The error is inspected
			// below purely for the bus-local question the fixture cannot ask.
			ok, err := pathWithinRoots(roots, target)
			want := c.Expect == "allow"
			if ok != want {
				t.Errorf("pathWithinRoots(%v, %q) = %v, want %v\ncase: %s\nwhy: %s",
					roots, target, ok, want, c.Name, c.Why)
			}
			// Bus-only strengthening, not part of the shared corpus. The bus
			// carries TWO refusals: the containment one may echo the plugin's own
			// requested path, the secret one must not (spec 7.5), and they are
			// told apart by the errSecretPath sentinel. Which arm a deny SHOULD
			// take is derivable without a second opinion on the verdict: if the
			// resolved target sits inside a granted root, containment passed and
			// only the gate can have refused it. Without this, a regression that
			// collapsed the two arms back into one path-echoing message would
			// leave all 65 cases green.
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
