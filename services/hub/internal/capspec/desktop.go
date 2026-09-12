package capspec

// DesktopServices is the explicit headless companion surface. These are
// operator actions, not arbitrary IPC forwarding: the Node host has a fixed
// dispatcher and adds no methods based on caller input. Its filesystem calls
// use main/lib/pathConfinement with roots supplied privately by the brain.
var DesktopServices = []string{
	"desktop.worktreeInfo",
	"desktop.worktreeCreate",
	"desktop.worktreeRemove",
	"desktop.pricingGetRates",
	"desktop.pricingSaveOverrides",
	"desktop.claudeProfilesAccounts",
	"desktop.claudeProfilesLoginStatus",
	"desktop.claudeProfilesAddAccount",
	"desktop.toolsStatus",
	"desktop.fleetReviewRead",
	"desktop.fleetReviewForget",
	"desktop.taskInspectorEdit",
	"desktop.taskInspectorOpen",
	"desktop.dispatchHistoryRead",
	"desktop.htmlCardReadDiff",
	"desktop.fleetWorkflowRequest",
	"desktop.managerRequestPrepare",
	"desktop.managerRequestSend",
	"desktop.loadBriefBoard",
	"desktop.moveBriefCard",
	"desktop.claudeProfilesAdd",
	"desktop.claudeProfilesUpdate",
	"desktop.claudeProfilesRemove",
	"desktop.saveConfig",
	"desktop.agentSuggestTitle",
	"desktop.providerReadiness",
	"desktop.agentRuntimeStatus",
	"desktop.keepWarmHeartbeats",
	"desktop.workflowAgentTranscript",
	"desktop.workflowAgentConversation",
	"desktop.installUiFont",
	"desktop.downloadProjectIcon",
	"desktop.managerReplacement",
	"desktop.sessionGrantReconcile",
	"desktop.readFileBytes",
	"desktop.filePickerList",
}

// Display assets have the same read authority as the shared interface. They
// expose only fixed cache roots, never arbitrary server files.
var UIAssetServices = []string{
	"ui.fonts",
	"ui.asset",
}

func IsUIAssetService(method string) bool {
	for _, m := range UIAssetServices {
		if method == m {
			return true
		}
	}
	return false
}

func IsDesktopService(method string) bool {
	for _, m := range DesktopServices {
		if method == m {
			return true
		}
	}
	return false
}

func init() {
	inertMethods["ui.fonts"] = "no caller parameters; reads uploaded font filenames/family labels from a fixed UI cache, never writes config or grants"
	unscopedByDecision["ui.asset"] = "reads only a validated single asset filename from fixed font/icon cache roots; canonical confinement and bounded descriptor reads in headless/uiAssets.ts, not a general filesystem capability"
	unscopedParams["ui.asset"] = map[string]ParamDecision{"file": {KindFilename, "one basename selecting a cached font/icon, canonicalized under the fixed asset root and bounded on read"}}
	compositionInert["ui.asset"] = InertClaim{Reason: "The file selects cached display bytes under a fixed root. The assertPathAllowed call confines it before descriptor access; no durable state or authority is changed, and the browser only interprets these bytes as a font or image.", Witnesses: []Witness{guarded(argBearing("assertPathAllowed", "ui.asset", []string{"apps", "desktop", "src", "main", "headless", "uiAssets.ts"}))}}
	for _, method := range DesktopServices {
		unscopedByDecision[method] = "operator-only shared desktop service; fixed dispatcher validates parameters and host-owned selectors, and all filesystem roots come from the brain outside caller-controlled params (main/headless/desktopHost.ts); no generic IPC, command, or arbitrary-file bridge"
		compositionInert[method] = InertClaim{
			Reason:    "authenticatedDesktopUser preserves native IPC owner authority before any shared service runs. Writes can create worktrees, account configuration, numeric rates or host-owned evidence; subsequent interpreters and selectors are deliberately available only to this authenticated owner. Scoped operators, agents, plugins and peer links cannot acquire this transport, and caller params cannot supply the brain's filesystem roots or live session identities.",
			Witnesses: []Witness{guarded(argBearing("authenticatedDesktopUser", method, []string{"services", "hub", "internal", "bus", "desktop.go"}))},
		}
	}
	unscopedByDecision["desktop.filePickerList"] = "Authenticated-owner filename-only picker metadata under the server user's OS rights, including system executable directories. This does not read file contents or add roots to fs.read/fs.write; actual bytes remain guarded separately."

}
