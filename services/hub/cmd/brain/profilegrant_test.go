package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// Profile-aware dispatch, provider half (FLEET_MANAGER_SPIKE §6a). The hub
// router stamps `profileGranted` onto an agents.spawn only after verifying the
// caller's token grant (internal/bus sanitizeSpawnParams; no caller can be the
// stamp's source). A granted spawn keeps the LOCAL profile's CLAUDE_CONFIG_DIR
// — the account IS the configDir — while the bypass scrub stays exactly as
// strict, because the grant speaks for the account, never for skipping
// approvals.

// grantProfile is a locally-blessed account: a real configDir (only a LOCAL
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

// TestGrantedSpawnKeepsConfigDirOnThePtyPath: profileGranted:true (hub-stamped)
// carries the account onto the classic PTY argv spawn — and nothing else.
func TestGrantedSpawnKeepsConfigDirOnThePtyPath(t *testing.T) {
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
		t.Errorf("a GRANTED spawn must run under the profile's account (CLAUDE_CONFIG_DIR), got %q", got)
	}
	// The grant is about the account, not about approvals: every bypass smuggle
	// stays scrubbed, the allowlisted flag still rides.
	for _, banned := range []string{"--dangerously-skip-permissions", "--settings", "/tmp/evil.json", "--allowedTools", "Bash,Edit"} {
		if containsStr(gotBody.Argv, banned) {
			t.Errorf("%q survived onto a granted spawn's argv — the grant must not weaken the bypass scrub: %v", banned, gotBody.Argv)
		}
	}
	if !containsPair(gotBody.Argv, "--model", "opus[1m]") {
		t.Errorf("allowlisted profile flag should still ride a granted spawn, argv = %v", gotBody.Argv)
	}
	if gotBody.Model != "opus[1m]" || gotBody.ModelIdentity != "opus" || gotBody.ContextWindow == nil || *gotBody.ContextWindow != 1_000_000 {
		t.Errorf("PTY profile model pair = legacy %q identity %q window %v", gotBody.Model, gotBody.ModelIdentity, gotBody.ContextWindow)
	}
}

// TestGrantedSpawnKeepsConfigDirOnTheManagedPath: same contract on the shipping
// default (claude.transport=stream → /sessions/spawn-managed).
func TestGrantedSpawnKeepsConfigDirOnTheManagedPath(t *testing.T) {
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
		t.Errorf("granted managed spawn must carry the account's CLAUDE_CONFIG_DIR, got %q", got)
	}
	for _, banned := range []string{"--dangerously-skip-permissions", "--settings", "--allowedTools"} {
		if containsStr(gotBody.ExtraArgs, banned) {
			t.Errorf("%q survived onto a granted managed spawn — the grant must not weaken the bypass scrub: %v", banned, gotBody.ExtraArgs)
		}
	}
	if gotBody.Model != "opus[1m]" || gotBody.ModelIdentity != "opus" || gotBody.ContextWindow == nil || *gotBody.ContextWindow != 1_000_000 {
		t.Errorf("managed profile model pair = legacy %q identity %q window %v", gotBody.Model, gotBody.ModelIdentity, gotBody.ContextWindow)
	}
}

// TestUngrantedSpawnStillDropsConfigDir: without the hub's stamp the doctrine
// is byte-for-byte yesterday's — profileId resolves, configDir does not ride.
// (The hub additionally strips profileId itself from ungranted callers, so
// this leg is defense in depth against a stale or bypassed hub.)
func TestUngrantedSpawnStillDropsConfigDir(t *testing.T) {
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)
	saveGrantProfile(t)

	params := []byte(`{"cwd":"/tmp","transport":"pty","profileId":"work"}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if got := gotBody.Env["CLAUDE_CONFIG_DIR"]; got != "" {
		t.Errorf("an ungranted spawn inherited the profile's CLAUDE_CONFIG_DIR: %q", got)
	}
}
