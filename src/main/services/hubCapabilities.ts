/**
 * Real capabilities the main process exposes on the hub bus. These are the
 * inverse of events — things a plugin (or, later, Claude via the MCP facade)
 * can *ask workspacer to do*. Kept small and explicit; each is a future MCP tool.
 */

import { Notification } from 'electron';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { registerCapability } from './hubClient';

export function registerHubCapabilities(): void {
  // Read-only: list live agents with light state. The bread-and-butter "what's
  // running?" call for any dashboard plugin or MCP client.
  registerCapability('agents.list', () =>
    claudeSessionStore.getAllSnapshots().map((s) => ({
      sessionId: s.sessionId,
      cwd: s.cwd,
      state: s.ambientState,
      model: s.usage?.model ?? null,
      contextTokens: s.usage?.contextTokens ?? 0,
      contextLimit: s.usage?.contextLimit ?? 0,
      costUSD: s.usage?.costUSD ?? 0,
    })),
  );

  // Control: send a prompt to an agent. Only succeeds when that session is at an
  // input prompt (claudemon enforces mode=input).
  registerCapability('agents.sendMessage', async (params: unknown) => {
    const { sessionId, text } = (params ?? {}) as { sessionId?: string; text?: string };
    if (!sessionId || typeof text !== 'string') {
      throw new Error('agents.sendMessage requires { sessionId, text }');
    }
    return claudemonSessionClient.message(sessionId, text);
  });

  // Surface an OS notification.
  registerCapability('notifications.post', (params: unknown) => {
    const { title, body } = (params ?? {}) as { title?: string; body?: string };
    new Notification({ title: title || 'workspacer', body: body || '' }).show();
    return { ok: true };
  });
}
