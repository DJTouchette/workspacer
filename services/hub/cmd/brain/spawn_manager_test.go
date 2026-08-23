package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// managerSpawnRig stands a fake claudemon up and returns a registry with a live
// metaStore, since newRegistry leaves meta nil (main.go wires it) and the whole
// point of `manager` headless is what lands in that store.
func managerSpawnRig(t *testing.T) (*registry, *spawnManagedReq, *spawnReq) {
	t.Helper()
	var managed spawnManagedReq
	var pty spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/sessions/spawn-managed" {
			_ = json.NewDecoder(r.Body).Decode(&managed)
			_ = json.NewEncoder(w).Encode(map[string]string{"session_id": managed.SessionID})
			return
		}
		_ = json.NewDecoder(r.Body).Decode(&pty)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": pty.SessionID})
	}))
	t.Cleanup(srv.Close)
	reg := newSpawnTestRegistry(t, srv.URL)
	reg.meta = newMetaStore()
	return reg, &managed, &pty
}

// TestManagerSpawnIsRecordedAsAWakeTarget is the behavioural half of the
// spawnParams mirror: `manager:true` must make the session nudge-eligible
// (spawnMeta.IsSupervisor, surfaced as the snapshot's isSupervisor), exactly as
// `supervisor:true` does. The desktop bug this mirrors (8cabb4a5) was precisely
// that a bus-spawned Fleet Manager came up WITHOUT this, so the worker-finished
// wake router never saw it and its workers finished into the void. Both spawn
// legs are covered because the desktop's hand-copied option literals drifted.
func TestManagerSpawnIsRecordedAsAWakeTarget(t *testing.T) {
	for _, tc := range []struct {
		name   string
		params string
	}{
		{"pty", `{"cwd":"/tmp","transport":"pty","manager":true}`},
		{"stream", `{"cwd":"/tmp","transport":"stream","manager":true}`},
		{"managed provider", `{"cwd":"/tmp","provider":"codex","manager":true}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			reg, _, _ := managerSpawnRig(t)
			res, err := reg.handle(context.Background(), "agents.spawn", []byte(tc.params))
			if err != nil {
				t.Fatal(err)
			}
			var got struct {
				SessionID string `json:"sessionId"`
			}
			if err := json.Unmarshal(res, &got); err != nil || got.SessionID == "" {
				t.Fatalf("no sessionId back: %v %s", err, res)
			}
			meta, ok := reg.meta.get(got.SessionID)
			if !ok {
				t.Fatalf("manager spawn recorded no spawnMeta at all — the wake router has nothing to key on")
			}
			if !meta.IsSupervisor {
				t.Errorf("manager spawn must be recorded IsSupervisor (a manager IS a supervisor for wake purposes), got %+v", meta)
			}
		})
	}
}

// TestManagerSpawnGrantsNoBypass is the security half. `manager` is caller-set
// (unlike yoloGranted/profileGranted, which only the hub router stamps), so a
// bus client can always assert it. It must therefore buy nothing but the wake
// subscription: the permission clamp has to behave byte-for-byte as it does for
// an ordinary spawn. If `manager` ever grows a privilege implication it must
// move behind the hub-verified stamp instead, and this test should fail loudly
// when someone wires one in.
func TestManagerSpawnGrantsNoBypass(t *testing.T) {
	reg, _, pty := managerSpawnRig(t)
	// The most aggressive thing an ungranted bus caller can ask for, with
	// `manager` asserted alongside in the hope it unlocks something.
	params := []byte(`{"cwd":"/tmp","transport":"pty","manager":true,"skipPermissions":true,"permissionMode":"bypassPermissions"}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	if containsStr(pty.Argv, "--dangerously-skip-permissions") {
		t.Errorf("manager:true must not unlock --dangerously-skip-permissions for an ungranted bus caller, got %v", pty.Argv)
	}
	if containsPair(pty.Argv, "--permission-mode", "bypassPermissions") {
		t.Errorf("manager:true must not unlock a bypass permission mode for an ungranted bus caller, got %v", pty.Argv)
	}
}

// TestManagerSpawnDoesNotSelfGrantOnTheManagedLeg is the same clamp on the leg
// that carries the bypass as a wire field rather than argv, so neither spawn
// path can become the manager-shaped escalation door.
func TestManagerSpawnDoesNotSelfGrantOnTheManagedLeg(t *testing.T) {
	reg, managed, _ := managerSpawnRig(t)
	params := []byte(`{"cwd":"/tmp","provider":"codex","manager":true,"skipPermissions":true}`)
	if _, err := reg.handle(context.Background(), "agents.spawn", params); err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(managed)
	var m map[string]any
	_ = json.Unmarshal(body, &m)
	for _, k := range []string{"yolo", "skip_permissions", "permission_mode"} {
		switch v := m[k].(type) {
		case bool:
			if v {
				t.Errorf("manager:true must not set %q on an ungranted managed spawn: %s", k, body)
			}
		case string:
			if isPermissionEscalation(v) {
				t.Errorf("manager:true must not set %q to a bypass mode on an ungranted managed spawn: %s", k, body)
			}
		}
	}
}
