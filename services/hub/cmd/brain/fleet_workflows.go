package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"reflect"
)

// The headless writer preserves desktop-owned selections but cannot mutate them.
// Definitions and task execution intentionally have no Go executor in v1.
func preserveFleetWorkflowSelections(current, partial map[string]any) error {
	oldAgents, _ := current["agents"].(map[string]any)
	if value, present := partial["agents"]; present && value != nil {
		if _, ok := value.(map[string]any); !ok && (oldAgents["defaultWorkflowId"] != nil || oldAgents["workflowSelectionRevision"] != nil) {
			return fmt.Errorf("workflow selections cannot be removed by replacing agents")
		}
	}
	if agents, ok := partial["agents"].(map[string]any); ok {
		for _, key := range []string{"defaultWorkflowId", "workflowSelectionRevision"} {
			if v, present := agents[key]; present && !sameWorkflowConfigValue(v, oldAgents[key]) {
				return fmt.Errorf("use the desktop Fleet workflow selection API with expectedRevision")
			}
		}
	}
	oldProjects, _ := current["projects"].(map[string]any)
	if projects, ok := partial["projects"].(map[string]any); ok {
		for cwd, v := range projects {
			p, _ := v.(map[string]any)
			old, _ := oldProjects[cwd].(map[string]any)
			if old["workflowId"] != nil && p == nil {
				return fmt.Errorf("set project workflow to inherit before replacing selected project")
			}
			if selected, present := p["workflowId"]; present && !reflect.DeepEqual(selected, old["workflowId"]) {
				return fmt.Errorf("use the desktop Fleet workflow selection API with expectedRevision")
			}
			if selected, present := old["workflowId"]; present && p != nil {
				p["workflowId"] = selected
			}
		}
		for cwd, v := range oldProjects {
			p, _ := v.(map[string]any)
			if _, selected := p["workflowId"]; selected {
				if _, exists := projects[cwd]; !exists {
					return fmt.Errorf("set project workflow to inherit before removing selected project")
				}
			}
		}
	}
	return nil
}

func sameWorkflowConfigValue(a, b any) bool {
	left, e1 := json.Marshal(a)
	right, e2 := json.Marshal(b)
	return e1 == nil && e2 == nil && bytes.Equal(left, right)
}
