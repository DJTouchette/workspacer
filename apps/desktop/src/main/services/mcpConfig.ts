/**
 * Supervisor MCP config: lazily writes (and caches) the JSON config file that
 * points the supervisor Claude Code agent at the workspacer MCP facade, and
 * returns its absolute path.  The facade is assumed to be already running at
 * http://127.0.0.1:7897/mcp — this module does NOT start it.
 */

import { app } from 'electron';
import * as path from 'path';
import * as fs from 'fs';
import type { McpServerConfig } from './libraryService';

/** System prompt injected into every supervisor session. */
export const SUPERVISOR_SYSTEM_PROMPT =
  `You are the Workspacer fleet supervisor. You have MCP tools (prefixed mcp__workspacer__) that give you the same control over workspacer as the desktop app, to observe and coordinate the user's other Claude Code agent sessions:
- Observe: list_agents (the fleet overview — start here), get_snapshot and get_transcript (full detail on one session), list_models, list_resumable_sessions, get_host_cwd.
- Spawn & drive: spawn_agent, create_terminal, send_message, approve, answer, signal, set_approval_gate, terminal_input, terminal_resize.
- Host filesystem: list_dir, list_entries, read_file, write_file, search_project — for inspecting the host's projects to brief or route work (you coordinate; you don't write the code yourself).
- Manage: get_config/save_config, profiles (list/add/update/remove), saved sessions and layouts, the prompt/skill library, and analytics_summary/analytics_recent.
- notify to surface a desktop notification.
Start by calling list_agents to discover the fleet, then get_snapshot or get_transcript for detail. Whenever you reference a session in your answer, write its id in the form session:<sessionId> so the UI can turn it into a clickable link. Be concise and direct — you are briefing a busy senior engineer.`;

/** The workspacer MCP facade — an HTTP MCP server started at app launch
 *  (mcpFacadeDaemon). Claude points at it via --mcp-config; managed providers
 *  (Codex/OpenCode) register it through their own MCP config. */
export const MCP_FACADE_URL = 'http://127.0.0.1:7897/mcp';

const MCP_CONFIG_CONTENTS = JSON.stringify(
  { mcpServers: { workspacer: { type: 'http', url: MCP_FACADE_URL } } },
  null,
  2,
);

/**
 * Role instructions prepended to a *managed* (Codex/OpenCode) agent's first
 * turn when it's given the facade. Claude gets this via --append-system-prompt
 * plus the /supervise skill; managed providers don't have those, so we inject
 * the role as system text on the opening message.
 */
export function managedFacadeInstructions(supervisor: boolean): string {
  if (!supervisor) {
    return (
      'You have the workspacer MCP tools (the workspacer__* tool set) to observe the agent fleet. ' +
      'You may be asked to read another session’s transcript and reply with a concise, structured digest ' +
      'of what that agent is doing and whether it is blocked. Do not spawn or coordinate other agents.'
    );
  }
  return (
    `${SUPERVISOR_SYSTEM_PROMPT}\n\n` +
    'Watch the fleet continuously: start with list_agents, then get_snapshot / get_transcript for detail, ' +
    'and surface anything that needs a human. Spawn cheap summarizer workers when you need transcript digests. ' +
    'Tool names may be prefixed by your runtime (e.g. workspacer__list_agents) — use whichever the workspacer server exposes.'
  );
}

/**
 * Build the argv fragment that grants a spawned session the workspacer MCP
 * facade (--mcp-config + pre-allowed tools + an --append-system-prompt note).
 *
 * Two flavours:
 *  - supervisor: the full fleet-coordination role + a kick to run /supervise on
 *    a loop, parameterised with the configured summarizer model + cadence.
 *  - plain facade worker (supervisor:false): just a short note that it has the
 *    tools — used for the cheap summarizer workers a supervisor spawns, which
 *    read transcripts themselves (so they need the facade) but must NOT loop or
 *    coordinate.
 *
 * Centralised here so the desktop (ipc) and bus (hubCapabilities) spawn paths
 * stay identical.
 */
