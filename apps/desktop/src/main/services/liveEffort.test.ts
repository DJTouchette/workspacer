/**
 * The shared live-effort body, which both the `claude:setEffort` IPC handler and
 * the `claude.setEffort` hub capability call. What matters here is the *provider
 * branch*: claude takes the `/effort` slash command through the message path
 * (there is no structural control for it — the stream protocol has only
 * set_model / set_permission_mode), while managed providers take the daemon's
 * model endpoint with effort only. Getting the branch backwards would silently
 * post "/effort high" as a literal prompt to codex, or drop the level entirely
 * for claude.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const message = vi.fn(async () => ({ ok: true }) as { ok: boolean; mode?: string });
const setModel = vi.fn(async () => ({ ok: true }) as { ok: boolean; error?: string });
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: {
    message: (...a: unknown[]) => message(...(a as [string, string])),
    setModel: (...a: unknown[]) => setModel(...(a as [string, string?, string?])),
  },
}));

const noteEffort = vi.fn();
const getSnapshot = vi.fn(() => ({ provider: 'claude' }) as { provider?: string } | null);
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    noteEffort: (...a: unknown[]) => noteEffort(...(a as [string, string])),
    getSnapshot: (...a: unknown[]) => getSnapshot(...(a as [string])),
  },
}));

import { applyLiveEffort } from './liveEffort';

beforeEach(() => {
  message.mockClear();
  setModel.mockClear();
  noteEffort.mockClear();
  getSnapshot.mockReturnValue({ provider: 'claude' });
  message.mockResolvedValue({ ok: true });
  setModel.mockResolvedValue({ ok: true });
});

describe('applyLiveEffort', () => {
  it('claude goes through the message path as a slash command', async () => {
    const res = await applyLiveEffort('s1', 'high');

    expect(message).toHaveBeenCalledWith('s1', '/effort high');
    expect(setModel).not.toHaveBeenCalled();
    expect(res).toEqual({ ok: true, effort: 'high' });
    expect(noteEffort).toHaveBeenCalledWith('s1', 'high');
  });

  it('a managed provider goes structural, with effort only', async () => {
    getSnapshot.mockReturnValue({ provider: 'codex' });

    const res = await applyLiveEffort('s1', 'xhigh');

    expect(setModel).toHaveBeenCalledWith('s1', undefined, 'xhigh');
    expect(message).not.toHaveBeenCalled();
    expect(res.ok).toBe(true);
  });

  it('an unknown session defaults to the claude mechanism', async () => {
    getSnapshot.mockReturnValue(null);
    await applyLiveEffort('s1', 'low');
    expect(message).toHaveBeenCalledWith('s1', '/effort low');
  });

  it('a session that cannot take input reports why and notes nothing', async () => {
    message.mockResolvedValue({ ok: false, mode: 'stopped' });

    const res = await applyLiveEffort('s1', 'high');

    expect(res.ok).toBe(false);
    expect(res.error).toContain('stopped');
    expect(noteEffort).not.toHaveBeenCalled();
  });

  it("passes a managed provider's refusal through for the restart fallback", async () => {
    getSnapshot.mockReturnValue({ provider: 'codex' });
    setModel.mockResolvedValue({ ok: false, error: 'rollout fallback cannot switch live' });

    const res = await applyLiveEffort('s1', 'high');

    expect(res).toEqual({ ok: false, error: 'rollout fallback cannot switch live' });
    expect(noteEffort).not.toHaveBeenCalled();
  });

  it('a thrown transport error is reported, not propagated', async () => {
    message.mockRejectedValue(new Error('message HTTP 500'));

    const res = await applyLiveEffort('s1', 'high');

    expect(res).toEqual({ ok: false, error: 'message HTTP 500' });
    expect(noteEffort).not.toHaveBeenCalled();
  });

  it('rejects an empty level rather than sending a bare "/effort"', async () => {
    const res = await applyLiveEffort('s1', '   ');

    expect(res.ok).toBe(false);
    expect(message).not.toHaveBeenCalled();
  });
});
