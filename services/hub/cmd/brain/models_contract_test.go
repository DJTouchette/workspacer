package main

// The claude.listModels contract, shared with apps/desktop's claudeModels.ts.
//
// This method is answered by the BRAIN for every web / mobile / remote / MCP
// client (it is in catalogMethods()) and by the desktop over IPC, and until this
// fixture the two returned different alias sets, different labels, no context
// badges on one side and no defaultPermissionMode on one side — with each
// suite asserting its own contradictory answer.

import (
	"encoding/json"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

type modelCatalogCase struct {
	Name   string `json:"name"`
	Config struct {
		DefaultModel           string   `json:"defaultModel"`
		ContextWindow          *uint64  `json:"contextWindow"`
		SkipPermissionsDefault bool     `json:"skipPermissionsDefault"`
		DefaultPermissionMode  string   `json:"defaultPermissionMode"`
		SeenModels             []string `json:"seenModels"`
	} `json:"config"`
	Live     []string         `json:"live"`
	Expected listModelsResult `json:"expected"`
	Why      string           `json:"why"`
}

func TestClaudeModelCatalogContractCases(t *testing.T) {
	const path = "contracts/claude-model-catalog-cases.json"
	raw := mustReadRepoFile(t, "contracts", "claude-model-catalog-cases.json")
	var fx struct {
		Cases []modelCatalogCase `json:"cases"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", path, err)
	}
	if len(fx.Cases) == 0 {
		t.Fatal("the fixture has no cases; this loader guards nothing")
	}
	// The block's size today; `len(fx.Cases) == 0` is met by a corpus down to one.
	const modelCatalogFloor = 7
	var tally sweepguard.Tally
	for _, c := range fx.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			got := buildListModels(c.Config.DefaultModel, c.Config.ContextWindow, c.Config.SkipPermissionsDefault,
				c.Config.DefaultPermissionMode, c.Config.SeenModels, c.Live)
			gotJSON, _ := json.Marshal(got)
			wantJSON, _ := json.Marshal(c.Expected)
			if string(gotJSON) != string(wantJSON) {
				t.Errorf("claude.listModels drifted from the contract\n  got:  %s\n  want: %s\n  why:  %s",
					gotJSON, wantJSON, c.Why)
			}
		})
	}
	if err := tally.RequireEvery("the claude-model-catalog corpus", modelCatalogFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}
