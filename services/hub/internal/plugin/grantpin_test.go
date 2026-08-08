package plugin

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// pinRegistrar records every grant registration, so a test can compare the
// authority a plugin held before and after it edited its own manifest.
type pinRegistrar struct {
	mu     sync.Mutex
	grants map[string][]capspec.Grant
	events map[string]capspec.EventGrants
	tokens map[string]string
}

func newPinRegistrar() *pinRegistrar {
	return &pinRegistrar{grants: map[string][]capspec.Grant{}, events: map[string]capspec.EventGrants{}, tokens: map[string]string{}}
}

func (p *pinRegistrar) RegisterPluginToken(token, id string, grants []capspec.Grant, ev capspec.EventGrants) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.grants[id] = grants
	p.events[id] = ev
	p.tokens[id] = token
}
func (p *pinRegistrar) UnregisterPluginToken(string) {}

func (p *pinRegistrar) methods(id string) []string {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := []string{}
	for _, g := range p.grants[id] {
		out = append(out, g.Method)
	}
	return out
}

func (p *pinRegistrar) rootsFor(id, method string) []string {
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, g := range p.grants[id] {
		if g.Method == method {
			return g.FSRoots
		}
	}
	return nil
}

// writePluginJSON writes a manifest into dir and loads it the way the hub does.
func writePluginJSON(t *testing.T, dir string, body string) Manifest {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(dir, "plugin.json")
	if err := os.WriteFile(file, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	mf, err := Load(file)
	if err != nil {
		t.Fatalf("loading %s: %v", file, err)
	}
	return mf
}

// PROVEN, critical. A plugin rewrites its own plugin.json — the one file inside
// its one writable sandbox root — and the hub re-derived a strictly larger bus
// grant on the SAME persisted token, with no consent re-check.
//
// The two designs were each right alone. The sandbox says "the sidecar may write
// only its own directory"; the loader says "a plugin's authority is what its
// manifest declares". Together the write scope CONTAINS the file that defines
// the authority, so the containment is self-referential and bytes the sandbox
// permits as DATA are re-read as the plugin's CAPABILITY GRANT.
func TestAPluginCannotRewriteItsOwnGrant(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")

	mf := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]}]
	}`)

	reg := newPinRegistrar()
	m := NewManager(&recorder{}, reg)
	// The install/consent moment: the dialog showed exactly these capabilities.
	RebaselineGrantPin(mf)
	m.Add(mf)

	before := reg.methods("notes")
	tokenBefore := reg.tokens["notes"]
	if len(before) != 1 || before[0] != "fs.read" {
		t.Fatalf("consented grant = %v, want just fs.read", before)
	}

	// THE SANDBOX'S WRITE ROOT, from manager.go: sandbox.Policy{WriteRoots:
	// []string{mf.Dir}} — the directory that holds plugin.json. This is the
	// sidecar doing the one write it is permitted.
	if _, err := os.Stat(filepath.Join(dir, "plugin.json")); err != nil {
		t.Fatalf("precondition: plugin.json must live inside the sandbox write root: %v", err)
	}
	rewritten := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[
	    {"method":"fs.read","paths":["${pluginDir}","/"]},
	    {"method":"fs.write","paths":["/"]},
	    "agents.spawn","terminals.create","config.save","sessions.save"
	  ],
	  "emits":["layout.changed"],
	  "consumes":["pty.bytes.*"]
	}`)

	// The next hub start (or any non-consented re-Add) re-reads the manifest.
	m2 := NewManager(&recorder{}, reg)
	m2.Add(rewritten)

	after := reg.methods("notes")
	for _, escalated := range []string{"fs.write", "agents.spawn", "terminals.create", "config.save", "sessions.save"} {
		for _, got := range after {
			if got == escalated {
				t.Errorf("the plugin granted itself %q by editing the file inside its own sandbox write root; grant is now %v", escalated, after)
			}
		}
	}
	if roots := reg.rootsFor("notes", "fs.read"); len(roots) > 0 && roots[0] == "/" {
		t.Errorf("fs.read widened to the volume root by manifest edit: %v", roots)
	}
	ev := reg.events["notes"]
	if len(ev.Emits) > 0 || len(ev.Consumes) > 0 {
		t.Errorf("the plugin granted itself event reach it never consented to: emits=%v consumes=%v — layout.changed is a host-state topic and pty.bytes.* is a guarded terminal stream", ev.Emits, ev.Consumes)
	}
	// The token really is the same one, which is what makes this an escalation
	// on a LIVE credential rather than a new install.
	if reg.tokens["notes"] != tokenBefore {
		t.Fatalf("precondition drifted: the persisted .bus-token changed, so this is no longer the same credential")
	}
	// FLOOR: what WAS consented still works.
	if len(after) != 1 || after[0] != "fs.read" {
		t.Fatalf("the consented capability was lost as well; grant = %v", after)
	}
}

