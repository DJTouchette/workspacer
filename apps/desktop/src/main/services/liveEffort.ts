/**
 * Shared live reasoning-effort switch — no restart, conversation untouched.
 *
 * Two entry points reach it, the `claude:setEffort` IPC handler and the
 * `claude.setEffort` hub capability (web / remote client + MCP facade), and this
 * is the single body they both call so they can't drift — the standing rule that
 * spawnManagedAgent follows. It matters more here than for the sibling switches
 * because this one *branches on provider*: a copy-paste pair would diverge the
 * moment a provider's mechanism changes.
 *
 * The two mechanisms:
 *  - claude, both transports: the `/effort <level>` slash command through the
 *    normal message path — the same shape as the `/model` switch. Verified on the
 *    wire: the CLI answers "Set effort level to high (this session only)" rather
 *    than treating it as a prompt, and there is no `set_effort` in the stream
 *    control protocol (only set_model / set_permission_mode), so the message path
 *    is the mechanism on stream too.
 *  - managed (codex): the daemon's `/sessions/:id/model` with effort only, which
 *    applies `thread/settings/update` to the running thread.
 *
 * Either way the level is noted in the snapshot store immediately. For claude
 * that note is the pill's only truth — its effective effort appears in no hook,
 * status line or init frame — while codex's own `thread/settings/updated`
 * confirmation arrives on the status line and supersedes it.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';

export interface LiveEffortResult {
  ok: boolean;
  effort?: string;
  /** Why it couldn't be applied live — the caller offers the restart path. */
  error?: string;
}

export async function applyLiveEffort(
  sessionId: string,
  effort: string,
): Promise<LiveEffortResult> {
  const level = effort.trim();
  if (!sessionId || !level) return { ok: false, error: 'requires a session and an effort level' };

  const provider = claudeSessionStore.getSnapshot(sessionId)?.provider ?? 'claude';
  try {
    if (provider === 'claude') {
      const res = await claudemonSessionClient.message(sessionId, `/effort ${level}`);
      // A 409 carries the live mode: the session has ended and can't take input.
      if (!res.ok) {
        return {
          ok: false,
          error: `this session can't take input right now (${res.mode ?? 'unknown'})`,
        };
      }
    } else {
      const res = await claudemonSessionClient.setModel(sessionId, undefined, level);
      if (!res.ok) return { ok: false, error: res.error };
    }
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }

  claudeSessionStore.noteEffort(sessionId, level);
  return { ok: true, effort: level };
}
