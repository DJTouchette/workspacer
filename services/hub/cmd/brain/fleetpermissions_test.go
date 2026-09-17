package main

import (
	"context"
	"encoding/json"
	"testing"
)

func TestFleetFullAccessLaunches(t *testing.T) {
	for _, provider := range []string{"claude", "codex", "copilot"} {
		for _, role := range []string{"manager", "worker", "grandchild", "ordinary", "disabled", "cycle"} {
			t.Run(provider+"/"+role, func(t *testing.T) {
				r := newSpawnTestRegistry(t, spawnLaunchTestServer(t).URL)
				r.meta = newMetaStore()
				r.meta.set("manager", spawnMeta{IsWakeTarget: true})
				r.meta.set("worker", spawnMeta{ParentSessionID: "manager"})
				r.meta.set("cycle", spawnMeta{ParentSessionID: "cycle"})
				r.cfg.save(map[string]any{"agents": map[string]any{"fleetFullAccess": role != "disabled"}})
				params := map[string]any{"provider": provider, "cwd": t.TempDir(), "transport": "stream", "skipPermissions": false}
				if role == "manager" || role == "disabled" {
					params["manager"] = true
				}
				if role == "worker" {
					params["parentSessionId"] = "manager"
				}
				if role == "grandchild" {
					params["parentSessionId"] = "worker"
				}
				if role == "cycle" {
					params["parentSessionId"] = "cycle"
				}
				raw, _ := json.Marshal(params)
				result, err := r.handle(context.Background(), "agents.spawn", raw)
				if err != nil {
					t.Fatal(err)
				}
				var out map[string]any
				if err := json.Unmarshal(result, &out); err != nil {
					t.Fatal(err)
				}
				want := role == "manager" || role == "worker" || role == "grandchild"
				if out["fullAccess"] != want {
					t.Fatalf("fullAccess = %v, want %v", out["fullAccess"], want)
				}
				meta, _ := r.meta.get(out["sessionId"].(string))
				mode := "ask"
				if provider == "claude" {
					mode = "default"
				}
				if want {
					mode = "yolo"
					if provider == "claude" {
						mode = "bypassPermissions"
					}
				}
				if meta.LaunchPermissionMode != mode {
					t.Fatalf("mode = %q, want %q", meta.LaunchPermissionMode, mode)
				}
			})
		}
	}
}
