/**
 * MCP facade wiring for spawned sessions: writes the JSON config files that
 * point an agent at the workspacer MCP facade, and builds the argv/prompt
 * fragments that go with them. The facade is assumed to be already running at
 * http://127.0.0.1:7897/mcp — this module does NOT start it.
 *
 * Sessions granted the facade get a per-session SCOPED token (remoteTokens.ts)
 * carried as an Authorization header in a per-session config file, so the
 * facade serves them a TIER of its tools (view/triage/operator) instead of the
 * whole surface. The prompts here are deliberately short: detailed usage
 * guidance lives behind the facade's own `help` tool, fetched on demand, so
 * connected agents don't pay context for docs they may never need.
 */

import { app } from 'electron';
import * as path from 'path';
import * as fs from 'fs';
import type { McpServerConfig } from './libraryService';
import type { RemoteTokenScope } from '../shared/ipcTypes';

/** The workspacer MCP facade — an HTTP MCP server started at app launch
 *  (mcpFacadeDaemon). Claude points at it via --mcp-config; managed providers
 *  (Codex/OpenCode) register it through their own MCP config. */
export const MCP_FACADE_URL = 'http://127.0.0.1:7897/mcp';

const MCP_CONFIG_CONTENTS = JSON.stringify(
  { mcpServers: { workspacer: { type: 'http', url: MCP_FACADE_URL } } },
  null,
  2,
);

/** Per-scope role note for facade sessions. One or two
 *  sentences on purpose — the tier's own `help` tool carries the detail. */
function workerRoleNote(scope: RemoteTokenScope): string {
  switch (scope) {
    case 'view':
      return (
        'You have read-only workspacer tools (mcp__workspacer__*) to observe the agent fleet — list agents, read transcripts/conversations/snapshots. ' +
        'You may be asked to digest another session’s activity: reply with a concise, structured summary of what that agent is doing and whether it is blocked. ' +
        'You cannot (and must not try to) spawn, message, or approve agents.'
      );
    case 'triage':
      return (
        'You have workspacer tools (mcp__workspacer__*) to observe the agent fleet and act on its attention needs: read transcripts/snapshots, approve or deny permission prompts, send messages, and interrupt. ' +
        'Call the workspacer "help" tool before first use. Do not attempt to spawn agents or modify host files/config — your tier does not include those tools.'
      );
    default:
      return (
        'You have the full workspacer tool set (mcp__workspacer__*) to observe and drive the agent fleet. ' +
        'Call the workspacer "help" tool before first using the others.'
      );
  }
}

/**
 * Role instructions prepended to a *managed* (headless Claude / Codex /
 * OpenCode) agent's first turn when it's given the facade. The PTY Claude path
 * injects the equivalent through --append-system-prompt (facadeSpawnArgs);
 * managed adapters take no such flag, so the role rides the opening message.
 *
 * TWIN of facadeSpawnArgs: keep the two in step.
 */
export function managedFacadeInstructions(opts: {
  scope?: RemoteTokenScope;
  sessionId?: string;
}): string {
  const scope = opts.scope ?? 'operator';
  // The PTY path has always told a facade session its own id (facadeSpawnArgs'
  // idNote); this path didn't, so a stream-transport manager had to HUNT for
  // itself via list_agents before it could set parentSessionId on a dispatch.
  const idNote = opts.sessionId
    ? ` Your own workspacer session id is ${opts.sessionId} — pass it as parentSessionId when you spawn agents so they nest under you, and never target it with send_message/approve/signal.`
    : '';
  const prefixNote =
    ' Tool names may be prefixed by your runtime (e.g. workspacer__list_agents) — use whichever the workspacer server exposes.';
  return workerRoleNote(scope) + idNote + prefixNote;
}

/**
 * Build the argv fragment that grants a spawned session the workspacer MCP
 * facade (--mcp-config + pre-allowed tools + an --append-system-prompt note).
 *
 * The session gets a short scope-appropriate role note for its tier.
 *
 * When `token` is set the config is a per-session file carrying it as an
 * Authorization header, so the facade serves the session its tier; without a
 * token the shared untokened config is used (legacy operator default).
 *
 * Centralised here so the desktop (ipc) and bus (hubCapabilities) spawn paths
 * stay identical.
 */
export function facadeSpawnArgs(opts: {
  sessionId: string;
  scope?: RemoteTokenScope;
  token?: string;
}): { mcpConfig: string; allowedTools: string[]; appendSystemPrompt: string } {
  const mcpConfig = opts.token
    ? facadeSessionMcpConfig(opts.sessionId, opts.token)
    : supervisorMcpConfigPath();
  const idNote = `Your own workspacer session id is ${opts.sessionId}.`;
  return {
    mcpConfig,
    allowedTools: ['mcp__workspacer'],
    appendSystemPrompt: `${workerRoleNote(opts.scope ?? 'operator')} ${idNote}`,
  };
}

let cachedPath: string | undefined;

/**
 * Returns the absolute path to the shared UNTOKENED facade MCP config file,
 * writing it to `<userData>/supervisor-mcp.json` if it does not yet exist.
 * Sessions with a scoped token use facadeSessionMcpConfig instead.
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

/**
 * Write the per-session facade MCP config carrying the session's scoped token
 * as an Authorization header, and return its path. The header (not a URL query
 * param) keeps the token out of argv — the file rides `--mcp-config <path>`,
 * and /proc/<pid>/cmdline is world-readable where argv is not.
 */
export function facadeSessionMcpConfig(sessionId: string, token: string): string {
  const built = buildSessionMcpConfig(sessionId, [
    {
      id: 'workspacer',
      mcp: {
        type: 'http',
        url: MCP_FACADE_URL,
        headers: { Authorization: `Bearer ${token}` },
      } as McpServerConfig,
    },
  ]);
  if (!built) throw new Error('facadeSessionMcpConfig: failed to build facade entry');
  return built.path;
}

/** The facade URL with the session token as a `t` query param — for providers
 *  whose MCP registration carries only a URL (codex `-c` config overrides,
 *  opencode.json) and cannot send headers. The exposure is argv/config-file
 *  visibility on the local machine, which the untokened facade already granted
 *  MORE than; the token is revoked when the session ends. */
export function facadeUrlWithToken(token: string): string {
  return `${MCP_FACADE_URL}?t=${encodeURIComponent(token)}`;
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
    const entry: Record<string, unknown> = {
      type: cfg.type === 'sse' ? 'sse' : 'http',
      url: cfg.url.trim(),
    };
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
  // 0600: these entries carry live credentials — a library server's real
  // headers/env, or the facade session token.
  fs.writeFileSync(filePath, JSON.stringify({ mcpServers }, null, 2), {
    encoding: 'utf8',
    mode: 0o600,
  });
  return { path: filePath, toolNames };
}
