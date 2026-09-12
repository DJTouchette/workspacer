package bus

import (
	"encoding/json"
	"strings"
	"time"
)

// These are background-safe reads, not a grant of authority. Unknown methods
// still count as activity. No caller flag can make a mutation passive.
func passiveCall(method string, params ...json.RawMessage) bool {
	if strings.HasPrefix(method, "hub:") {
		_, bare, ok := strings.Cut(method, "/")
		if ok {
			method = bare
		}
	}
	if method == "desktop.managerReplacement" {
		if len(params) == 0 {
			return false
		}
		var p struct {
			Request struct {
				Action string `json:"action"`
			} `json:"request"`
		}
		return json.Unmarshal(params[0], &p) == nil && p.Request.Action == "list"
	}
	if method == "desktop.providerReadiness" {
		if len(params) == 0 {
			return false
		}
		var p struct {
			Check bool `json:"check"`
		}
		return json.Unmarshal(params[0], &p) == nil && !p.Check
	}
	if method == "desktop.fleetWorkflowRequest" || method == "fleetWorkflows.request" {
		if len(params) == 0 {
			return false
		}
		var p struct {
			Op      string `json:"op"`
			Request *struct {
				Op string `json:"op"`
			} `json:"request"`
		}
		if json.Unmarshal(params[0], &p) != nil {
			return false
		}
		op := p.Op
		if method == "desktop.fleetWorkflowRequest" {
			if p.Request == nil {
				return false
			}
			op = p.Request.Op
		}
		switch op {
		case "list", "get", "validate", "requestInbox", "requestContent", "next", "taskReferences":
			return true
		}
		return false
	}
	switch method {
	case "ui.fonts", "ui.asset":
		return true
	case "desktop.worktreeInfo", "desktop.pricingGetRates", "desktop.claudeProfilesAccounts", "desktop.claudeProfilesLoginStatus", "desktop.toolsStatus", "desktop.fleetReviewRead", "desktop.dispatchHistoryRead", "desktop.htmlCardReadDiff", "desktop.loadBriefBoard", "desktop.agentRuntimeStatus", "desktop.keepWarmHeartbeats", "desktop.workflowAgentTranscript", "desktop.workflowAgentConversation":
		return true
	case "remote.sharingInfo", "remote.tailscaleInfo", "remote.pairingInfo", "remote.tokensList", "machine.power", "fleet.quiescence", "sessions.snapshots", "sessions.snapshot",
		"sessions.list", "sessions.recent", "sessions.conversation", "sessions.get",
		"sessions.stats", "sessions.analytics", "agents.list", "agents.get",
		"config.get", "config.getPath", "layout.get", "layouts.list",
		"federation.peers", "nodes.list", "brain.info", "app.getCwd",
		"usage.report", "usage.pacingSchedule", "analytics.summary", "analytics.recent",
		"providers.checkAll", "providers.listModels", "sessions.load", "git.commitDiff", "claude.listModels", "claude.profiles.list", "claude.sessionsForDir",
		"plugins.list", "plugins.tools", "plugins.settings", "plugins.manifests",
		"jobs.list", "jobs.history", "library.list", "push.key",
		"git.status", "git.diff", "git.numstat", "git.branch", "git.branches",
		"fs.listDir", "fs.listEntries", "fs.read", "fs.readImage", "fs.readFile", "fs.stat", "fs.watch", "fs.unwatch", "search.project",
		"git.log", "git.commitNumstat", "app.supervisorHome",
		"fleet.tasks", "fleet.workers", "fleet.managers", "fleet.dispatches",
		"routing.preferences.get", "routing.preview":
		return true
	}
	return false
}

func (cn *conn) markInteraction(now time.Time) {
	cn.lastInteractionMilli.Store(now.UnixMilli())
}

func (cn *conn) markCall(now time.Time, method string, params ...json.RawMessage) {
	cn.markActive(now)
	if !passiveCall(method, params...) {
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