export function facadeSpawnArgs(opts: {
  sessionId: string;
  supervisor?: boolean;
  summarizerModel?: string;
  pollSeconds?: number;
}): { mcpConfig: string; allowedTools: string[]; appendSystemPrompt: string } {
  const idNote = `Your own workspacer session id is ${opts.sessionId}.`;
  if (!opts.supervisor) {
    return {
      mcpConfig: supervisorMcpConfigPath(),
      allowedTools: ['mcp__workspacer'],
      appendSystemPrompt:
        `You have the workspacer MCP tools (mcp__workspacer__*) to observe the agent fleet. ${idNote} ` +
        `You may be asked to call get_transcript for another session and reply with a concise, structured digest of what that agent is doing and whether it is blocked. ` +
        `Do not spawn or coordinate other agents.`,
    };
  }
  const summarizer = (opts.summarizerModel ?? '').trim() || 'sonnet';
  const poll = opts.pollSeconds && opts.pollSeconds > 0 ? opts.pollSeconds : 45;
  return {
    mcpConfig: supervisorMcpConfigPath(),
    allowedTools: ['mcp__workspacer'],
    appendSystemPrompt:
      `${SUPERVISOR_SYSTEM_PROMPT}\n\n${idNote} When you spawn worker agents with spawn_agent, pass parentSessionId:"${opts.sessionId}" and a short label so they appear nested under you in the UI.\n\n` +
      `Run the /supervise skill now to begin watching the fleet, and keep it running on a loop (about every ${poll}s). ` +
      `Spawn your transcript-summarizer workers with model "${summarizer}" and mcpFacade:true so they can read transcripts themselves without consuming your context.`,
  };
}

let cachedPath: string | undefined;

/**
 * Returns the absolute path to the supervisor MCP config file, writing it to
 * `<userData>/supervisor-mcp.json` if it does not yet exist.  The result is
 * cached in-process so subsequent calls are instant.
 */
export function supervisorMcpConfigPath(): string {
  if (cachedPath) return cachedPath;
  const filePath = path.join(app.getPath('userData'), 'supervisor-mcp.json');
  if (!fs.existsSync(filePath)) {
    fs.writeFileSync(filePath, MCP_CONFIG_CONTENTS, 'utf8');
    console.log(`[mcpConfig] wrote supervisor MCP config: ${filePath}`);
  }
  cachedPath = filePath;
  return filePath;
}

/** One selected MCP server: a stable key (the library item id) + its config. */
export interface SessionMcpServer {
  id: string;
  mcp: McpServerConfig;
}

/** Translate a stored McpServerConfig into Claude Code's `mcpServers` entry. */
function toClaudeEntry(cfg: McpServerConfig): Record<string, unknown> | null {
  // URL-based servers (http/sse) — `type` + `url` are required.
  if (cfg.url && cfg.url.trim()) {
    const entry: Record<string, unknown> = { type: cfg.type === 'sse' ? 'sse' : 'http', url: cfg.url.trim() };
    if (cfg.headers && Object.keys(cfg.headers).length) entry.headers = cfg.headers;
    return entry;
  }
  // Local (stdio) servers — `command` is required; `type` may be omitted.
  if (cfg.command && cfg.command.trim()) {
    const entry: Record<string, unknown> = { command: cfg.command.trim() };
    if (cfg.args && cfg.args.length) entry.args = cfg.args;
    if (cfg.env && Object.keys(cfg.env).length) entry.env = cfg.env;
    return entry;
  }
  return null; // incomplete — skip it
}

/**
 * Write a per-session `--mcp-config` JSON for the given selected servers and
 * return its path plus the `mcp__<id>` tool globs to pre-allow. The server's
 * key in the config (and thus its tool prefix) is the library item id, so it's
 * stable across sessions. Returns null when nothing valid was selected.
 *
 * The file is keyed by session id under `<userData>/session-mcp/` so concurrent
 * sessions don't clobber each other; it's rewritten on every (re)spawn.
 */
export function buildSessionMcpConfig(
  sessionId: string,
  servers: SessionMcpServer[],
): { path: string; toolNames: string[] } | null {
  const mcpServers: Record<string, unknown> = {};
  const toolNames: string[] = [];
  for (const s of servers) {
    const entry = toClaudeEntry(s.mcp);
    if (!entry) continue;
    mcpServers[s.id] = entry;
    toolNames.push(`mcp__${s.id}`);
  }
  if (!toolNames.length) return null;

  const dir = path.join(app.getPath('userData'), 'session-mcp');
  fs.mkdirSync(dir, { recursive: true });
  const filePath = path.join(dir, `${sessionId}.json`);
  fs.writeFileSync(filePath, JSON.stringify({ mcpServers }, null, 2), 'utf8');
  return { path: filePath, toolNames };
}
