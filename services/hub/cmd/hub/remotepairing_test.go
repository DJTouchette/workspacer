package main

import (
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func TestRemotePairingsRequireOwnerAndPreserveInfrastructure(t *testing.T) {
	p := &remotePairings{path: filepath.Join(t.TempDir(), "tokens.json")}
	provider, err := authtoken.Mint(p.path, authtoken.ScopeProvider, "node")
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range []bus.CallerIdentity{{}, {Scope: "triage"}, {Trusted: true, Scope: "operator"}, {PluginID: "p"}, {Scope: "provider"}} {
		for _, handler := range []bus.LocalIdentHandler{remoteTokensList(p), remoteTokenGetOrCreate(p), remoteTokenRevoke(p)} {
			if _, err := handler(c, json.RawMessage(`{"scope":"operator"}`)); err == nil {
				t.Fatalf("accepted non-owner %+v", c)
			}
		}
	}
	owner := bus.CallerIdentity{Trusted: true, AuthenticatedHost: true, Scope: "operator"}
	if _, err := remoteTokenGetOrCreate(p)(owner, json.RawMessage(`{"scope":"provider"}`)); err == nil {
		t.Fatal("minted provider")
	}
	raw := json.RawMessage(`{"scope":"operator","yoloAllowed":true,"provides":["*"],"profilesAllowed":["private"]}`)
	a, err := remoteTokenGetOrCreate(p)(owner, raw)
	if err != nil {
		t.Fatal(err)
	}
	b, err := remoteTokenGetOrCreate(p)(owner, raw)
	if err != nil {
		t.Fatal(err)
	}
	rec := a.(authtoken.Record)
	if rec.Token != b.(authtoken.Record).Token || rec.YoloAllowed || len(rec.Provides) > 0 || len(rec.ProfilesAllowed) > 0 {
		t.Fatal("reuse or grant confinement failed")
	}
	list, err := remoteTokensList(p)(owner, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(list.([]authtoken.Record)) != 1 {
		t.Fatal("infrastructure was exposed")
	}
	request, _ := json.Marshal(map[string]string{"token": provider.Token})
	if _, err := remoteTokenRevoke(p)(owner, request); err == nil {
		t.Fatal("revoked provider")
	}
	request, _ = json.Marshal(map[string]string{"token": rec.Token})
	if _, err := remoteTokenRevoke(p)(owner, request); err != nil {
		t.Fatal(err)
	}
	recs, err := authtoken.Load(p.path)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 1 || recs[0].Token != provider.Token || len(recs[0].Provides) != 1 {
		t.Fatal("provider grant was changed")
	}
}
