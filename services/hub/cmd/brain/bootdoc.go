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
//
// STILL UNCONDITIONAL AFTER THE 2026-08-26 FULL-ACCESS DECISION, and this is the
// deliberate half of that change rather than an oversight. Live agents.spawn now
// honors skipPermissions / a bypass permissionMode for a host or operator-tier
// token, because the token is the trust boundary and a remote operator should
// feel like they are sitting at the machine. PERSISTENCE is not the same
// question, for one reason that decides it:
//
//	A LIVE SPAWN DIES WITH THE PROCESS. A PERSISTED DOCUMENT OUTLIVES THE TOKEN.
//
// Everything else in this design is revocable: pull a token out of tokens.json
// and revalidateScoped closes the socket on the next frame, and the agents it
// started are still just processes the user can kill. A boot-restore document is
// not reached by any of that. It sits in <configDir>/sessions/<slug>.yaml, it is
// sessions[0] by virtue of its fresh timestamp, and the desktop respawns it
// through the LOCAL IPC spawn door — which scrubs nothing and asks nobody —
// EVERY launch, forever, long after the credential that wrote it was revoked.
// A stolen phone token would plant a permanent bypassed agent that revoking the
// token does not undo.
//
// The alternative considered and rejected: persist the fields behind a
// provenance stamp the respawn door checks. It cannot be made to hold here. The
// stamp would have to live in the same 0644 YAML file as the fields it
// authorizes, and the threat model for this file is precisely a writer who can
// put arbitrary keys in it — so the stamp is forgeable by exactly the party it
// is meant to stop, unless it becomes an HMAC over the record with a host
// secret. That is a real key-management surface for a feature whose cost is one
// re-grant after a restart.
//
// So: FULL ACCESS IS LIVE-ONLY. A bus-spawned full-access agent runs bypassed
// for its whole life; a restart brings it back in ask-mode, and the record says
// so out loud (escalationScrubbedKey) instead of coming back quietly weaker.

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
		// Hub-stamped, never caller-supplied — deleted first so it can be
		// neither forged nor left stale on a record that came back clean.
		delete(entry, escalationScrubbedKey)
		var mine []string
		for _, k := range spawnEscalationKeys {
			if _, present := entry[k]; present {
				delete(entry, k)
				mine = append(mine, k)
			}
		}
		// Recurse into tabs[].panes[]: a restored terminal pane's `shell` /
		// `initialCommand` are host-execution sinks the agent-level scrub can't
		// see. `shell` is argv[0] of the LOCAL terminal:create (which allowlists
		// nothing); `initialCommand` is typed into the ready PTY with a trailing
		// CR, i.e. arbitrary shell text auto-executed on restore. Both fire on
		// the desktop's next launch, one level below the spawn-escalation keys.
		//
		// NOT `continue`-ing past this block on a tab-less agent: the stamp
		// below has to run for every entry that lost anything, and the common
		// shape is an agent with escalation fields and no tabs at all.
		if tabs, ok := entry["tabs"].([]any); ok {
			for _, t := range tabs {
				tab, ok := t.(map[string]any)
				if !ok {
					continue
				}
				panes, ok := tab["panes"].([]any)
				if !ok {
					continue
				}
				for _, p := range panes {
					pane, ok := p.(map[string]any)
					if !ok {
						continue
					}
					for _, k := range paneEscalationKeys {
						if _, present := pane[k]; present {
							delete(pane, k)
							mine = append(mine, "pane."+k)
						}
					}
				}
			}
		}
		if len(mine) > 0 {
			// NO SILENT DOWNGRADES: the persisted record itself carries what it
			// lost, so sessions.load / layouts.list show a client the downgrade
			// rather than leaving it in this process's log. Per AGENT, because
			// that is the granularity of the loss.
			entry[escalationScrubbedKey] = mine
			dropped = append(dropped, mine...)
		}
	}
	if len(dropped) > 0 {
		log.Printf("SECURITY: %s: dropping spawn-escalation field(s) %v from a bus client — full access is LIVE-ONLY: a live agents.spawn honors it for a host/operator token, but this document is respawned through the LOCAL IPC spawn door on every launch, which scrubs nothing and outlives any revocation. Each record now carries an escalationScrubbed note so the caller sees this too", method, dropped)
	}
	return dropped
}

// spawnEscalationKeys is the same list internal/layout scrubs, spelled here
// because cmd/brain must not import the hub's internal packages. The two are
// held equal by TestBootDocumentWritersScrubTheSameFields.
var spawnEscalationKeys = []string{"skipPermissions", "permissionMode", "profileId", "mcpItemIds"}

// escalationScrubbedKey is the per-agent note this scrub leaves behind: the
// keys THIS write lost. TWINS: internal/layout/layout.go and
// lib/bootDocumentScrub.ts, held equal with the two lists above by
// TestBootDocumentWritersScrubTheSameFields.
const escalationScrubbedKey = "escalationScrubbed"

// paneEscalationKeys is the per-pane twin of PANE_ESCALATION_KEYS in
// bootDocumentScrub.ts — the pane fields that become host command execution or a
// credential leak on the desktop's next launch. `shell`/`initialCommand` are a
// terminal pane's argv/PTY sinks; `pluginId` makes a restored `plugin` pane MINT
// a live plugin-scoped bus token and splice it onto the pane's `url`, so a bus
// writer could have the host hand a fresh capability to an attacker origin.
// Held equal to the TS copy by TestBootDocumentWritersScrubTheSameFields.
var paneEscalationKeys = []string{"shell", "initialCommand", "pluginId"}
