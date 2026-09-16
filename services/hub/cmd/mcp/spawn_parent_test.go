package main

import (
	"context"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

func spawnParentParams(t *testing.T, label string, explicit string) map[string]any {
	t.Helper()
	hub := &spawnHub{confirms: true}
	b := &build{
		scope:  authtoken.ScopeOperator,
		allow:  authtoken.ScopeOperator.Methods(),
		caller: hub.Call,
	}
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, label)
	res, _, err := spawnWithGrants(ctx, b, "agents.spawn", spawnAgentIn{
		Cwd:             "/tmp",
		ParentSessionId: explicit,
	})
	if err != nil || res == nil || res.IsError {
		t.Fatalf("spawn failed: result=%v err=%v", res, err)
	}
	call := hub.call("agents.spawn")
	if call == nil {
		t.Fatal("agents.spawn was not called")
	}
	return call.params
}

func TestSpawnAgentDerivesParentFromTheCallingSession(t *testing.T) {
	params := spawnParentParams(t, "session:ordinary-parent", "")
	if got := params["parentSessionId"]; got != "ordinary-parent" {
		t.Fatalf("parentSessionId = %v, want host-derived ordinary-parent", got)
	}
}

func TestSpawnAgentOverwritesAConflictingCallerParent(t *testing.T) {
	params := spawnParentParams(t, "session:ordinary-parent", "some-other-session")
	if got := params["parentSessionId"]; got != "ordinary-parent" {
		t.Fatalf("conflicting parent escaped host derivation: %v", got)
	}
}

func TestSpawnAgentKeepsExplicitParentForStaticControllers(t *testing.T) {
	params := spawnParentParams(t, "static-api-token", "controller-parent")
	if got := params["parentSessionId"]; got != "controller-parent" {
		t.Fatalf("static controller parent = %v, want explicit value", got)
	}
}
