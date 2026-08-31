package bus

import (
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// PLUGIN CHILD DELEGATION — consent to SPAWN is not consent to mint an agent
// holding workspacer's own tools.
//
// A plugin has no rung on the view/triage/operator ladder, so callerToolScopeCeiling
// used to exempt it: a plugin consented merely to call agents.spawn could ask for
// `mcpFacade: true` — whose legacy meaning is OPERATOR — and get a child holding
// the full first-party tool set (approve, spawn, terminals, config), on a fleet
// whose agents run with permissions bypassed. The manifest now carries the rung,
// consent-pinned, and its ABSENCE is a real answer rather than an exemption.

// delegationServer wires one plugin token per child-delegation grant shape.
func delegationServer(t *testing.T) (url string, got chan json.RawMessage) {
	t.Helper()
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-none", "plug.none",
		[]capspec.Grant{{Method: "agents.spawn"}}, capspec.EventGrants{})
	srv.RegisterPluginToken("plug-view", "plug.view",
		[]capspec.Grant{{Method: "agents.spawn", ChildToolScope: "view"}}, capspec.EventGrants{})
	srv.RegisterPluginToken("plug-operator", "plug.operator",
		[]capspec.Grant{{Method: "agents.spawn", ChildToolScope: "operator"}}, capspec.EventGrants{})
	srv.RegisterPluginToken("plug-bogus", "plug.bogus",
		[]capspec.Grant{{Method: "agents.spawn", ChildToolScope: "superuser"}}, capspec.EventGrants{})
	srv.RegisterPluginToken("plug-misplaced", "plug.misplaced",
		[]capspec.Grant{{Method: "agents.spawn"}, {Method: "agents.list", ChildToolScope: "operator"}},
		capspec.EventGrants{})

	got = make(chan json.RawMessage, 8)
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	go func() {
		for {
			f, ok := provider.tryRead("call")
			if !ok {
				return
			}
			got <- f.Params
			provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()
	return url, got
}

// THE HEADLINE. Every tool-bearing field goes, not only `toolScope` — a clamp
// that left `mcpFacade` behind would be walked around by one boolean.
func TestAPluginWithNoDelegationGrantHandsItsChildNoTools(t *testing.T) {
	url, got := delegationServer(t)
	for _, tc := range []struct{ name, params string }{
		{"the tier", `{"cwd":"/tmp","toolScope":"operator"}`},
		{"the legacy facade boolean", `{"cwd":"/tmp","mcpFacade":true}`},
		{"plugin tool grants", `{"cwd":"/tmp","mcpFacade":true,"pluginTools":["other.plugin"]}`},
		{"all of them at once", `{"cwd":"/tmp","toolScope":"triage","mcpFacade":true,"pluginTools":["x"]}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			m := spawnVia(t, url, "plug-none", tc.params, got)
			for _, f := range []string{"toolScope", "mcpFacade", "pluginTools"} {
				if _, has := m[f]; has {
					t.Errorf("%q survived a plugin with no childToolScope grant: %v", f, m)
				}
			}
			// The spawn itself still happens — the plugin WAS consented to that.
			if m["cwd"] != "/tmp" {
				t.Errorf("the spawn itself was damaged: %v", m)
			}
			// NO SILENT DOWNGRADES.
			scrub := scrubbedList(t, m)
			if len(scrub) == 0 {
				t.Errorf("nothing was reported to the caller: %v", m)
			}
		})
	}
}

// A DECLARED grant is honoured, and it CLAMPS rather than admits: a view-tier
// delegation asking for operator lands on view.
func TestAPluginsDeclaredDelegationTierIsTheCeilingOnItsChild(t *testing.T) {
	url, got := delegationServer(t)

	m := spawnVia(t, url, "plug-view", `{"cwd":"/tmp","toolScope":"operator"}`, got)
	if m["toolScope"] != "view" {
		t.Errorf("a view-tier delegation grant produced toolScope %v, want view", m["toolScope"])
	}

	// At or under the declared tier, nothing is taken.
	m = spawnVia(t, url, "plug-view", `{"cwd":"/tmp","toolScope":"view"}`, got)
	if m["toolScope"] != "view" {
		t.Errorf("a view request under a view grant was altered: %v", m)
	}
	if s := scrubbedList(t, m); len(s) != 0 {
		t.Errorf("nothing should have been taken: %v", s)
	}

	// The legacy boolean means OPERATOR, so it is clamped to the declared tier
	// and replaced by an explicit one — the flag has no gradations.
	m = spawnVia(t, url, "plug-view", `{"cwd":"/tmp","mcpFacade":true}`, got)
	if _, has := m["mcpFacade"]; has {
		t.Errorf("the legacy operator flag survived a view-tier grant: %v", m)
	}
	if m["toolScope"] != "view" {
		t.Errorf("the clamped legacy flag did not become an explicit view tier: %v", m)
	}
}

// An OPERATOR delegation grant is the deliberate opt-in, and it works — the fix
// must not make plugin-spawned facade workers impossible, only consented.
func TestAPluginGrantedOperatorDelegationStillGetsIt(t *testing.T) {
	url, got := delegationServer(t)
	m := spawnVia(t, url, "plug-operator", `{"cwd":"/tmp","toolScope":"operator","pluginTools":["x"]}`, got)
	if m["toolScope"] != "operator" {
		t.Errorf("an explicitly granted operator delegation was clamped to %v", m["toolScope"])
	}
	if _, has := m["pluginTools"]; !has {
		t.Errorf("pluginTools was stripped from a plugin that may delegate operator tools: %v", m)
	}
}

// FAIL CLOSED on a grant nobody can rank, and on one declared against a method
// that hands no child anything. Either is a manifest the loader should have
// refused; if one reaches the bus it must mean LESS authority, never unclamped.
func TestAnUnreadableOrMisplacedDelegationGrantMeansNone(t *testing.T) {
	url, got := delegationServer(t)
	for _, token := range []string{"plug-bogus", "plug-misplaced"} {
		t.Run(token, func(t *testing.T) {
			m := spawnVia(t, url, token, `{"cwd":"/tmp","toolScope":"operator"}`, got)
			if _, has := m["toolScope"]; has {
				t.Errorf("%s delegated a tier from an unusable grant: %v", token, m)
			}
		})
	}
}
