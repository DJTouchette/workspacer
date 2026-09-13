import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: { message: vi.fn() } }));
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
import { ManagerDeliveryRejected } from '../shared/managerReplacement';
import { deliverIntentDirection } from './intentDirectionDelivery';
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(claudemonSessionClient.message).mockReset().mockResolvedValue({ ok: true });
  vi.mocked(callHub).mockReset().mockResolvedValue({ ok: true });
  vi.mocked(managerReplacementState.assertAvailable).mockReset();
});
it('uses the existing local messaging client and records only its acceptance', async () => {
  expect(await deliverIntentDirection(target, 'Pinned packet')).toMatchObject({
    status: 'accepted',
  });
  expect(claudemonSessionClient.message).toHaveBeenCalledWith('worker', 'Pinned packet');
  expect(managerReplacementState.admitted).toHaveBeenCalledWith(['worker'], expect.any(Function));
});
it('distinguishes explicit local refusal from a lost response or handoff queue', async () => {
  vi.mocked(claudemonSessionClient.message).mockRejectedValueOnce(new ManagerDeliveryRejected(404));
  expect(await deliverIntentDirection(target, 'packet')).toMatchObject({ status: 'failed' });
  vi.mocked(claudemonSessionClient.message).mockRejectedValueOnce(new Error('Socket closed'));
  await expect(deliverIntentDirection(target, 'packet')).rejects.toThrow('Socket closed');
  vi.mocked(claudemonSessionClient.message).mockResolvedValueOnce({
    ok: true,
    mode: 'handoff-queued',
  });
  expect(await deliverIntentDirection(target, 'packet')).toMatchObject({ status: 'unknown' });
});
it('pins the peer hub, and never falls back to a same-ID local session after a peer error', async () => {
  const peer = { ...target, hub: 'peer-one' };
  expect(await deliverIntentDirection(peer, 'packet')).toMatchObject({ status: 'accepted' });
  expect(callHub).toHaveBeenCalledWith('hub:peer-one/agents.sendMessage', {
    sessionId: 'worker',
    text: 'packet',
  });
  vi.mocked(callHub).mockRejectedValueOnce(new Error('Peer timeout'));
  await expect(deliverIntentDirection(peer, 'packet')).rejects.toThrow('Peer timeout');
  expect(claudemonSessionClient.message).not.toHaveBeenCalled();
});
it('refuses fenced or invalid targets before dispatch', async () => {
  vi.mocked(managerReplacementState.assertAvailable).mockImplementationOnce(() => {
    throw new Error('Manager handoff in progress');
  });
  expect(await deliverIntentDirection(target, 'packet')).toMatchObject({ status: 'failed' });
  expect(
    await deliverIntentDirection({ ...target, sessionId: '../other' }, 'packet'),
  ).toMatchObject({ status: 'failed' });
  expect(claudemonSessionClient.message).not.toHaveBeenCalled();
});
