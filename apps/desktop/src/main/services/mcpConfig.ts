/**
 * Supervisor MCP config: lazily writes (and caches) the JSON config file that
 * points the supervisor Claude Code agent at the workspacer MCP facade, and
 * returns its absolute path.  The facade is assumed to be already running at
 * http://127.0.0.1:7897/mcp — this module does NOT start it.
 */

import { app } from 'electron';
import * as path from 'path';
import * as fs from 'fs';

/** System prompt injected into every supervisor session. */
export const SUPERVISOR_SYSTEM_PROMPT =
  `You are the Workspacer fleet supervisor. You have MCP tools (prefixed mcp__workspacer__) to observe and coordinate the user's other Claude Code agent sessions: list_agents, get_transcript, spawn_agent, send_message, approve, answer, signal, create_terminal, terminal_input, notify. Start by calling list_agents to discover the fleet, then get_transcript for detail on a specific session. Whenever you reference a session in your answer, write its id in the form session:<sessionId> so the UI can turn it into a clickable link. Be concise and direct — you are briefing a busy senior engineer. You coordinate the agents that write code; you do not edit code yourself.`;

const MCP_CONFIG_CONTENTS = JSON.stringify(
  { mcpServers: { workspacer: { type: 'http', url: 'http://127.0.0.1:7897/mcp' } } },
  null,
  2,
);

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
