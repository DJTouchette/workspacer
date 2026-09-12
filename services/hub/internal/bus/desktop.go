package bus

import "github.com/djtouchette/workspacer-hub/internal/capspec"

// Native IPC services require the same authenticated owner over the web.
func desktopServiceAllowed(method string, cn *conn) bool {
	switch method {
	case "desktop.worktreeInfo":
		return authenticatedDesktopUser("desktop.worktreeInfo", cn)
	case "desktop.worktreeCreate":
		return authenticatedDesktopUser("desktop.worktreeCreate", cn)
	case "desktop.worktreeRemove":
		return authenticatedDesktopUser("desktop.worktreeRemove", cn)
	case "desktop.pricingGetRates":
		return authenticatedDesktopUser("desktop.pricingGetRates", cn)
	case "desktop.pricingSaveOverrides":
		return authenticatedDesktopUser("desktop.pricingSaveOverrides", cn)
	case "desktop.claudeProfilesAccounts":
		return authenticatedDesktopUser("desktop.claudeProfilesAccounts", cn)
	case "desktop.claudeProfilesLoginStatus":
		return authenticatedDesktopUser("desktop.claudeProfilesLoginStatus", cn)
	case "desktop.claudeProfilesAddAccount":
		return authenticatedDesktopUser("desktop.claudeProfilesAddAccount", cn)
	case "desktop.toolsStatus":
		return authenticatedDesktopUser("desktop.toolsStatus", cn)
	case "desktop.fleetReviewRead":
		return authenticatedDesktopUser("desktop.fleetReviewRead", cn)
	case "desktop.fleetReviewForget":
		return authenticatedDesktopUser("desktop.fleetReviewForget", cn)
	case "desktop.taskInspectorEdit":
		return authenticatedDesktopUser("desktop.taskInspectorEdit", cn)
	case "desktop.taskInspectorOpen":
		return authenticatedDesktopUser("desktop.taskInspectorOpen", cn)
	case "desktop.dispatchHistoryRead":
		return authenticatedDesktopUser("desktop.dispatchHistoryRead", cn)
	case "desktop.htmlCardReadDiff":
		return authenticatedDesktopUser("desktop.htmlCardReadDiff", cn)
	case "desktop.fleetWorkflowRequest":
		return authenticatedDesktopUser("desktop.fleetWorkflowRequest", cn)
	case "desktop.managerRequestPrepare":
		return authenticatedDesktopUser("desktop.managerRequestPrepare", cn)
	case "desktop.managerRequestSend":
		return authenticatedDesktopUser("desktop.managerRequestSend", cn)
	case "desktop.loadBriefBoard":
		return authenticatedDesktopUser("desktop.loadBriefBoard", cn)
	case "desktop.moveBriefCard":
		return authenticatedDesktopUser("desktop.moveBriefCard", cn)
	case "desktop.claudeProfilesAdd":
		return authenticatedDesktopUser("desktop.claudeProfilesAdd", cn)
	case "desktop.claudeProfilesUpdate":
		return authenticatedDesktopUser("desktop.claudeProfilesUpdate", cn)
	case "desktop.claudeProfilesRemove":
		return authenticatedDesktopUser("desktop.claudeProfilesRemove", cn)
	case "desktop.saveConfig":
		return authenticatedDesktopUser("desktop.saveConfig", cn)
	case "desktop.agentSuggestTitle":
		return authenticatedDesktopUser("desktop.agentSuggestTitle", cn)
	case "desktop.providerReadiness":
		return authenticatedDesktopUser("desktop.providerReadiness", cn)
	case "desktop.agentRuntimeStatus":
		return authenticatedDesktopUser("desktop.agentRuntimeStatus", cn)
	case "desktop.keepWarmHeartbeats":
		return authenticatedDesktopUser("desktop.keepWarmHeartbeats", cn)
	case "desktop.workflowAgentTranscript":
		return authenticatedDesktopUser("desktop.workflowAgentTranscript", cn)
	case "desktop.workflowAgentConversation":
		return authenticatedDesktopUser("desktop.workflowAgentConversation", cn)
	case "desktop.installUiFont":
		return authenticatedDesktopUser("desktop.installUiFont", cn)
	case "desktop.downloadProjectIcon":
		return authenticatedDesktopUser("desktop.downloadProjectIcon", cn)
	case "desktop.managerReplacement":
		return authenticatedDesktopUser("desktop.managerReplacement", cn)
	case "desktop.sessionGrantReconcile":
		return authenticatedDesktopUser("desktop.sessionGrantReconcile", cn)
	case "desktop.readFileBytes":
		return authenticatedDesktopUser("desktop.readFileBytes", cn)
	case "desktop.filePickerList":
		return authenticatedDesktopUser("desktop.filePickerList", cn)
	default:
		return false
	}
}
func authenticatedDesktopUser(method string, cn *conn) bool {
	return capspec.IsDesktopService(method) && cn.authenticatedHost && cn.trusted && !cn.revoked.Load()
}

func authenticatedUploadReceiver(method string, cn *conn) bool {
	return method == "files.receiveUpload" && cn != nil && cn.authenticatedHost && cn.trusted && !cn.revoked.Load() && !cn.federated && cn.pluginID == ""
}
