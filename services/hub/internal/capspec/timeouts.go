package capspec

import (
	"strings"
	"time"
)

// Server-owned operation budgets. Clients cannot lengthen arbitrary calls by
// supplying a timeout; test/custom short router deadlines remain authoritative.
func ProviderTimeout(method string, base time.Duration) time.Duration {
	if base < 30*time.Second {
		return base
	}
	if strings.HasPrefix(method, "hub:") {
		if _, bare, ok := strings.Cut(method, "/"); ok {
			method = bare
		}
	}
	budget := base
	switch method {
	case "agents.spawn", "desktop.worktreeCreate":
		budget = 6 * time.Minute
	case "claude.handoffAgentBrief":
		budget = 3 * time.Minute
	case "desktop.managerRequestSend", "desktop.worktreeRemove":
		budget = time.Minute
	}
	if budget < base {
		return base
	}
	return budget
}
