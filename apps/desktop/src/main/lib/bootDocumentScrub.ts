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
 *
 * Bus callers have no legitimate use for either: the desktop persists its own
 * sessions through the LOCAL sessionService, so every document arriving on the
 * bus is a remote/plugin/MCP one. Held equal to the Go twin's paneEscalationKeys
 * by TestBootDocumentWritersScrubTheSameFields.
 */
export const PANE_ESCALATION_KEYS = ['shell', 'initialCommand'] as const;

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
    for (const key of SPAWN_ESCALATION_KEYS) {
      if (key in copy) {
        delete copy[key];
        dropped.push(key);
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
                dropped.push(`pane.${key}`);
              }
            }
            return paneCopy;
          });
        }
        return tabCopy;
      });
    }
    return copy;
  });
  return { agents: out as unknown as T, dropped };
}
