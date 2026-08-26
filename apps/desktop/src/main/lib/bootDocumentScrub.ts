/**
 * THE BOOT-RESTORE DOCUMENTS — the desktop's copy of cmd/brain/bootdoc.go.
 *
 * The shared layout document is scrubbed on write: internal/layout drops
 * skipPermissions, permissionMode, profileId and mcpItemIds from every
 * non-trusted layout.set, because those fields "STOP BEING DESCRIPTION on the
 * desktop's next launch and become arguments to a spawn" — handed straight to
 * window.electronAPI.spawnClaude, the LOCAL IPC spawn door, which scrubs
 * nothing.
 *
 * `sessions.save` and `layouts.save` write the SAME `agents` array into the SAME
 * respawn path and were not scrubbed anywhere. A bus caller (a plugin declaring
 * the method, an agent through the MCP facade, a non-host operator token) could
 * write {cwd:"/", sessionId:"dead-session-id", skipPermissions:true,
 * permissionMode:"bypassPermissions", profileId:"attacker-profile",
 * mcpItemIds:["evil-mcp"]}; the freshly stamped timestamp makes it sessions[0],
 * useSessionLifecycle loads it on boot, migrateSessionData passes the modern
 * format through as-is, and reconcileAgents({respawnStopped:true}) hands the
 * whole record to respawnFromRecord because claudemon holds no such session.
 * Proven end to end against the real useAgentManager.
 *
 * capspec classified WHERE these methods write and stopped; the composition
 * record named ONE writer of the boot document and stopped. Both providers write
 * it, so both providers scrub it.
 *
 * Unconditional, like the Go copy and for the same reason: caller identity does
 * not reach a bus provider, and the desktop's own UI persists sessions through
 * the SESSION_* IPC handlers rather than through this capability — so every
 * caller that arrives here is a remote/plugin/MCP one, exactly the population
 * layout.set scrubs.
 *
 * STILL UNCONDITIONAL AFTER THE 2026-08-26 FULL-ACCESS DECISION, deliberately.
 * A live agents.spawn now honors skipPermissions / a bypass permissionMode for a
 * host or operator-tier token — the token is the trust boundary. Persistence is
 * a different question, decided by one property: A LIVE SPAWN DIES WITH THE
 * PROCESS; A PERSISTED DOCUMENT OUTLIVES THE TOKEN. Revoking a credential closes
 * its socket and stops nothing that is already on disk, and this document is
 * respawned through the LOCAL IPC door — which scrubs nothing and asks nobody —
 * on EVERY launch thereafter. Persisting the fields behind a provenance stamp
 * was considered and rejected: the stamp would live in the same file as the
 * fields it authorizes, forgeable by exactly the writer it exists to stop,
 * unless it became an HMAC under a host secret. So full access is LIVE-ONLY, and
 * a restart brings the agent back in ask-mode — saying so (see
 * ESCALATION_SCRUBBED_KEY) rather than coming back quietly weaker. The long form
 * of this reasoning lives in cmd/brain/bootdoc.go.
 */

/** The four fields, spelled the same as internal/layout's spawnEscalationKeys. */
export const SPAWN_ESCALATION_KEYS = [
  'skipPermissions',
  'permissionMode',
  'profileId',
  'mcpItemIds',
] as const;

/**
 * Per-PANE host-execution fields, dropped from every `tabs[].panes[]` entry of a
 * bus-written boot document. Both are argv/command sinks that fire on the LOCAL
 * desktop's next launch, NOT via spawnClaude but via the terminal restore path:
 *
 * - `shell` is argv[0] of the restored terminal. TerminalPane feeds
 *   `shell || termCfg.shell` to `IPC.TERMINAL_CREATE`, which spawns `argv:[shell]`
 *   through the LOCAL terminal door — the one that calls `resolveTerminalShell`
 *   NOWHERE (lib/shellAllowlist.ts guards only the bus `terminals.create`). A
 *   planted `/tmp/x` executable then runs on restore. The agent-LEVEL scrub above
 *   never reached this, because it lives one level down inside a pane.
 * - `initialCommand` is strictly worse: TerminalPane types it into the ready PTY
 *   WITH a trailing CR (`write(initialCommand + '\r')`), so it is arbitrary shell
 *   TEXT auto-executed on restore — no binary needs planting at all.
 * - `pluginId` is a CREDENTIAL sink, not an argv one. A restored `plugin` pane
 *   whose `pluginId` names a loaded plugin makes PluginPane MINT a live
 *   plugin-scoped hub-bus token (`window.electronAPI.pluginPaneToken(pluginId,
 *   cwd)`) and splice it onto the pane's `url` (`u.searchParams.set('busToken',
 *   token)`) before loading it in the webview. A bus writer that set both
 *   `url:'https://attacker/x'` and `pluginId:'<loaded-plugin>'` would have the
 *   host hand a fresh authenticated capability to an attacker origin on restore.
 *   Dropping `pluginId` makes `canMint` false, so no token is minted for any
 *   bus-restored pane; the surviving `url` then loads UNauthenticated, at parity
 *   with a browser pane. `url` itself is left alone because it is shared with
 *   browser panes and carries no credential without the mint.
 *
 * Bus callers have no legitimate use for any of these: the desktop persists its
 * own sessions through the LOCAL sessionService, so every document arriving on
 * the bus is a remote/plugin/MCP one. Held equal to the Go twin's
 * paneEscalationKeys by TestBootDocumentWritersScrubTheSameFields.
 */
