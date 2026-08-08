package plugin

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCapabilityUnmarshal(t *testing.T) {
	// Bare string → verb only, no paths.
	var c Capability
	if err := json.Unmarshal([]byte(`"agents.list"`), &c); err != nil {
		t.Fatal(err)
	}
	if c.Method != "agents.list" || c.Paths != nil {
		t.Fatalf("string form = %+v, want {agents.list <nil>}", c)
	}

	// Object form → method + paths.
	var d Capability
	if err := json.Unmarshal([]byte(`{"method":"fs.read","paths":["${pluginDir}","/abs"]}`), &d); err != nil {
		t.Fatal(err)
	}
	if d.Method != "fs.read" || len(d.Paths) != 2 || d.Paths[0] != "${pluginDir}" || d.Paths[1] != "/abs" {
		t.Fatalf("object form = %+v", d)
	}

	// A whole manifest with mixed capability forms round-trips.
	var caps []Capability
	if err := json.Unmarshal([]byte(`["agents.list",{"method":"fs.write","paths":["${pluginDir}"]}]`), &caps); err != nil {
		t.Fatal(err)
	}
	if len(caps) != 2 || caps[0].Method != "agents.list" || caps[1].Method != "fs.write" {
		t.Fatalf("mixed list = %+v", caps)
	}
}

func TestValidate_RejectsUnscopedFilesystemCapability(t *testing.T) {
	m := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "fs.read"}}}
	if err := m.Validate(); err == nil {
		t.Fatal("expected unscoped fs.read to be rejected")
	}

	// With paths declared it's fine.
	ok := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "fs.read", Paths: []string{"${pluginDir}"}}}}
	if err := ok.Validate(); err != nil {
		t.Fatalf("scoped fs.read should validate, got %v", err)
	}

	// A non-path capability needs no paths.
	verb := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "agents.list"}}}
	if err := verb.Validate(); err != nil {
		t.Fatalf("verb-only capability should validate, got %v", err)
	}

	// Empty method is rejected.
	empty := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: ""}}}
	if err := empty.Validate(); err == nil {
		t.Fatal("expected empty capability method to be rejected")
	}
}

// A path scope that climbs out of the directory it names is refused by the
// manifest itself, so it is never installed and never reaches a grant. The
// concrete attack: "${pluginDir}/../.." resolves to the config dir, which holds
// remote-token; reading that token gets the plugin a trusted bus connection and
// with it /plugins/install, i.e. arbitrary command execution.
func TestValidate_RejectsEscapingPathScope(t *testing.T) {
	escapes := []string{
		"${pluginDir}/../..",
		"${pluginDir}/..",
		"${pluginDir}/data/../../..",
		"${agentCwd}/..",
		filepath.FromSlash("/plugins/acme/../.."),
		"${pluginDir}\\..\\..",
	}
	for _, p := range escapes {
		m := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "fs.read", Paths: []string{p}}}}
		if err := m.Validate(); err == nil {
			t.Errorf("expected path scope %q to be rejected", p)
		}
	}

	// The escape is caught wherever it hides in the list, not just first.
	buried := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{
		{Method: "fs.read", Paths: []string{"${pluginDir}", "${pluginDir}/../.."}},
	}}
	if err := buried.Validate(); err == nil {
		t.Fatal("expected a later escaping scope to be rejected")
	}

	// Scopes that only ever narrow still validate — including a dotfile subpath,
	// which is not a ".." segment.
	ok := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{
		{Method: "fs.read", Paths: []string{"${pluginDir}", "${pluginDir}/data", "${agentCwd}/src", filepath.FromSlash("/abs/path"), "${pluginDir}/.cache"}},
	}}
	if err := ok.Validate(); err != nil {
		t.Fatalf("narrowing scopes should validate, got %v", err)
	}
}

// The real path a plugin arrives by: Load reads plugin.json and validates it, so
// an escaping scope is refused at install/load time rather than granted.
func TestLoad_RejectsEscapingPathScope(t *testing.T) {
	dir := t.TempDir()
	body := `{"id":"acme","apiVersion":"1","capabilities":[{"method":"fs.read","paths":["${pluginDir}/../.."]}]}`
	path := filepath.Join(dir, "plugin.json")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := Load(path); err == nil {
		t.Fatal("expected Load to reject a manifest with an escaping path scope")
	} else if !strings.Contains(err.Error(), "..") {
		t.Fatalf("error should name the offending scope, got %v", err)
	}
}

