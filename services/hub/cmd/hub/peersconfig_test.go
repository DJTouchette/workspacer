package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/federation"
)

func TestPeerConfigOwnerRedactionKeepClearAndHotReload(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	controller, err := federation.NewController(ctx, broker.New(), nil)
	if err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(t.TempDir(), "peers.json")
	p := &peerConfig{path: file, controller: controller}
	owner := bus.CallerIdentity{AuthenticatedHost: true, Trusted: true, Scope: "operator"}
	save := func(raw string) {
		t.Helper()
		if _, err := peerConfigSave(p)(owner, json.RawMessage(raw)); err != nil {
			t.Fatal(err)
		}
	}
	save(`{"peers":[{"name":"work","url":"ws://127.0.0.1:1/bus","token":"private-test-credential","dispatch":true}]}`)
	if !controller.DispatchEnabled("work") {
		t.Fatal("peer config did not apply")
	}
	redacted, err := peerConfigRead(p)(owner, nil)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(redacted)
	if strings.Contains(string(encoded), "private-test-credential") || !strings.Contains(string(encoded), `"hasToken":true`) {
		t.Fatalf("invalid redacted read: %s", encoded)
	}
	save(`{"peers":[{"name":"work","url":"ws://127.0.0.1:1/bus"}]}`)
	records, err := federation.LoadPeersFile(file)
	if err != nil {
		t.Fatal(err)
	}
	if records[0].Token != "private-test-credential" || records[0].Dispatch {
		t.Fatal("omitted token/dispatch semantics drifted")
	}
	save(`{"peers":[{"name":"work","url":"ws://127.0.0.1:1/bus","token":""}]}`)
	records, _ = federation.LoadPeersFile(file)
	if records[0].Token != "" {
		t.Fatal("explicit clear did not clear")
	}
	for _, caller := range []bus.CallerIdentity{{Trusted: true, Scope: "operator"}, {Scope: "provider"}, {PluginID: "plugin"}} {
		if _, err := peerConfigRead(p)(caller, nil); err == nil {
			t.Fatal("non-owner read peer config")
		}
		if _, err := peerConfigSave(p)(caller, json.RawMessage(`{"peers":[]}`)); err == nil {
			t.Fatal("non-owner edited peer config")
		}
	}
	before, _ := os.ReadFile(file)
	if _, err := peerConfigSave(p)(owner, json.RawMessage(`{"peers":[{"name":"work","url":"https://invalid"}]}`)); err == nil {
		t.Fatal("invalid peer accepted")
	}
	after, _ := os.ReadFile(file)
	if string(after) != string(before) {
		t.Fatal("invalid save changed persisted peers")
	}
	save(`{"peers":[]}`)
	if len(controller.Peers()) != 0 {
		t.Fatal("removed peer stayed configured")
	}
}

func TestJobAdministrationRequiresActualOwner(t *testing.T) {
	for _, method := range []string{"jobs.list", "jobs.upsert", "jobs.run", "jobs.remove", "jobs.history"} {
		if jobsTrusted(method, bus.CallerIdentity{Trusted: true, Scope: "operator"}) == nil {
			t.Fatalf("scoped worker can invoke %s", method)
		}
		if err := jobsTrusted(method, bus.CallerIdentity{AuthenticatedHost: true, Trusted: true, Scope: "operator"}); err != nil {
			t.Fatal(err)
		}
	}
}
