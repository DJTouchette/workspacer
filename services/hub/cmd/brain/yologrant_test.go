package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// Full-access dispatch, provider half. The hub router stamps `yoloGranted`
// onto an agents.spawn only after verifying the caller's token grant
// (internal/bus sanitizeSpawnParams; no caller can be the stamp's source). A
// stamped spawn's OWN bypass request — skipPermissions, or a bypass
// permissionMode — is honored instead of clamped; an unstamped spawn keeps the
// clamp byte-for-byte. Twin of profilegrant_test.go: the grants are minted the
// same way and stamped in the same place, they just speak for different
// escalations (which account vs. skipping approvals).

// TestYoloGrantedSpawnHonorsBypassOnThePtyPath: yoloGranted:true (hub-stamped)
// lets the request's skipPermissions ride onto the classic PTY argv.
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
		t.Errorf("a hub-stamped full-access spawn must honor skipPermissions, argv = %v", gotBody.Argv)
	}

	// The mode spelling of the same escalation is honored too (buildArgv rides
	// bypassPermissions on the skip flag).
	params = []byte(`{"cwd":"/tmp","transport":"pty","permissionMode":"bypassPermissions","yoloGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if !containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("a granted bypass permissionMode must be honored, argv = %v", gotBody.Argv)
	}
}

// TestYoloGrantedSpawnHonorsBypassOnTheManagedPath: same contract on the
// shipping default (claude.transport=stream → /sessions/spawn-managed): the
// wire `yolo` — always false for a bus caller until now — carries the granted
// request through.
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
		t.Errorf("granted managed spawn must carry yolo=true on the wire, got %+v", gotBody)
	}

	// Mode spelling on the managed leg: a granted bypassPermissions survives
	// into permission_mode instead of being dropped to default.
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","permissionMode":"bypassPermissions","yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if gotBody.PermissionMode != "bypassPermissions" {
		t.Errorf("granted bypass permissionMode was clamped on the managed leg, got %q", gotBody.PermissionMode)
	}
}

// TestUngrantedSpawnStillClampsEveryBypass: without the hub's stamp the
// doctrine is byte-for-byte yesterday's — skipPermissions forced off on both
// legs, a bypass permissionMode dropped. (The hub deletes any caller-supplied
// yoloGranted before the call gets here, so this leg is defense in depth
// against a stale or bypassed hub — same posture as the profile-grant twin.)
// NO SILENT DOWNGRADES, provider half (2026-08-26). Every agents.spawn answer
// says what the session ACTUALLY runs with (`fullAccess`), and names anything
// the caller asked for that did not survive (`escalationScrubbed`) — folding in
// what the hub router already took. Before this, a remote "full access" click
// came back indistinguishable from an ask-mode spawn, and the only record was a
// log line on the host.
