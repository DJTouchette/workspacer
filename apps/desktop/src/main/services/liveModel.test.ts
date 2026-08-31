import { beforeEach, describe, expect, it, vi } from 'vitest';

const setModel = vi.fn();
const getSnapshot = vi.fn();
const noteRequestedModelSelection = vi.fn();

vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { setModel: (...args: unknown[]) => setModel(...args) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    getSnapshot: (...args: unknown[]) => getSnapshot(...args),
    noteRequestedModelSelection: (...args: unknown[]) => noteRequestedModelSelection(...args),
  },
}));

import { applyLiveModel } from './liveModel';

beforeEach(() => {
  vi.clearAllMocks();
  getSnapshot.mockReturnValue({ provider: 'claude' });
  setModel.mockResolvedValue({ ok: true, queued: false, disposition: 'accepted' });
});

describe('applyLiveModel', () => {
  it('normalizes a legacy marker, forwards the pair, and supports old daemon responses', async () => {
    const result = await applyLiveModel('s1', { model: 'opus[1m]' });

    expect(setModel).toHaveBeenCalledWith('s1', 'opus[1m]', undefined, 'opus', 1_000_000);
    expect(noteRequestedModelSelection).toHaveBeenCalledWith(
      's1',
      { model: 'opus', contextWindow: 1_000_000 },
      'opus[1m]',
    );
    expect(result.requestedSelection).toEqual({ model: 'opus', contextWindow: 1_000_000 });
  });

  it('uses owner truth instead of the locally requested pair', async () => {
    setModel.mockResolvedValue({
      ok: true,
      model: 'fable',
      requestedSelection: { model: 'fable', contextWindow: 1_000_000 },
      queued: false,
      disposition: 'accepted',
    });

    const result = await applyLiveModel('s1', {
      model: 'opus[1m]',
      modelIdentity: 'opus',
      contextWindow: 1_000_000,
    });

    expect(noteRequestedModelSelection).toHaveBeenCalledWith(
      's1',
      { model: 'fable', contextWindow: 1_000_000 },
      'fable',
    );
    expect(result.requestedSelection?.model).toBe('fable');
  });

  it('rejects a marker/pair conflict before reaching the daemon', async () => {
    const result = await applyLiveModel('s1', {
      model: 'opus[1m]',
      modelIdentity: 'sonnet',
      contextWindow: 1_000_000,
    });

    expect(result.ok).toBe(false);
    expect(result.error).toBe('conflicting-model-identity');
    expect(setModel).not.toHaveBeenCalled();
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });

  it('keeps native-1M Fable bare at the compatibility boundary', async () => {
    await applyLiveModel('s1', {
      model: 'fable',
      modelIdentity: 'fable',
      contextWindow: 1_000_000,
    });
    expect(setModel).toHaveBeenCalledWith('s1', 'fable', undefined, 'fable', 1_000_000);
  });

  it('does not mutate the mirror when the owner refuses', async () => {
    setModel.mockResolvedValue({ ok: false, error: 'queue full' });
    const result = await applyLiveModel('s1', { model: 'opus' });
    expect(result).toEqual({ ok: false, error: 'queue full' });
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });

  it('refuses combined Claude PTY model and effort without claiming either', async () => {
    const result = await applyLiveModel('s1', { model: 'opus', effort: 'high' });
    expect(result).toEqual({
      ok: false,
      error: 'claude-pty-effort-unsupported: the PTY model command cannot deliver effort',
    });
    expect(setModel).not.toHaveBeenCalled();
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });

  it('still delivers combined model and effort structurally for Claude stream', async () => {
    getSnapshot.mockReturnValue({ provider: 'claude', transport: 'stream' });
    await applyLiveModel('s1', { model: 'opus', effort: 'high' });
    expect(setModel).toHaveBeenCalledWith('s1', 'opus', 'high', 'opus', null);
  });

  it('rejects whitespace with the stable code before reaching the owner', async () => {
    const result = await applyLiveModel('s1', { model: ' \t ' });
    expect(result).toEqual({ ok: false, error: 'empty-model' });
    expect(setModel).not.toHaveBeenCalled();
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });

  it('rejects control bytes before reaching the owner or mirror', async () => {
    const result = await applyLiveModel('s1', { model: 'opus\u001b[201~/help' });
    expect(result).toEqual({ ok: false, error: 'invalid-model-identity' });
    expect(setModel).not.toHaveBeenCalled();
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });

  it('does not claim compatibility with a pre-Phase-5 PTY owner', async () => {
    setModel.mockResolvedValue({ ok: true });
    const result = await applyLiveModel('s1', { model: 'opus' });
    expect(result).toEqual({
      ok: false,
      error:
        'upgrade-required: the session owner does not support durable Claude PTY model switching',
    });
    expect(noteRequestedModelSelection).not.toHaveBeenCalled();
  });
});
