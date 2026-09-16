package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// Legacy yoloGranted compatibility, provider half. The stamp is inert;
// authenticated provider permission requests flow through unchanged.

// The legacy stamp cannot prevent the explicit PTY permission request flowing.
func TestYoloGrantedSpawnHonorsBypassOnThePtyPath(t *testing.T) {
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)

	params := []byte(`{"cwd":"/tmp","transport":"pty","skipPermissions":true,"yoloGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if !containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("spawn must honor skipPermissions, argv = %v", gotBody.Argv)
	}

	// The mode spelling of the same escalation is honored too (buildArgv rides
	// bypassPermissions on the skip flag).
	params = []byte(`{"cwd":"/tmp","transport":"pty","permissionMode":"bypassPermissions","yoloGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if !containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("bypass permissionMode must be honored, argv = %v", gotBody.Argv)
	}
}

// The same provider-native request flows through on the managed path.
func TestYoloGrantedSpawnHonorsBypassOnTheManagedPath(t *testing.T) {
	var gotBody spawnManagedReq
	var gotPath string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": "s1"})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)

	params := []byte(`{"cwd":"/tmp","skipPermissions":true,"yoloGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/sessions/spawn-managed" {
		t.Fatalf("default-transport spawn went to %q — this test is no longer exercising the managed leg", gotPath)
	}
	if !gotBody.Yolo {
		t.Errorf("managed spawn must carry yolo=true on the wire, got %+v", gotBody)
	}

	// Mode spelling on the managed leg: bypassPermissions survives
	// into permission_mode instead of being dropped to default.
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","permissionMode":"bypassPermissions","yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if gotBody.PermissionMode != "bypassPermissions" {
		t.Errorf("bypass permissionMode changed on the managed leg, got %q", gotBody.PermissionMode)
	}
}

// NO SILENT DOWNGRADES, provider half (2026-08-26). Every agents.spawn answer
// says what the session ACTUALLY runs with (`fullAccess`), and names anything
// the caller asked for that did not survive (`escalationScrubbed`) — folding in
// what the hub router already took. Before this, a remote "full access" click
// came back indistinguishable from an ask-mode spawn, and the only record was a
// log line on the host.
