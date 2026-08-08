package main

import "log"

// THE BOOT-RESTORE DOCUMENTS. layout.set is not the only writer of an `agents`
// array the desktop respawns.
//
// internal/layout scrubs four fields — skipPermissions, permissionMode,
// profileId, mcpItemIds — from every non-trusted layout.set, because (its own
// comment) those fields "STOP BEING DESCRIPTION on the desktop's next launch and
// become arguments to a spawn", handed straight to window.electronAPI.spawnClaude,
// "the LOCAL IPC spawn door, which does no scrubbing".
//
// sessions.save writes the SAME shape into <configDir>/sessions/<slug>.yaml with
// a fresh timestamp, which makes it the most recent session — and
// useSessionLifecycle takes sessions[0] on boot, migrateSessionData passes the
// modern format through as-is, loadAgentsFromSession adopts it, and the
// phase-triggered reconcileAgents{respawnStopped:true} sends every card whose
// sessionId claudemon no longer holds to respawnFromRecord, which forwards
// profileId, permissionMode, skipPermissions and mcpItemIds to spawnClaude.
// Proven end to end: a bus-written document with cwd "/" and all four fields
// produced spawnClaude({cwd:"/", skipPermissions:true,
// permissionMode:"bypassPermissions", profileId:"attacker-profile",
// mcpItemIds:["evil-mcp"], resumeSessionId:"dead-session-id"}).
//
// layouts.save is the third copy of the shape (<configDir>/layouts/<slug>.yaml,
// "the caller's whole `agents` array", restored from the Layouts menu).
//
// capspec excused both as PATH questions — "the filename is derived from the
// session name by the provider's slug and re-checked by the same resolver" —
// and nothing in either provider looked at what the document CONTAINS. The
// composition record named ONE writer of the boot document and stopped.
//
// UNCONDITIONAL, unlike layout.set's scrub, because caller identity does not
// reach a bus PROVIDER: bus.CallerIdentity is delivered to in-hub
// RegisterLocalIdent handlers only, and brain answers over the wire like any
// other provider. There is no trusted bus caller of these methods to protect:
// the desktop persists its own sessions through its local sessionService and
// mirrors live state through layout.set, so every caller that arrives HERE is a
// remote/plugin/MCP one — exactly the population layout.set scrubs.

// scrubBootDocumentAgents removes the spawn-escalation fields from every entry
// of a boot-restore document's `agents` array, in place, and reports what it
// dropped. Shape-tolerant on purpose: a document whose agents are not objects is
// one the desktop's respawn path gets nothing out of either.
func scrubBootDocumentAgents(method string, doc map[string]any) []string {
	agents, ok := doc["agents"].([]any)
	if !ok {
		return nil
	}
	var dropped []string
	for _, a := range agents {
		entry, ok := a.(map[string]any)
		if !ok {
			continue
		}
		for _, k := range spawnEscalationKeys {
			if _, present := entry[k]; present {
				delete(entry, k)
				dropped = append(dropped, k)
			}
		}
	}
	if len(dropped) > 0 {
		log.Printf("SECURITY: %s: dropping spawn-escalation field(s) %v from a bus client — the desktop respawns this document's agents through the LOCAL IPC spawn door on its next launch, which scrubs nothing", method, dropped)
	}
	return dropped
}

// spawnEscalationKeys is the same list internal/layout scrubs, spelled here
// because cmd/brain must not import the hub's internal packages. The two are
// held equal by TestBootDocumentWritersScrubTheSameFields.
var spawnEscalationKeys = []string{"skipPermissions", "permissionMode", "profileId", "mcpItemIds"}
