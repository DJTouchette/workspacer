import type { IntentControlDelivery } from './intentControlStore';
import { deliverIntentDirection } from './intentDirectionDelivery';
import { claudemonSessionClient } from './claudemonSessionClient';
import { callHub } from './hubClient';
import { managerReplacementState } from './managerReplacementState';

export const deliverIntentControl: IntentControlDelivery = async (target, kind, packet) => {
  if (kind === 'continue') return deliverIntentDirection(target, packet);
  if (
    !/^[A-Za-z0-9_.-]{1,128}$/.test(target.sessionId) ||
    target.sessionId.includes('..') ||
    (target.hub && !/^[A-Za-z0-9_.-]{1,128}$/.test(target.hub))
  )
    return { status: 'failed', detail: 'Invalid target identity. No interrupt was sent.' };
  if (target.hub) {
    const result = (await callHub(`hub:${target.hub}/claude.signal`, {
      sessionId: target.sessionId,
      signal: 'SIGINT',
    })) as { ok?: boolean } | undefined;
    return result?.ok === true
      ? {
          status: 'accepted',
          detail:
            'The peer signal service accepted the interrupt request. Inspect the session to assess its effect.',
        }
      : { status: 'unknown', detail: 'The peer did not confirm interrupt acceptance.' };
  }
  try {
    managerReplacementState.assertAvailable(target.sessionId);
  } catch (error) {
    return { status: 'failed', detail: `${String(error)} No interrupt was sent.` };
  }
  return managerReplacementState.admitted([target.sessionId], async () => {
    await claudemonSessionClient.signal(target.sessionId, 'SIGINT');
    return {
      status: 'accepted',
      detail:
        'The local signal service accepted the interrupt request. Background work may continue; no work has been rolled back.',
    };
  });
};