// PROVEN, critical. The escalation TestAPluginCannotRewriteItsOwnGrant blocks,
// but reached by RENAMING the plugin's own id in the same write. The grant pin
// used to be keyed on mf.ID — which is read from plugin.json, the file inside
// the sandbox's one write root — so a sidecar that both broadened its scope AND
// changed its id pointed the pin lookup at a name that had no pin on disk. That
// reads as "never seen", trust-on-first-load re-baselines the escalated manifest,
// and the broadened surface is granted with no consent. The pin must be keyed on
// the one identity the plugin cannot forge: the directory it lives in.
func TestAPluginCannotEscapeItsGrantByRenamingItsId(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")
	elsewhere := t.TempDir() // resolvable, absolute, NOT consented

	mf := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]}]
	}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf) // the consent moment
	NewManager(&recorder{}, reg).Add(mf)
	if got := reg.methods("notes"); len(got) != 1 || got[0] != "fs.read" {
		t.Fatalf("floor: consented grant = %v, want just fs.read", got)
	}

	// The sidecar rewrites plugin.json inside its own write root: it changes the
	// id AND broadens fs.read to a directory it was never consented to, AND adds
	// an escalated capability. The directory it runs in is unchanged (it cannot
	// rename that — the sandbox does not grant write on the parent).
	rewritten := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"notes2","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[
	    {"method":"fs.read","paths":[`+jsonString(elsewhere)+`]},
	    "config.save"
	  ]
	}`)
	m2 := NewManager(&recorder{}, reg)
	m2.Add(rewritten)

	// Grants are registered under whatever id the manifest now claims.
	if roots := reg.rootsFor("notes2", "fs.read"); len(roots) > 0 {
		t.Errorf("fs.read granted roots %v under the renamed id — the pin (keyed on the plugin directory) must still narrow the broadened scope to nothing", roots)
	}
	for _, m := range reg.methods("notes2") {
		if m == "config.save" {
			t.Errorf("the plugin granted itself config.save by renaming its id; the grant pin never fired")
		}
	}
}

// The other direction: a legitimate UPDATE, where a human saw the new
// capabilities in the install dialog, must actually get them. A pin that could
// only ever shrink would make every plugin update silently broken, and the next
// person would delete it.
func TestAConsentedInstallRebaselinesThePin(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")
	mf := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]}]}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf)
	NewManager(&recorder{}, reg).Add(mf)

	updated := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"2.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]},"agents.list"]}`)
	RebaselineGrantPin(updated) // what the install / explicit-reload route does
	NewManager(&recorder{}, reg).Add(updated)

	got := reg.methods("notes")
	if len(got) != 2 {
		t.Fatalf("a consented update did not get its new capability: %v", got)
	}
}

// A pin that exists but is damaged is the TAMPER case, and must not resolve to
// "believe plugin.json".
func TestACorruptGrantPinGrantsNothing(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")
	mf := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]},"agents.list"]}`)
	RebaselineGrantPin(mf)

	pin := filepath.Join(plugins, grantPinDirName, "notes.json")
	if _, err := os.Stat(pin); err != nil {
		t.Fatalf("the pin must live OUTSIDE the plugin directory (the sandbox's write root): %v", err)
	}
	if strings.HasPrefix(pin, dir+string(filepath.Separator)) {
		t.Fatalf("the pin is inside the plugin's own writable directory (%s) — the plugin could rewrite the record of what it was allowed", pin)
	}
	if err := os.WriteFile(pin, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}

	reg := newPinRegistrar()
	NewManager(&recorder{}, reg).Add(mf)
	if got := reg.methods("notes"); len(got) != 0 {
		t.Fatalf("a corrupt pin granted %v — an unreadable record of consent must not mean 'take what the file says'", got)
	}
}

// First sight of a plugin writes the pin from its manifest. Trust on first load
// is the only answer that does not revoke every already-installed plugin the
// moment this ships, and it matches what the install dialog showed.
func TestFirstLoadRecordsThePinFromTheManifest(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")
	mf := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":["agents.list"],"emits":["notes.saved"]}`)

	reg := newPinRegistrar()
	NewManager(&recorder{}, reg).Add(mf)
	if got := reg.methods("notes"); len(got) != 1 || got[0] != "agents.list" {
		t.Fatalf("first load did not grant the manifest's own capabilities: %v", got)
	}
	data, err := os.ReadFile(filepath.Join(plugins, grantPinDirName, "notes.json"))
	if err != nil {
		t.Fatalf("first load did not record a pin: %v", err)
	}
	var pin grantPin
	if err := json.Unmarshal(data, &pin); err != nil {
		t.Fatal(err)
	}
	if len(pin.Capabilities) != 1 || pin.Capabilities[0].Method != "agents.list" || len(pin.Emits) != 1 {
		t.Fatalf("pin = %+v, want the manifest's declared surface", pin)
	}
}

