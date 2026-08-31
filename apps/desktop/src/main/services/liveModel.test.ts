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
  setModel.mockResolvedValue({ ok: true });
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
    expect(result.error).toContain('conflicts');
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
});
