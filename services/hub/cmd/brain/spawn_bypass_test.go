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
