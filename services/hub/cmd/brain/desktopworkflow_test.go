package main

import (
	"encoding/json"
	"testing"
)

func TestDesktopWorkflowOverlayPreservesLiveState(t *testing.T) {
	s := newSessionStore()
	s.set("s", json.RawMessage(`{"session_id":"s","status":"active","ambientState":"streaming","cwd":"/project","subagents":[{"id":"agent-child","description":"old"},{"id":"in-run"}]}`))
	s.setDesktopWorkflow("s", json.RawMessage(`{"runs":[{"id":"run","status":"running"}],"workflowAgentIds":["in-run"],"subagentActivity":{"child":{"description":"new","tokens":10}}}`))
	s.set("s", json.RawMessage(`{"session_id":"s","status":"ended","ambientState":"idle","cwd":"/new","subagents":[{"id":"agent-child"}]}`))
	raw, _ := s.get("s")
	var got struct {
		Status string `json:"status"`
		Cwd    string `json:"cwd"`
		Runs   []any  `json:"workflows"`
		Subs   []struct {
			Description string `json:"description"`
			Tokens      int    `json:"tokens"`
		} `json:"subagents"`
	}
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if got.Status != "ended" || got.Cwd != "/new" || len(got.Runs) != 1 || len(got.Subs) != 1 || got.Subs[0].Tokens != 10 || got.Subs[0].Description != "new" {
		t.Fatalf("overlay lost state: %s", raw)
	}
	s.remove("s")
	s.setDesktopWorkflow("s", json.RawMessage(`{"runs":[{}]}`))
	if _, ok := s.get("s"); ok {
		t.Fatal("late workflow update resurrected a dismissed session")
	}
}
