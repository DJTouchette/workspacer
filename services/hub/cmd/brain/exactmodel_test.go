package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestExactModelStopsOldHubSubstitutionsBeforeRemoteAdmission(t *testing.T) {
	for _, field := range []string{"model", "modelIdentity", "contextWindow", "effort", "capability"} {
		t.Run(field, func(t *testing.T) {
			r := &registry{} // Any attempt to reach daemon/remote setup would fail this test.
			raw, _ := json.Marshal(map[string]any{"provider": "codex", "model": "replacement", "exactModel": true, "escalationScrubbed": []string{field}})
			_, err := r.spawn(context.Background(), raw)
			if err == nil || !strings.Contains(err.Error(), "no substitute was launched") {
				t.Fatalf("%v", err)
			}
		})
	}
}
