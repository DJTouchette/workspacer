package main

// The one place this process answers "does this permission mode turn the host's
// approvals OFF?" — for every bus path that takes a mode from a caller who is
// not the local user.
//
// agents.spawn has always carried the invariant in prose ("a YOLO agent must be
// started locally") and enforced it with two inline string comparisons,
// `bypassPermissions` and `yolo`, inside the spawn handler. That made the
// invariant look like a property of SPAWNING. It is a property of the MODE, and
// the desktop provider takes the same mode through a second door
// (claude.setPermissionMode, which reaches an agent that is already running and
// was not clamped at all). The two stacks have to agree on the vocabulary, so it
// is written down here and held to lib/permissionBypass.ts by
// TestPermissionModeAllowlistMatchesTheDesktop.
//
// ALLOWLIST, not denylist. The modes in flight across the four providers are
// default/acceptEdits/plan/bypassPermissions (claude, both transports), ask/yolo
// (codex, copilot, opencode, pi), and auto/dontAsk/manual, which claudemon's stream
// endpoint also accepts and which show up in live telemetry without appearing in
// any menu. A denylist has to name every spelling of "stop asking" that any
// provider will ever ship; an allowlist names the ones checked to mean "keep
// asking (or ask MORE)" and fails closed on the rest — the same reasoning that
// made terminals.create's `shell` an allowlist of login shells (shellallow.go)
// rather than a list of forbidden binaries.
var busSettablePermissionModes = map[string]bool{
	"default":     true,
	"ask":         true,
	"acceptEdits": true, // still gates every non-edit tool call, Bash included
	"plan":        true, // strictly more restrictive than default
	"manual":      true, // the stream endpoint's "I approve each step"
}

// isPermissionEscalation reports whether a bus caller asking for this mode is
// asking the host to stop requiring approvals. An empty mode is not an
// escalation — the caller is asking for the provider default.
func isPermissionEscalation(mode string) bool {
	if mode == "" {
		return false
	}
	return !busSettablePermissionModes[mode]
}

// permissionModeMeansBypass reports whether a CONFIG-CHOSEN mode means
// "approvals off" — used to resolve claude.defaultPermissionMode into a
// skipPermissions default for a spawn that omitted the field. Deliberately NOT
// isPermissionEscalation: that one judges a caller's REQUEST and rightly fails
// closed on every unknown spelling, but here failing closed means the opposite
// direction — a garbled config value must resolve to approvals ON, so only the
// spellings known to mean bypass count. TWIN: cmd/mcp permissionModeMeansBypass
// and lib/permissionBypass.ts CONFIG_BYPASS_PERMISSION_MODES.
func permissionModeMeansBypass(mode string) bool {
	return mode == "bypassPermissions" || mode == "yolo"
}
