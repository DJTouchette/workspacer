package main

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestFleetWorkflowSelectionPreservation(t *testing.T) {
	current := map[string]any{"agents": map[string]any{"defaultWorkflowId": "research", "workflowSelectionRevision": float64(3)}, "projects": map[string]any{"/repo": map[string]any{"workflowId": "custom"}}}
	patch := map[string]any{"agents": map[string]any{"fleetRoot": "/work"}, "projects": map[string]any{"/repo": map[string]any{"label": "New label"}}}
	if err := preserveFleetWorkflowSelections(current, patch); err != nil {
		t.Fatal(err)
	}
	if got := patch["projects"].(map[string]any)["/repo"].(map[string]any)["workflowId"]; got != "custom" {
		t.Fatal(got)
	}
	for _, bad := range []map[string]any{{"agents": map[string]any{"defaultWorkflowId": "other"}}, {"projects": map[string]any{}}, {"projects": map[string]any{"/repo": map[string]any{"workflowId": map[string]any{"evil": true}}}}} {
		if err := preserveFleetWorkflowSelections(current, bad); err == nil {
			t.Fatalf("accepted %+v", bad)
		}
	}
	if !reflect.DeepEqual(current["agents"], map[string]any{"defaultWorkflowId": "research", "workflowSelectionRevision": float64(3)}) {
		t.Fatal("mutated original")
	}
}

func TestFleetWorkflowsHeadlessUnavailableBeforeLaunch(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)
	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"workflowStepId":"implement","provider":"codex"}`)); err == nil {
		t.Fatal("headless accepted workflow step")
	}
	res, err := reg.handle(context.Background(), "fleetWorkflows.request", []byte(`{"op":"start","cwd":"/tmp"}`))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(res), "unavailable") {
		t.Fatal(string(res))
	}
	if len(rec.calls("/sessions/spawn-managed")) != 0 {
		t.Fatal("phantom launch")
	}
}

func TestFleetSelectionSurvivesActualHeadlessConfigRoundtrip(t *testing.T) {
	dir := tempConfigHome(t)
	file := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(file), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(file, []byte("agents:\n  defaultWorkflowId: research\n  workflowSelectionRevision: 3\nprojects:\n  /repo:\n    workflowId: custom\n"), 0600); err != nil {
		t.Fatal(err)
	}
	c := newConfigService()
	if _, err := c.save(map[string]any{"projects": map[string]any{"/repo": map[string]any{"label": "After Go save"}}, "agents": map[string]any{"workflowSelectionRevision": float64(3)}}); err != nil {
		t.Fatal(err)
	}
	saved := newConfigService().get()
	agents := saved["agents"].(map[string]any)
	project := saved["projects"].(map[string]any)["/repo"].(map[string]any)
	if agents["defaultWorkflowId"] != "research" || project["workflowId"] != "custom" || project["label"] != "After Go save" {
		t.Fatal(saved)
	}
	if _, err := c.save(map[string]any{"agents": map[string]any{"workflowSelectionRevision": "3"}}); err == nil {
		t.Fatal("accepted type-confused revision")
	}
}