// Defence in depth: a manifest that got past Validate some other way still yields
// no root for the escaping scope, and a grant with no roots denies every call.
func TestGrantsFor_DropsEscapingScope(t *testing.T) {
	mf := Manifest{
		Dir: filepath.FromSlash("/plugins/acme"),
		Capabilities: []Capability{
			{Method: "fs.read", Paths: []string{"${pluginDir}/../.."}},
		},
	}
	grants := grantsFor(mf)
	if len(grants) != 1 {
		t.Fatalf("got %d grants, want 1: %+v", len(grants), grants)
	}
	if len(grants[0].FSRoots) != 0 {
		t.Fatalf("escaping scope produced roots %v, want none", grants[0].FSRoots)
	}
}

func TestExpandScope(t *testing.T) {
	dir := filepath.FromSlash("/plugins/acme")
	cwd := filepath.FromSlash("/work/project")
	bindings := map[string]string{"pluginDir": dir, "agentCwd": cwd}
	cases := []struct {
		in   string
		want string
	}{
		{"${pluginDir}", dir},
		{"${pluginDir}/data", filepath.Join(dir, "data")},
		{"${agentCwd}", cwd},                           // bound dynamic scope resolves
		{"${agentCwd}/src", filepath.Join(cwd, "src")}, // …with a subpath
		{filepath.FromSlash("/abs/path"), filepath.FromSlash("/abs/path")},
		{"relative/path", ""}, // relative → dropped
		{"${unknown}/x", ""},  // no such binding → dropped
		{"${malformed", ""},   // no closing brace → dropped
		// A ".." segment is dropped, not Cleaned. "${pluginDir}/../.." used to join
		// to the config dir — remote-token's home, i.e. a trusted bus connection.
		{"${pluginDir}/../..", ""},
		{"${pluginDir}/..", ""},
		{"${agentCwd}/../../etc", ""},
		{"${pluginDir}/data/../../other", ""}, // climbing back in is still refused
		{filepath.FromSlash("/plugins/acme/../.."), ""},
		{"${pluginDir}\\..\\..", ""}, // the other separator is no way around it
	}
	for _, c := range cases {
		if got := expandScope(c.in, bindings); got != c.want {
			t.Errorf("expandScope(%q) = %q, want %q", c.in, got, c.want)
		}
	}
	// With no agentCwd binding (the static load-time case), ${agentCwd} grants nothing.
	if got := expandScope("${agentCwd}", map[string]string{"pluginDir": dir}); got != "" {
		t.Errorf("unbound ${agentCwd} = %q, want \"\"", got)
	}
}

// TestBareTokenBoundToAVolumeRootGrantsNothing covers the branch that had no
// narrowing step: a BARE `${name}` used to return its binding verbatim.
//
// `${agentCwd}` is bound by the trusted host from the pane it is opening, and
// that pane comes out of the SHARED LAYOUT DOCUMENT — which a non-trusted bus
// caller may write with layout.set. So the document decided the plugin sandbox's
// own boundary. One agent with cwd "/" and one plugin pane minted a token whose
// fsRoots were ["/"], and since a volume root contains everything below it
// (BINDING DECISION 3), the bus's per-plugin path confinement — the ONE guard
// that is per-caller rather than per-host — then admitted every path on the
// machine. Neither call is wrong alone: layout.set writes an opaque document and
// PaneToken faithfully binds what the trusted host hands it.
//
// The SUBPATH branch already refused this exact shape (`${pluginDir}/all` with
// `all -> /`, in the test below); a bare token reached it without a symlink.
func TestBareTokenBoundToAVolumeRootGrantsNothing(t *testing.T) {
	root := filepath.VolumeName(mustAbs(t)) + string(filepath.Separator)
	if got := expandScope("${agentCwd}", map[string]string{"agentCwd": root}); got != "" {
		t.Errorf("expandScope(${agentCwd}) with the binding at the volume root %q = %q, want \"\" — a root is the one value for which \"confined to this subtree\" and \"not confined at all\" are the same sentence", root, got)
	}
	// THE FLOOR: an ordinary directory must still expand, or the guard above is
	// satisfied by a function that grants nothing at all.
	ordinary, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if got := expandScope("${agentCwd}", map[string]string{"agentCwd": ordinary}); got != ordinary {
		t.Fatalf("expandScope(${agentCwd}) with an ordinary binding = %q, want %q — the refusal above is refusing everything", got, ordinary)
	}
	// And a SYMLINK to the root is refused too: the check resolves rather than
	// comparing strings, for the reason the subpath branch resolves.
	if !canSymlink(t) {
		return
	}
	link := filepath.Join(t.TempDir(), "toroot")
	if err := os.Symlink(root, link); err != nil {
		t.Skipf("cannot create a symlink here: %v", err)
	}
	if got := expandScope("${agentCwd}", map[string]string{"agentCwd": link}); got != "" {
		t.Errorf("expandScope(${agentCwd}) with a binding SYMLINKED to %q = %q, want \"\" — the check must resolve, not compare spellings", root, got)
	}
}

