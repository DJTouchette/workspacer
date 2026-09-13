import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { signal: vi.fn(), message: vi.fn() },
}));
vi.mock('./hubClient', () => ({ callHub: vi.fn() }));
vi.mock('./managerReplacementState', () => ({
  managerReplacementState: {
    assertAvailable: vi.fn(),
    admitted: vi.fn(async (_ids, run) => run()),
  },
}));
import { claudemonSessionClient } from './claudemonSessionClient';
import { callHub } from './hubClient';
import { managerReplacementState } from './managerReplacementState';
import { deliverIntentControl } from './intentControlDelivery';
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(claudemonSessionClient.signal).mockReset().mockResolvedValue();
  vi.mocked(claudemonSessionClient.message).mockReset().mockResolvedValue({ ok: true });
  vi.mocked(callHub).mockReset().mockResolvedValue({ ok: true });
  vi.mocked(managerReplacementState.assertAvailable).mockReset();
});
it('interrupts through the existing provider-neutral signal client with manager admission; never sends the recorded reason as a message', async () => {
  expect(await deliverIntentControl(target, 'interrupt', 'Recorded reason')).toMatchObject({
    status: 'accepted',
  });
  expect(claudemonSessionClient.signal).toHaveBeenCalledWith('worker', 'SIGINT');
  expect(managerReplacementState.admitted).toHaveBeenCalledWith(['worker'], expect.any(Function));
  expect(claudemonSessionClient.message).not.toHaveBeenCalled();
});
it('continues through the existing message client without signalling or respawning', async () => {
  expect(await deliverIntentControl(target, 'continue', 'Exact saved packet')).toMatchObject({
    status: 'accepted',
  });
  expect(claudemonSessionClient.message).toHaveBeenCalledWith('worker', 'Exact saved packet');
  expect(claudemonSessionClient.signal).not.toHaveBeenCalled();
});
it('pins peer-qualified signal identity and never falls back after a lost peer response', async () => {
  const peer = { ...target, hub: 'peer' };
  await deliverIntentControl(peer, 'interrupt', '');
  expect(callHub).toHaveBeenCalledWith('hub:peer/claude.signal', {
    sessionId: 'worker',
    signal: 'SIGINT',
  });
  vi.mocked(callHub).mockRejectedValueOnce(new Error('Timeout'));
  await expect(deliverIntentControl(peer, 'interrupt', '')).rejects.toThrow('Timeout');
  expect(claudemonSessionClient.signal).not.toHaveBeenCalled();
});
it('refuses malformed or locally fenced targets before I/O and preserves post-I/O ambiguity', async () => {
  expect(
    await deliverIntentControl({ ...target, sessionId: '../worker' }, 'interrupt', ''),
  ).toMatchObject({ status: 'failed' });
  vi.mocked(managerReplacementState.assertAvailable).mockImplementationOnce(() => {
    throw new Error('Manager replacement');
  });
  expect(await deliverIntentControl(target, 'interrupt', '')).toMatchObject({ status: 'failed' });
  expect(claudemonSessionClient.signal).not.toHaveBeenCalled();
  vi.mocked(claudemonSessionClient.signal).mockRejectedValueOnce(new Error('Socket closed'));
  await expect(deliverIntentControl(target, 'interrupt', '')).rejects.toThrow('Socket closed');
});
