package capspec

import (
	"testing"
	"time"
)

func TestIntentWorkspaceProviderBudget(t *testing.T) {
	for _, method := range []string{"desktop.intentWorkspaceRequest", "hub:peer/desktop.intentWorkspaceRequest"} {
		if got := ProviderTimeout(method, 30*time.Second); got != 3*time.Minute {
			t.Fatalf("%s budget %s", method, got)
		}
		if got := ProviderTimeout(method, time.Second); got != time.Second {
			t.Fatalf("custom short deadline expanded: %s", got)
		}
		if got := ProviderTimeout(method, 4*time.Minute); got != 4*time.Minute {
			t.Fatalf("long deadline shortened: %s", got)
		}
	}
}