// mustAbs returns an absolute path on this platform, used only to recover the
// volume name ("" on POSIX, "C:" on Windows).
func mustAbs(t *testing.T) string {
	t.Helper()
	p, err := filepath.Abs(".")
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func canSymlink(t *testing.T) bool {
	t.Helper()
	d := t.TempDir()
	if err := os.Symlink(d, filepath.Join(d, "probe")); err != nil {
		return false
	}
	return true
}

func TestWithinRoot(t *testing.T) {
	root := filepath.FromSlash("/plugins/acme")
	cases := []struct {
		base, path string
		want       bool
	}{
		{root, root, true},
		{root, filepath.Join(root, "data"), true},
		{root + string(filepath.Separator), filepath.Join(root, "data"), true}, // trailing separator on the binding
		{root, filepath.FromSlash("/plugins"), false},
		{root, filepath.FromSlash("/plugins/acme-evil"), false}, // sibling with the root as a name prefix
		{filepath.FromSlash("/"), filepath.FromSlash("/etc"), true},
	}
	for _, c := range cases {
		if got := withinRoot(c.base, c.path); got != c.want {
			t.Errorf("withinRoot(%q, %q) = %v, want %v", c.base, c.path, got, c.want)
		}
	}
}

func TestGrantsFor(t *testing.T) {
	dir := filepath.FromSlash("/plugins/acme")
	mf := Manifest{
		Dir: dir,
		Capabilities: []Capability{
			{Method: "agents.list"},
			{Method: "fs.read", Paths: []string{"${pluginDir}", "relative-dropped"}},
			{Method: ""}, // skipped
		},
	}
	grants := grantsFor(mf)
	if len(grants) != 2 {
		t.Fatalf("got %d grants, want 2 (empty-method skipped): %+v", len(grants), grants)
	}
	// agents.list → verb only, no roots.
	if grants[0].Method != "agents.list" || len(grants[0].FSRoots) != 0 {
		t.Errorf("grant[0] = %+v, want agents.list with no roots", grants[0])
	}
	// fs.read → only the resolvable root survives (relative dropped).
	if grants[1].Method != "fs.read" || len(grants[1].FSRoots) != 1 || grants[1].FSRoots[0] != dir {
		t.Errorf("grant[1] = %+v, want fs.read scoped to %q", grants[1], dir)
	}
}

// The absolute branch of expandScope is a named anti-escalation check with its
// own comment naming the escalation ("a manifest declaring fs.read on the config
// dir could read remote-token and reconnect as a TRUSTED bus connection, which
// drops per-plugin scoping and unlocks /plugins/install — arbitrary commands"),
// and nothing entered it: the table above only covers the ${token} cases, and
// `/abs/path` passes because it is nowhere near a credential.
//
// Reverting the branch to the pre-fix `return p` left the whole hub suite green
// while granting a plugin a bus root of <configDir>/workspacer, or of
// remote-token itself. The per-call gate (policy.go pathIsSecret) still refuses
// at call time — both layers ship deliberately — but the load-time half must not
// be deletable in silence.
func TestExpandScopeRefusesAnAbsoluteScopeOnACredential(t *testing.T) {
	cfgHome := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfgHome)
	t.Setenv("APPDATA", cfgHome)
	cfg := filepath.Join(cfgHome, "workspacer")
	for _, d := range []string{"library", "layouts", "sessions", "plugins"} {
		if err := os.MkdirAll(filepath.Join(cfg, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(cfg, "remote-token"), []byte("tok"), 0o600); err != nil {
		t.Fatal(err)
	}
	project := t.TempDir()
	bindings := map[string]string{"pluginDir": filepath.Join(cfg, "plugins", "acme"), "agentCwd": project}

	refused := []string{
		cfg,                                  // the config dir itself
		filepath.Join(cfg, "remote-token"),   // the credential by location
		filepath.Join(cfg, "plugins"),        // every plugin's .bus-token
		filepath.Join(cfg, "config.yaml"),    // updates.channel → the updater feed URL
		filepath.Join(project, ".bus-token"), // a credential BASENAME anywhere
		filepath.Join(project, ".settings.json"),
	}
	for _, scope := range refused {
		if got := expandScope(scope, bindings); got != "" {
			t.Errorf("expandScope(%q) = %q, want \"\" — this scope names a credential or the config dir", scope, got)
		}
		if got := resolveRoots([]string{scope}, bindings); len(got) != 0 {
			t.Errorf("resolveRoots(%q) granted %v, want no roots", scope, got)
		}
	}

	// The floor: the three store carve-outs and an ordinary project directory
	// are still grantable, or a check that dropped every absolute scope would
	// satisfy the loop above.
	for _, scope := range []string{
		filepath.Join(cfg, "library"),
		filepath.Join(cfg, "layouts"),
		filepath.Join(cfg, "sessions"),
		project,
	} {
		if got := expandScope(scope, bindings); got != scope {
			t.Errorf("expandScope(%q) = %q, want it granted", scope, got)
		}
	}
}

// A symlink SHIPPED INSIDE the plugin's own directory used to relocate the
// plugin's own grant, and nothing anywhere noticed.
//
// expandScope validated "${pluginDir}/all" lexically and returned it; the bus's
// canonRoots then FOLLOWED the link (BINDING DECISION 2 — a root is stored
// resolved) and recorded "/" as the grant root; within("/", anything) is true
// (BINDING DECISION 3), so the plugin held fs.write on the whole filesystem
// minus the secret gate — ~/.claude/settings.json (hooks are arbitrary
// commands), ~/.ssh/authorized_keys, /etc. `/plugins/install` clones a
// repository verbatim, so the link arrives with the plugin; the plugin's own
// sidecar can also create it before a pane token is minted.
//
// The scope must confine the plugin to the directory it NAMES, wherever the
// subpath happens to land.
func TestExpandScope_SymlinkInsideThePluginDirCannotWidenItsOwnGrant(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	pluginDir := filepath.Join(sandbox, "plugins", "evil")
	victim := filepath.Join(sandbox, "someone-elses-project")
	for _, d := range []string{pluginDir, victim} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Symlink(string(filepath.Separator), filepath.Join(pluginDir, "all")); err != nil {
		t.Skipf("cannot create symlinks here: %v", err)
	}
	if err := os.Symlink(victim, filepath.Join(pluginDir, "data")); err != nil {
		t.Skipf("cannot create symlinks here: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(pluginDir, "own"), 0o755); err != nil {
		t.Fatal(err)
	}

	bindings := map[string]string{"pluginDir": pluginDir}
	for _, c := range []struct {
		scope string
		want  string
	}{
		{"${pluginDir}/all", ""},                                  // -> "/" : the whole filesystem
		{"${pluginDir}/data", ""},                                 // -> another project
		{"${pluginDir}/own", filepath.Join(pluginDir, "own")},     // a real subdirectory still narrows
		{"${pluginDir}/later", filepath.Join(pluginDir, "later")}, // …and so does one not created yet
	} {
		if got := expandScope(c.scope, bindings); got != c.want {
			t.Errorf("expandScope(%q) = %q, want %q — a subpath may only NARROW the binding it names, and where it LANDS is what decides that", c.scope, got, c.want)
		}
	}

	// End to end, through the function the manager actually calls: the widened
	// scope must contribute no root at all rather than a wider one.
	mf := Manifest{Dir: pluginDir, Capabilities: []Capability{
		{Method: "fs.write", Paths: []string{"${pluginDir}/all"}},
		{Method: "fs.read", Paths: []string{"${pluginDir}/data"}},
	}}
	for _, g := range grantsFor(mf) {
		if len(g.FSRoots) != 0 {
			t.Errorf("%s kept roots %v — the bus canonicalizes these, so storing them hands the plugin what the symlink points at", g.Method, g.FSRoots)
		}
	}
}
