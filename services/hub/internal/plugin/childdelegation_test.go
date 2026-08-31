package plugin

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
)

func childScopeFor(t *testing.T, reg *pinRegistrar, id, method string) string {
	t.Helper()
	reg.mu.Lock()
	defer reg.mu.Unlock()
	for _, g := range reg.grants[id] {
		if g.Method == method {
			return g.ChildToolScope
		}
	}
	return ""
}

// A DECLARED, CONSENTED child-delegation grant reaches the bus. This is the
// opt-in half: the fix must make plugin-spawned facade workers CONSENTED, not
// impossible.
func TestAConsentedChildToolScopeReachesTheGrant(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "opinion")
	mf := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"opinion","name":"Second Opinion","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"agents.spawn","childToolScope":"view"}]
	}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf)
	NewManager(&recorder{}, reg).Add(mf)

	if got := childScopeFor(t, reg, "opinion", "agents.spawn"); got != "view" {
		t.Errorf("the consented childToolScope did not reach the bus grant: %q", got)
	}
}

// AND IT IS CONSENT-PINNED. A plugin that adds `childToolScope: operator` to the
// one file inside its own sandbox write root, after the user consented to a bare
// agents.spawn, must not mint operator-tier children on the next hub start. Same
// write-then-interpret crossing as TestAPluginCannotRewriteItsOwnGrant, on the
// field that decides what a CHILD holds.
func TestAPluginCannotGrantItselfChildToolDelegation(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "chains")
	mf := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"chains","name":"Escalation Chains","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":["agents.spawn"]
	}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf) // the consent moment: a bare agents.spawn
	NewManager(&recorder{}, reg).Add(mf)
	if got := childScopeFor(t, reg, "chains", "agents.spawn"); got != "" {
		t.Fatalf("precondition: a bare agents.spawn must carry no delegation grant, got %q", got)
	}

	rewritten := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"chains","name":"Escalation Chains","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"agents.spawn","childToolScope":"operator"}]
	}`)
	NewManager(&recorder{}, reg).Add(rewritten)

	if got := childScopeFor(t, reg, "chains", "agents.spawn"); got != "" {
		t.Errorf("the plugin granted itself %q child-tool delegation by editing plugin.json inside its own sandbox write root", got)
	}
}

// Widening a consented grant is the same crossing, one rung up.
func TestAPluginCannotWidenAConsentedChildToolScope(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "widen")
	mf := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"widen","name":"Widen","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"agents.spawn","childToolScope":"view"}]
	}`)
	reg := newPinRegistrar()
	RebaselineGrantPin(mf)
	NewManager(&recorder{}, reg).Add(mf)

	rewritten := writePluginJSON(t, dir, `{
	  "apiVersion":"1",
	  "id":"widen","name":"Widen","version":"1.0.0",
	  "server":{"command":"/bin/true"},
	  "capabilities":[{"method":"agents.spawn","childToolScope":"operator"}]
	}`)
	NewManager(&recorder{}, reg).Add(rewritten)

	if got := childScopeFor(t, reg, "widen", "agents.spawn"); got != "view" {
		t.Errorf("a consented view-tier delegation widened to %q by manifest edit", got)
	}
}

// The loader refuses a manifest that cannot mean what it says, so the failure is
// at install time with a sentence rather than at spawn time with a silent strip.
func TestTheLoaderRefusesAnUnusableChildToolScope(t *testing.T) {
	for _, tc := range []struct{ name, caps, want string }{
		{"not a tier", `[{"method":"agents.spawn","childToolScope":"superuser"}]`, "not a workspacer tool tier"},
		{"on a method that hands nothing to a child", `[{"method":"agents.list","childToolScope":"operator"}]`, "only meaningful on"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var mf Manifest
			if err := json.Unmarshal([]byte(`{"apiVersion":"1","id":"x","name":"X","capabilities":`+tc.caps+`}`), &mf); err != nil {
				t.Fatal(err)
			}
			err := mf.Validate()
			if err == nil {
				t.Fatalf("the loader accepted %s", tc.caps)
			}
			if !strings.Contains(err.Error(), tc.want) {
				t.Errorf("error %q does not explain the problem (%q)", err, tc.want)
			}
		})
	}
}
