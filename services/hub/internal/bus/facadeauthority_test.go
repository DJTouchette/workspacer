package bus

import (
	"context"
	"encoding/json"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

func TestFacadeAuthorityOnlyDelegatesLocalSessionIdentity(t *testing.T) {
	rt := newRouter()
	cases := []struct {
		name   string
		caller *conn
		allow  bool
	}{
		{"owner", &conn{trusted: true, authenticatedHost: true}, true},
		{"facade", &conn{trusted: true, viaScopedToken: true, facadeAuthority: true}, true},
		{"operator", &conn{trusted: true, viaScopedToken: true}, false},
		{"provider", &conn{scope: "provider", facadeAuthority: true}, false},
		{"view", &conn{scope: "view", facadeAuthority: true}, false},
		{"plugin", &conn{trusted: true, pluginID: "plugin", facadeAuthority: true}, false},
		{"peer", &conn{trusted: true, viaScopedToken: true, facadeAuthority: true, federated: true}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			raw := json.RawMessage(`{"parentSessionId":"manager","dispatchOwnerSessionId":"manager","retrySourceSessionId":"worker","facadeAuthority":true}`)
			got, err := rt.sanitizeSpawnParams(tc.caller, raw)
			if err != nil {
				t.Fatal(err)
			}
			var fields map[string]any
			if err := json.Unmarshal(got, &fields); err != nil {
				t.Fatal(err)
			}
			if (fields["dispatchOwnerSessionId"] == "manager") != tc.allow {
				t.Fatalf("dispatch provenance: %s", got)
			}
			if (fields["retrySourceSessionId"] == "worker") != tc.allow {
				t.Fatalf("retry provenance: %s", got)
			}
			_, err = rt.sanitizeCallParams(tc.caller, "fleetWorkflows.request", json.RawMessage(`{"op":"start","callerSessionId":"manager"}`))
			if (err == nil) != tc.allow {
				t.Fatalf("workflow authority: %v", err)
			}
			if tc.name == "facade" {
				for _, method := range capspec.DesktopServices {
					if tc.caller.mayCall(method) {
						t.Fatalf("facade acquired owner method %s", method)
					}
				}
				if tc.caller.mayUseProfile("private-account") {
					t.Fatal("facade acquired profile grant")
				}
				tc.caller.revoked.Store(true)
				if tc.caller.mayAssertLocalSession() {
					t.Fatal("revoked facade retained authority")
				}
			}
		})
	}
}

func TestFacadeAuthorityRevocationClosesLiveConnection(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()
	url, srv := rpcServerWith(t)
	srv.SetToken("host")
	var granted atomic.Bool
	granted.Store(true)
	srv.SetScopedTokenLookup(func(token string) (ScopedIdent, bool) {
		return ScopedIdent{Scope: "operator", Methods: []string{"*"}, FacadeAuthority: granted.Load()}, token == "facade"
	})
	client := dialClientToken(t, url, "facade")
	client.send(Frame{Op: "call", ID: "floor", Method: "fleetWorkflows.request", Params: json.RawMessage(`{"op":"list"}`)})
	// No provider is installed: reaching this error proves the sanitizer admitted it.
	reply := client.readUntil("error")
	if reply.Error != "no provider for fleetWorkflows.request" {
		t.Fatalf("floor: %+v", reply)
	}
	granted.Store(false)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	for {
		if _, _, err := client.ws.Read(ctx); err != nil {
			if ctx.Err() != nil {
				t.Fatal("revoked facade socket stayed open")
			}
			break
		}
	}
}
