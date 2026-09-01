package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/modelselection"
)

func TestCodexSpawnContextDefaultOverrideAndResume(t *testing.T) {
	var bodies []spawnManagedReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body spawnManagedReq
		_ = json.NewDecoder(r.Body).Decode(&body)
		bodies = append(bodies, body)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": "s1"})
	}))
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	for _, params := range []string{
		`{"cwd":"/tmp","provider":"codex"}`,
		`{"cwd":"/tmp","provider":"codex","contextWindow":400000}`,
		`{"cwd":"/tmp","provider":"codex","resumeSessionId":"old"}`,
	} {
		if _, err := reg.handle(context.Background(), "agents.spawn", []byte(params)); err != nil {
			t.Fatal(err)
		}
	}
	if bodies[0].ContextWindow == nil || *bodies[0].ContextWindow != modelselection.DefaultCodexContextWindow {
		t.Fatalf("fresh default = %v, want 1M", bodies[0].ContextWindow)
	}
	if bodies[1].ContextWindow == nil || *bodies[1].ContextWindow != 400_000 {
		t.Fatalf("explicit override = %v", bodies[1].ContextWindow)
	}
	if bodies[2].ContextWindow != nil {
		t.Fatalf("resume must preserve persisted/provider state, got %v", *bodies[2].ContextWindow)
	}
}

func TestBrainRefusesUnsupportedProviderContext(t *testing.T) {
	reg := newSpawnTestRegistry(t, "http://127.0.0.1:1")
	_, err := reg.handle(context.Background(), "agents.spawn", []byte(
		`{"cwd":"/tmp","provider":"opencode","contextWindow":1000000}`,
	))
	if err == nil || !strings.Contains(err.Error(), "unsupported-context-window") {
		t.Fatalf("got %v, want stable unsupported-context-window", err)
	}
}
