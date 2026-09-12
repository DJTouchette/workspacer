package bus

import (
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	"slices"
	"testing"
)

func TestDesktopServicesRetainOwnerAuthority(t *testing.T) {
	owner := &conn{trusted: true, authenticatedHost: true}
	operator := &conn{trusted: true, scope: "operator", viaScopedToken: true, scopeMethods: []string{"*"}}
	provider := &conn{scope: "provider", provides: []string{"*"}, scopeMethods: []string{"*"}}
	plugin := &conn{caps: map[string]capGrant{}}
	for _, method := range append(append([]string{}, capspec.DesktopServices...), "files.receiveUpload") {
		if !owner.mayCall(method) {
			t.Errorf("owner cannot use %s", method)
		}
		for _, caller := range []*conn{operator, provider, plugin} {
			if caller.mayCall(method) {
				t.Errorf("non-owner can call %s", method)
			}
		}
		if !provider.mayProvide(method) {
			t.Errorf("worker cannot provide %s", method)
		}
	}
	if owner.mayCall("desktop.internal.acceptSpawn") {
		t.Fatal("unregistered private action is reachable")
	}
	owner.revoked.Store(true)
	if owner.mayCall(capspec.DesktopServices[0]) {
		t.Fatal("revoked owner retained desktop access")
	}
}

func TestDesktopServiceManifest(t *testing.T) {
	data, err := sweepguard.ReadRepoFile("contracts", "desktop-service-methods.json")
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		OwnerMethods   []string `json:"ownerMethods"`
		AssetMethods   []string `json:"assetMethods"`
		AuthorityCases []struct {
			Identity string `json:"identity"`
			Owner    bool   `json:"owner"`
		} `json:"authorityCases"`
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(manifest.OwnerMethods, capspec.DesktopServices) || !slices.Equal(manifest.AssetMethods, capspec.UIAssetServices) {
		t.Fatal("generated registries drifted from the shared manifest")
	}
	callers := map[string]*conn{"owner": {trusted: true, authenticatedHost: true}, "operator": {trusted: true, viaScopedToken: true}, "facade": {trusted: true, viaScopedToken: true, facadeAuthority: true}, "provider": {scope: "provider", provides: []string{"*"}}}
	for _, row := range manifest.AuthorityCases {
		caller := callers[row.Identity]
		if caller == nil {
			t.Fatalf("unhandled authority fixture %s", row.Identity)
		}
		for _, method := range manifest.OwnerMethods {
			if caller.mayCall(method) != row.Owner {
				t.Fatalf("%s authority for %s", row.Identity, method)
			}
		}
	}
}