// PROVEN, high. expandScope refuses "/" as a bound-token scope and accepted it
// as a literal absolute scope: one predicate, two callers, one of them not
// asked. isVolumeRoot's own doc comment states the rule as a property of the
// VALUE — "a root is the one value for which 'confined to this subtree' and 'not
// confined at all' are the same sentence" — and the absolute branch was left on
// install-time consent alone, which the finding above shows a plugin can outrun.
func TestAnAbsoluteVolumeRootIsNotAScope(t *testing.T) {
	mf := Manifest{ID: "p", Dir: t.TempDir()}
	if got := expandScope("/", map[string]string{"pluginDir": mf.Dir}); got != "" {
		t.Errorf(`expandScope("/") = %q, want "" — a volume root contains every path on the host, so it confines nothing`, got)
	}
	// FLOOR: an ordinary absolute scope still resolves, or the guard is a
	// blanket refusal of absolute paths.
	ordinary := t.TempDir()
	if got := expandScope(ordinary, map[string]string{"pluginDir": mf.Dir}); got != ordinary {
		t.Errorf("expandScope(%q) = %q, want it unchanged — the fix must not refuse every absolute scope", ordinary, got)
	}
}

// narrowToPin's DROP-WHOLE arm: a path-scoped capability whose every declared
// scope is new keeps nothing, so the capability itself is removed rather than
// registered with an empty root list.
//
// It survived deletion, which is why it is here. The difference the branch makes
// is not "granted vs not granted" today — bus.go's authorize refuses a
// filesystem-scoped capability with no roots outright ("granted with no roots")
// — it is what the REGISTERED grant says the plugin holds. Without the branch
// the token is registered carrying fs.read with zero roots, and every reader of
// that list (the bus's grant map, a consent/inspection UI, the next person who
// adds a default root for an empty scope) sees a capability the user never
// consented to, held with a scope that reads as "unspecified" rather than
// "refused". A dropped capability says the true thing: it was added after
// consent and it is gone.
func TestACapabilityWhoseEveryScopeIsNewIsDroppedWhole(t *testing.T) {
	plugins := t.TempDir()
	dir := filepath.Join(plugins, "notes")
	elsewhere := t.TempDir() // resolvable, absolute, and NOT what was consented

	mf := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}"]}]}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf) // the consent moment
	NewManager(&recorder{}, reg).Add(mf)
	if got := reg.methods("notes"); len(got) != 1 || got[0] != "fs.read" {
		t.Fatalf("floor: the consented grant = %v, want just fs.read", got)
	}

	// The sidecar rewrites plugin.json inside its own sandbox write root, moving
	// fs.read off the consented scope entirely. Both are legal, resolvable
	// scopes — the pin is the only thing that distinguishes them.
	rewritten := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":[`+jsonString(elsewhere)+`]}]}`)
	NewManager(&recorder{}, reg).Add(rewritten)

	if got := reg.methods("notes"); len(got) != 0 {
		t.Errorf("grant after the rewrite = %v with roots %v — every declared scope was new, so nothing was consented and the capability must not appear in the registered authority at all",
			got, reg.rootsFor("notes", "fs.read"))
	}

	// FLOOR: narrowing is per-scope, not all-or-nothing. A manifest that keeps
	// the consented scope AND adds a new one keeps the capability, minus the
	// addition — otherwise this branch would be a way to delete a live grant by
	// appending to plugin.json.
	partial := writePluginJSON(t, dir, `{"apiVersion":"1","id":"notes","name":"Notes","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"fs.read","paths":["${pluginDir}",`+jsonString(elsewhere)+`]}]}`)
	NewManager(&recorder{}, reg).Add(partial)
	got := reg.methods("notes")
	if len(got) != 1 || got[0] != "fs.read" {
		t.Fatalf("a manifest that still declares the consented scope lost the capability: %v", got)
	}
	roots := reg.rootsFor("notes", "fs.read")
	if len(roots) != 1 || roots[0] != dir {
		t.Fatalf("kept roots = %v, want just the consented %s", roots, dir)
	}
}

// jsonString quotes a filesystem path for embedding in the manifest literals
// above; a Windows path is full of backslashes and json.Marshal is the only
// correct escaper.
func jsonString(s string) string {
	b, err := json.Marshal(s)
	if err != nil {
		panic(err)
	}
	return string(b)
}
