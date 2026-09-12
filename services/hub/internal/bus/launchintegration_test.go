package bus

import (
	"encoding/json"
	"strconv"
	"testing"
)

func TestLaunchPreparationBoundToActiveOwnerSpawn(t *testing.T) {
	url, srv, token := providerTokenServer(t)
	provider := dialClientToken(t, url, token)
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	owner := dialClientToken(t, url, "host-secret")
	// providerTokenServer uses this explicit host token; the callback cannot
	// borrow the provider's own registration authority as owner identity.
	owner.send(Frame{Op: "call", ID: "spawn", Method: "agents.spawn", Params: json.RawMessage(`{"cwd":"/project","launchIntegrationId":"example.integration"}`)})
	incoming := provider.readUntil("call")
	var params map[string]any
	_ = json.Unmarshal(incoming.Params, &params)
	if params["launchIntegrationGranted"] != true {
		t.Fatal("owner integration was not stamped")
	}
	gid, err := strconv.ParseUint(incoming.ID, 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	srv.router.mu.Lock()
	pending := srv.router.pending[gid]
	providerID := pending.providerID
	srv.router.mu.Unlock()
	caller := CallerIdentity{ConnID: providerID, Scope: "provider"}
	if err := srv.AuthorizeLaunchPreparation(caller, incoming.ID, "example.integration"); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []struct {
		caller       CallerIdentity
		call, plugin string
	}{
		{CallerIdentity{ConnID: providerID + 100}, incoming.ID, "example.integration"},
		{caller, incoming.ID, "different.integration"},
		{caller, "999999", "example.integration"},
	} {
		if srv.AuthorizeLaunchPreparation(bad.caller, bad.call, bad.plugin) == nil {
			t.Fatal("callback escaped active spawn binding")
		}
	}
	pending.caller.revoked.Store(true)
	if srv.AuthorizeLaunchPreparation(caller, incoming.ID, "example.integration") == nil {
		t.Fatal("revoked owner retained launch authority")
	}
	pending.caller.revoked.Store(false)
	provider.send(Frame{Op: "result", ID: incoming.ID, Result: json.RawMessage(`{"sessionId":"session"}`)})
	owner.readUntil("result")
	if srv.AuthorizeLaunchPreparation(caller, incoming.ID, "example.integration") == nil {
		t.Fatal("completed spawn retained callback authority")
	}
}

func TestScopedCallerCannotForgeLaunchIntegrationGrant(t *testing.T) {
	rt := newRouter()
	for _, caller := range []*conn{{trusted: true, viaScopedToken: true}, {trusted: true, viaScopedToken: true, facadeAuthority: true}, {trusted: true, federated: true}} {
		if _, err := rt.sanitizeSpawnParams(caller, json.RawMessage(`{"launchIntegrationId":"example.integration","launchIntegrationGranted":true}`)); err == nil {
			t.Fatal("non-owner launched integration")
		}
	}
	data, err := rt.sanitizeSpawnParams(&conn{trusted: true, authenticatedHost: true}, json.RawMessage(`{"launchIntegrationGranted":true}`))
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]any
	_ = json.Unmarshal(data, &fields)
	if _, ok := fields["launchIntegrationGranted"]; ok {
		t.Fatal("unstamped grant survived without a selected integration")
	}
}
