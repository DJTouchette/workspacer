package bus

import (
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

func installScopedTiers(srv *Server) {
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-view":
			return ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		case "tok-triage":
			return ScopedIdent{Scope: "triage", Methods: authtoken.ScopeTriage.Methods()}, true
		case "tok-operator":
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		return ScopedIdent{}, false
	})
}

func TestEnabledPluginHasAmbientOrdinaryAccess(t *testing.T) {
	cn := &conn{pluginID: "acme.tools"}
	for _, method := range []string{"agents.list", "agents.spawn", "fs.read", "fs.write", "config.get"} {
		if !cn.mayCall(method) {
			t.Errorf("enabled plugin cannot call ordinary capability %q", method)
		}
	}
	for _, raw := range []json.RawMessage{
		json.RawMessage(`{"path":"/etc/hosts"}`),
		json.RawMessage(`{"path":"/tmp/anywhere"}`),
	} {
		if err := cn.authorize("fs.read", raw); err != nil {
			t.Errorf("ambient filesystem access was confined for %s: %v", raw, err)
		}
	}
	if !cn.mayConsume("pty.bytes.session") || !cn.mayConsume("example.clock.tick") {
		t.Fatal("enabled plugin did not receive ordinary events ambiently")
	}
	if !cn.mayPublish("example.clock.tick") {
		t.Fatal("enabled plugin could not publish an ordinary plugin-defined event")
	}
}

func TestPluginIdentityStillProtectsHostStateAndProviderNamespace(t *testing.T) {
	cn := &conn{pluginID: "acme.tools", provides: []string{"acme.tools.*"}}
	for _, topic := range []string{"plugin.loaded", "layout.changed", "agent.snapshot"} {
		if cn.mayPublish(topic) {
			t.Errorf("plugin forged host-owned topic %q", topic)
		}
	}
	if cn.mayConsume("plugin.settings.changed") {
		t.Fatal("plugin consumed host-only settings event")
	}
	if !cn.mayProvide("acme.tools.search") {
		t.Fatal("plugin could not register in its own namespace")
	}
	if cn.mayProvide("agents.spawn") || cn.mayProvide("other.plugin.search") {
		t.Fatal("plugin registered a core or foreign provider method")
	}

	srv := NewServer(broker.New())
	srv.RegisterPluginToken("tok", "acme.tools", nil, capspec.EventGrants{
		Provides: []string{"acme.tools.*", "agents.*", "other.plugin.search"},
	})
	pi, ok := srv.lookupPluginToken("tok")
	if !ok {
		t.Fatal("plugin token was not registered")
	}
	if len(pi.events.Provides) != 1 || pi.events.Provides[0] != "acme.tools.*" {
		t.Fatalf("provider namespace filter = %v, want [acme.tools.*]", pi.events.Provides)
	}
}

func TestRevokedPluginLosesAmbientAccess(t *testing.T) {
	cn := &conn{pluginID: "acme.tools"}
	if !cn.mayCall("fs.read") {
		t.Fatal("floor: enabled plugin should have ambient access")
	}
	cn.revoked.Store(true)
	if cn.mayCall("fs.read") || cn.mayConsume("agent.state_changed") || cn.mayPublish("acme.tools.tick") || cn.mayProvide("acme.tools.search") {
		t.Fatal("revoked plugin retained bus access")
	}
}

func TestLegacyChildScopeDoesNotClampPluginSpawn(t *testing.T) {
	rt := newRouter()
	raw, err := rt.sanitizeSpawnParams(&conn{pluginID: "acme.tools"}, json.RawMessage(`{"cwd":"/tmp","toolScope":"operator","pluginTools":["legacy.selection"]}`))
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if got["toolScope"] != "operator" {
		t.Fatalf("plugin spawn tier was clamped: %v", got)
	}
	if _, ok := got["pluginTools"]; !ok {
		t.Fatalf("legacy pluginTools field was stripped instead of remaining inert: %v", got)
	}
}
