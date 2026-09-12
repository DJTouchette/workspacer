package bus

import (
	"strings"
	"time"
)

// These are background-safe reads, not a grant of authority. Unknown methods
// still count as activity. No caller flag can make a mutation passive.
func passiveCall(method string) bool {
	if strings.HasPrefix(method, "hub:") {
		_, bare, ok := strings.Cut(method, "/")
		if ok {
			method = bare
		}
	}
	switch method {
	case "remote.pairingInfo", "remote.tokensList", "machine.power", "fleet.quiescence", "sessions.snapshots", "sessions.snapshot",
		"sessions.list", "sessions.recent", "sessions.conversation", "sessions.get",
		"sessions.stats", "sessions.analytics", "agents.list", "agents.get",
		"config.get", "config.getPath", "layout.get", "layouts.list",
		"federation.peers", "nodes.list", "brain.info", "app.getCwd",
		"usage.report", "usage.pacingSchedule", "analytics.summary", "analytics.recent",
		"providers.checkAll", "providers.listModels", "sessions.load", "git.commitDiff", "claude.listModels", "claude.profiles.list", "claude.sessionsForDir",
		"plugins.list", "plugins.tools", "plugins.settings", "plugins.manifests",
		"jobs.list", "jobs.history", "library.list", "push.key",
		"git.status", "git.diff", "git.numstat", "git.branch", "git.branches",
		"fs.listDir", "fs.readFile", "fs.stat", "search.project",
		"fleet.tasks", "fleet.workers", "fleet.managers", "fleet.dispatches",
		"routing.preferences.get", "routing.preview":
		return true
	}
	return false
}

func (cn *conn) markInteraction(now time.Time) {
	cn.lastInteractionMilli.Store(now.UnixMilli())
}

func (cn *conn) markCall(now time.Time, method string) {
	cn.markActive(now)
	if !passiveCall(method) {
		cn.markInteraction(now)
	}
}

func (cn *conn) idleActivity() time.Time {
	if cn.reportsInteraction.Load() {
		return time.UnixMilli(cn.lastInteractionMilli.Load())
	}
	// Old clients have no way to report input: retain conservative behavior.
	return time.UnixMilli(cn.lastActiveMilli.Load())
}
