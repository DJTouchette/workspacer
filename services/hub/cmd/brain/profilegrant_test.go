package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// Legacy profileGranted compatibility, provider half. The stamp is inert;
// authenticated spawns select local profiles directly.

// grantProfile is a legacy-named fixture with a real configDir (only a LOCAL
// write can set one — profilesAdd/Update scrub it from bus writers), plus
// extraArgs that mix one legitimate pin with every smuggle the allowlist
// exists to drop.
func saveGrantProfile(t *testing.T) {
	t.Helper()
	if err := saveProfiles([]profile{{
		ID:        "work",
		Name:      "Work",
		IsDefault: true,
		ConfigDir: "/home/user/.claude-work",
		ExtraArgs: []string{
			"--model", "opus[1m]",
			"--dangerously-skip-permissions",
			"--settings", "/tmp/evil.json",
			"--allowedTools", "Bash,Edit",
		},
		MCPItemIDs: []string{"mcp-item"},
	}}); err != nil {
		t.Fatal(err)
	}
}

// The selected account flows onto the classic PTY argv spawn.
func TestSpawnKeepsSelectedConfigDirOnThePtyPath(t *testing.T) {
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)
	saveGrantProfile(t)

	params := []byte(`{"cwd":"/tmp","transport":"pty","profileId":"work","profileGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}

	if got := gotBody.Env["CLAUDE_CONFIG_DIR"]; got != "/home/user/.claude-work" {
		t.Errorf("selected spawn must run under the profile's account (CLAUDE_CONFIG_DIR), got %q", got)
	}
	for _, expected := range []string{"--dangerously-skip-permissions", "--settings", "/tmp/evil.json", "--allowedTools", "Bash,Edit"} {
		if !containsStr(gotBody.Argv, expected) {
			t.Errorf("profile argument %q did not flow to the spawn: %v", expected, gotBody.Argv)
		}
	}
	if !containsPair(gotBody.Argv, "--model", "opus[1m]") {
		t.Errorf("profile flag should ride the selected spawn, argv = %v", gotBody.Argv)
	}
	if gotBody.Model != "opus[1m]" || gotBody.ModelIdentity != "opus" || gotBody.ContextWindow == nil || *gotBody.ContextWindow != 1_000_000 {
		t.Errorf("PTY profile model pair = legacy %q identity %q window %v", gotBody.Model, gotBody.ModelIdentity, gotBody.ContextWindow)
	}
}

// Same selected-profile contract on the shipping
// default (claude.transport=stream → /sessions/spawn-managed).
func TestSpawnKeepsSelectedConfigDirOnTheManagedPath(t *testing.T) {
	var gotBody spawnManagedReq
	var gotPath string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": "s1"})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)
	saveGrantProfile(t)

	params := []byte(`{"cwd":"/tmp","profileId":"work","profileGranted":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/sessions/spawn-managed" {
		t.Fatalf("default-transport spawn went to %q — this test is no longer exercising the managed leg", gotPath)
	}
	if got := gotBody.Env["CLAUDE_CONFIG_DIR"]; got != "/home/user/.claude-work" {
		t.Errorf("managed spawn must carry the account's CLAUDE_CONFIG_DIR, got %q", got)
	}
	for _, expected := range []string{"--dangerously-skip-permissions", "--settings", "--allowedTools"} {
		if !containsStr(gotBody.ExtraArgs, expected) {
			t.Errorf("profile argument %q did not flow to the managed spawn: %v", expected, gotBody.ExtraArgs)
		}
	}
	if gotBody.Model != "opus[1m]" || gotBody.ModelIdentity != "opus" || gotBody.ContextWindow == nil || *gotBody.ContextWindow != 1_000_000 {
		t.Errorf("managed profile model pair = legacy %q identity %q window %v", gotBody.Model, gotBody.ModelIdentity, gotBody.ContextWindow)
	}
}

// Legacy profileGranted spellings do not affect profile selection.
