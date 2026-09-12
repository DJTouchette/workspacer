package main

import (
	"context"
	"encoding/json"
	"slices"
	"testing"
)

func TestLiveWorkspaceRootsFollowSnapshotChangesImmediately(t *testing.T) {
	r := newRegistry(newClaudemonClient("http://unused"))
	r.store = newSessionStore()
	ctx := context.Background()
	directory := t.TempDir()
	if slices.Contains(r.agentCwds(ctx), directory) {
		t.Fatal("unobserved root admitted")
	}
	row, _ := json.Marshal(map[string]any{"session_id": "new-session", "cwd": directory, "mode": "working"})
	r.store.set("new-session", row)
	if !slices.Contains(r.agentCwds(ctx), directory) {
		t.Fatal("new live workspace was hidden by a stale cache")
	}
	ended, _ := json.Marshal(map[string]any{"session_id": "new-session", "cwd": directory, "mode": "stopped"})
	r.store.set("new-session", ended)
	if slices.Contains(r.agentCwds(ctx), directory) {
		t.Fatal("ended workspace retained a cached root grant")
	}
}