export const PANE_ESCALATION_KEYS = ['shell', 'initialCommand', 'pluginId'] as const;

/**
 * The per-agent note a scrub leaves behind: the keys THAT record lost on THIS
 * write. Hub-stamped only — every writer deletes an incoming copy before
 * deciding whether to add its own — so it can neither be forged into a document
 * nor left stale on a record that came back clean.
 *
 * It is the boot-document half of the no-silent-downgrade rule; the live spawn
 * path spells the same key in its RESULT (hubCapabilities.ts agents.spawn,
 * cmd/brain spawnResult, internal/bus sanitizeSpawnParams). TWINS:
 * internal/layout/layout.go and cmd/brain/bootdoc.go, all three held equal with
 * the two key lists by TestBootDocumentWritersScrubTheSameFields.
 */
export const ESCALATION_SCRUBBED_KEY = 'escalationScrubbed';

/**
 * Returns a copy of `agents` with the spawn-escalation fields removed from every
 * entry, and the list of keys that were actually dropped (for logging).
 *
 * Copies rather than mutates: the caller's object may be the params of an
 * in-flight RPC, and a scrub that mutated shared state would be a different bug.
 * Anything that is not an array of objects is returned unchanged — the respawn
 * path gets nothing out of such a document either.
 */
export function scrubBootDocumentAgents<T>(agents: T): { agents: T; dropped: string[] } {
  if (!Array.isArray(agents)) return { agents, dropped: [] };
  const dropped: string[] = [];
  const out = agents.map((entry) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return entry;
    const copy: Record<string, unknown> = { ...(entry as Record<string, unknown>) };
    // Hub-stamped, never caller-supplied: dropped first so a writer cannot plant
    // a false note, and so a record that came back clean loses a stale one.
    delete copy[ESCALATION_SCRUBBED_KEY];
    const mine: string[] = [];
    for (const key of SPAWN_ESCALATION_KEYS) {
      if (key in copy) {
        delete copy[key];
        mine.push(key);
      }
    }
    // Recurse into tabs[].panes[]: a restored terminal pane's `shell` /
    // `initialCommand` are host-execution sinks the agent-level scrub can't see.
    if (Array.isArray(copy.tabs)) {
      copy.tabs = copy.tabs.map((tab) => {
        if (!tab || typeof tab !== 'object' || Array.isArray(tab)) return tab;
        const tabCopy = { ...(tab as Record<string, unknown>) };
        if (Array.isArray(tabCopy.panes)) {
          tabCopy.panes = tabCopy.panes.map((pane) => {
            if (!pane || typeof pane !== 'object' || Array.isArray(pane)) return pane;
            const paneCopy = { ...(pane as Record<string, unknown>) };
            for (const key of PANE_ESCALATION_KEYS) {
              if (key in paneCopy) {
                delete paneCopy[key];
                mine.push(`pane.${key}`);
              }
            }
            return paneCopy;
          });
        }
        return tabCopy;
      });
    }
    if (mine.length) {
      // NO SILENT DOWNGRADES: the persisted record itself carries what it lost,
      // so sessions.load / layouts.list show a client the downgrade instead of
      // leaving it in this process's log. Per AGENT — that is the granularity of
      // the loss.
      copy[ESCALATION_SCRUBBED_KEY] = mine;
      dropped.push(...mine);
    }
    return copy;
  });
  return { agents: out as unknown as T, dropped };
}
