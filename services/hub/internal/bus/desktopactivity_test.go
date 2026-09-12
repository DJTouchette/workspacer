package bus

import (
	"encoding/json"
	"testing"
)

func TestDesktopBackgroundReadsDoNotPreventIdleStop(t *testing.T) {
	for _, m := range []string{"desktop.agentRuntimeStatus", "desktop.dispatchHistoryRead", "desktop.keepWarmHeartbeats", "desktop.workflowAgentConversation", "fs.read", "fs.watch", "git.log"} {
		if !passiveCall(m) {
			t.Errorf("background read counts as input: %s", m)
		}
	}
	for _, c := range []struct {
		method, params string
		passive        bool
	}{
		{"desktop.fleetWorkflowRequest", `{"request":{"op":"list"}}`, true},
		{"desktop.fleetWorkflowRequest", `{"request":{"op":"create"}}`, false},
		{"fleetWorkflows.request", `{"op":"next"}`, true},
		{"fleetWorkflows.request", `{"op":"decide"}`, false},
		{"desktop.providerReadiness", `{"check":false}`, true},
		{"desktop.providerReadiness", `{"check":true}`, false},
		{"desktop.providerReadiness", `invalid`, false},
	} {
		if got := passiveCall(c.method, json.RawMessage(c.params)); got != c.passive {
			t.Errorf("%s %s: passive=%v", c.method, c.params, got)
		}
	}
}
