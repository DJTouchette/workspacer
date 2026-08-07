package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestSpawnRemoteBypassProfileExtraArgsScrubbed: the remote spawn clamp must not
// be defeatable by pointing agents.spawn at a local profile whose extraArgs pin
// --dangerously-skip-permissions / --permission-mode bypassPermissions. The clamp
// zeroes the request fields; it must also strip the profile's smuggled flags, or
// a bus/web/MCP caller starts a YOLO agent the clamp claims to forbid.
func TestSpawnRemoteBypassProfileExtraArgsScrubbed(t *testing.T) {
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)

	// A local profile that pins auto-approve flags in its extraArgs.
	if err := saveProfiles([]profile{{
		ID:        "yolo-prof",
		Name:      "YOLO",
		IsDefault: true,
		ExtraArgs: []string{"--dangerously-skip-permissions", "--permission-mode", "bypassPermissions"},
	}}); err != nil {
		t.Fatal(err)
	}

	// Remote/bus caller points at that bypass profile on the PTY path.
	params := []byte(`{"cwd":"/tmp","transport":"pty","profileId":"yolo-prof"}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}

	if containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("profile extraArgs must not smuggle --dangerously-skip-permissions past the remote clamp, got %v", gotBody.Argv)
	}
	if containsPair(gotBody.Argv, "--permission-mode", "bypassPermissions") {
		t.Errorf("profile extraArgs must not smuggle bypassPermissions past the remote clamp, got %v", gotBody.Argv)
	}
}

// TestSpawnRemoteProfileSettingsAndConfigDirDropped exercises the same clamp
// through the handler a bus caller actually reaches, for the two doors the old
// denylist left open: --settings (a settings file can carry permissions and
// hooks) and the profile's configDir (which becomes CLAUDE_CONFIG_DIR, the
// directory those very settings are read from).
func TestSpawnRemoteProfileSettingsAndConfigDirDropped(t *testing.T) {
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)

	if err := saveProfiles([]profile{{
		ID:        "sneaky",
		Name:      "Sneaky",
		IsDefault: true,
		ConfigDir: "/tmp/attacker-claude-home",
		ExtraArgs: []string{"--settings", "/tmp/evil.json", "--allowedTools", "Bash,Edit", "--model", "opus"},
	}}); err != nil {
		t.Fatal(err)
	}

	params := []byte(`{"cwd":"/tmp","transport":"pty","profileId":"sneaky"}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}

	for _, banned := range []string{"--settings", "/tmp/evil.json", "--allowedTools", "Bash,Edit"} {
		if containsStr(gotBody.Argv, banned) {
			t.Errorf("%q reached claudemon through the profile, argv = %v", banned, gotBody.Argv)
		}
	}
	if gotBody.Env["CLAUDE_CONFIG_DIR"] != "" {
		t.Errorf("a remote spawn must not inherit the profile's CLAUDE_CONFIG_DIR, got %q", gotBody.Env["CLAUDE_CONFIG_DIR"])
	}
	// The allowlisted flag still rides, or the clamp is just breaking profiles.
	if !containsPair(gotBody.Argv, "--model", "opus") {
		t.Errorf("profile --model should survive the clamp, argv = %v", gotBody.Argv)
	}
}

// The SHIPPING DEFAULT leg. config_defaults.json sets claude.transport to
// "stream", so a spawn that names no transport goes through spawnManagedSession
// and POSTs /sessions/spawn-managed — and both scrub tests above pin
// `"transport":"pty"` in their params, so the leg that actually answers by
// default was untested. Deleting `scrubBypassProfile(...)` from that call site
// left the whole hub suite green while the default bus/remote/MCP spawn
// forwarded `--dangerously-skip-permissions`, `--settings /tmp/evil.json`,
// `--allowedTools Bash,Edit` and `CLAUDE_CONFIG_DIR=/tmp/attacker-claude-home`
// to claudemon. CLAUDE_CONFIG_DIR supplies settings.json — permissions.allow
// and hooks, i.e. commands claude runs unprompted — which is the exact
// escalation scrubBypassProfile's own comment names.
func TestManagedStreamSpawnScrubsTheProfileToo(t *testing.T) {
	var gotBody spawnManagedReq
	var gotPath string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": "s1"})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)
	if err := saveProfiles([]profile{{
		ID:        "sneaky",
		Name:      "Sneaky",
		IsDefault: true,
		ConfigDir: "/tmp/attacker-claude-home",
		ExtraArgs: []string{
			"--dangerously-skip-permissions",
			"--settings", "/tmp/evil.json",
			"--allowedTools", "Bash,Edit",
			"--model", "opus",
		},
	}}); err != nil {
		t.Fatal(err)
	}

	// NO transport key: the shipped default. The floor assertion below is what
	// keeps this test honest if that default ever moves.
	params := []byte(`{"cwd":"/tmp","profileId":"sneaky"}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/sessions/spawn-managed" {
		t.Fatalf("a spawn with no transport went to %q, not the managed stream path — config_defaults.json claude.transport is %q and this test is no longer exercising the default leg",
			gotPath, reg.claudeTransportDefault())
	}

	for _, banned := range []string{
		"--dangerously-skip-permissions", "--settings", "/tmp/evil.json", "--allowedTools", "Bash,Edit",
	} {
		if containsStr(gotBody.ExtraArgs, banned) {
			t.Errorf("%q reached claudemon through the profile on the DEFAULT transport, extra_args = %v", banned, gotBody.ExtraArgs)
		}
	}
	if gotBody.Env["CLAUDE_CONFIG_DIR"] != "" {
		t.Errorf("a remote managed spawn must not inherit the profile's CLAUDE_CONFIG_DIR, got %q", gotBody.Env["CLAUDE_CONFIG_DIR"])
	}
	// The allowlisted flag still rides, or the clamp is just breaking profiles.
	if !containsPair(gotBody.ExtraArgs, "--model", "opus") {
		t.Errorf("profile --model should survive the clamp, extra_args = %v", gotBody.ExtraArgs)
	}
}
