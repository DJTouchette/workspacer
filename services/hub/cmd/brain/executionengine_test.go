package main

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestExecutionEngineSnapshotProjection(t *testing.T) {
	raw := mustReadRepoFile(t, "contracts", "execution-engine-v1.json")
	var fixture map[string]any
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}

	for _, readiness := range []string{"ready", "unavailable"} {
		engine := fixture["metadata"].(map[string]any)
		engine["readiness"] = readiness
		row := mustJSONBytes(t, map[string]any{"session_id": "engine", "mode": "stopped", "execution_engine": engine})
		var result map[string]any
		if err := json.Unmarshal(compatSnapshot(row), &result); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(result["executionEngine"], engine) {
			t.Fatalf("engine pin changed: %v", result)
		}
	}
	var old map[string]any
	json.Unmarshal(compatSnapshot([]byte(`{"session_id":"old","mode":"input"}`)), &old)
	if _, ok := old["executionEngine"]; ok {
		t.Fatal("old peer falsely advertises engine support")
	}
}
