package main

import (
	"encoding/json"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func TestNetworkAdminPrivateBrokerAndOwnerGate(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix supervisor broker is for combined Linux deployment")
	}
	socket := filepath.Join(t.TempDir(), "network.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	_ = os.Chmod(socket, 0600)
	requests := make(chan string, 8)
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer network-fixture" {
			t.Error("missing private network authentication")
			w.WriteHeader(401)
			return
		}
		requests <- r.URL.Path
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/status" {
			_ = json.NewEncoder(w).Encode(map[string]any{"available": true, "magicName": "fixture.tailnet.ts.net", "serveActive": true, "canServe": true})
			return
		}
		var p map[string]any
		_ = json.NewDecoder(r.Body).Decode(&p)
		if len(p) != 1 || p["enabled"] != false {
			t.Error("network callback changed requested action")
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	})}
	defer server.Close()
	go server.Serve(listener)
	tokenFile := filepath.Join(t.TempDir(), "network-token")
	if err := os.WriteFile(tokenFile, []byte("network-fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	admin := &networkAdmin{socket: socket, tokenFile: tokenFile, port: 7895}
	owner := bus.CallerIdentity{AuthenticatedHost: true, Trusted: true, Scope: "operator"}
	value, err := networkInfo(admin)(owner, nil)
	if err != nil {
		t.Fatal(err)
	}
	data, _ := json.Marshal(value)
	if string(data) == "null" {
		t.Fatal("missing actual Tailscale state")
	}
	if _, err := setSharing(admin)(owner, json.RawMessage(`{"enabled":false}`)); err != nil {
		t.Fatal(err)
	}
	if <-requests != "/status" || <-requests != "/serve" {
		t.Fatal("wrong broker endpoints")
	}
	for _, c := range []bus.CallerIdentity{{Trusted: true, Scope: "operator"}, {Scope: "provider"}, {PluginID: "plugin"}} {
		if _, err := networkServe(admin)(c, json.RawMessage(`{"enabled":false}`)); err == nil {
			t.Fatal("non-owner changed network")
		}
		if _, err := networkInfo(admin)(c, nil); err == nil {
			t.Fatal("non-owner reached private broker")
		}
	}
	select {
	case <-requests:
		t.Fatal("denied caller reached private supervisor")
	default:
	}
}
