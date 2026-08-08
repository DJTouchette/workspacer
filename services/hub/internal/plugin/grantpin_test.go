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
