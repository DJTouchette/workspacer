import type { IntentDirectionDelivery } from './intentSteeringStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { callHub } from './hubClient';
import { managerReplacementState } from './managerReplacementState';
import { ManagerDeliveryRejected } from '../shared/managerReplacement';

/** Route by the saved qualified identity, never remoteHubOf(sessionId), which
 * could resolve a same-ID session on a different machine after reconnect. */
export const deliverIntentDirection: IntentDirectionDelivery = async (target, packet) => {
  if (
    !/^[A-Za-z0-9_.-]{1,128}$/.test(target.sessionId) ||
    target.sessionId.includes('..') ||
    (target.hub && !/^[A-Za-z0-9_.-]{1,128}$/.test(target.hub))
  )
    return { status: 'failed', detail: 'Invalid target identity. No message was sent.' };
  if (target.hub) {
    // Peer exceptions do not reliably distinguish refusal from a lost ACK.
    const result = (await callHub(`hub:${target.hub}/agents.sendMessage`, {
      sessionId: target.sessionId,
      text: packet,
    })) as { ok?: boolean } | undefined;
    return result?.ok === true
      ? {
          status: 'accepted',
          detail: 'Accepted by the peer messaging service; the message may be queued.',
        }
      : {
          status: 'unknown',
          detail: 'The peer did not return a confirmed message acknowledgment.',
        };
  }
  try {
    managerReplacementState.assertAvailable(target.sessionId);
  } catch (error) {
    return { status: 'failed', detail: `${String(error)} No message was sent.` };
  }
  try {
    return await managerReplacementState.admitted([target.sessionId], async () => {
      const result = await claudemonSessionClient.message(target.sessionId, packet);
      if (result.mode === 'handoff-queued')
        return {
          status: 'unknown' as const,
          detail:
            'Message held by manager handoff; delivery to the selected session is unconfirmed.',
        };
      return result.ok
        ? {
            status: 'accepted' as const,
            detail: 'Accepted by the local messaging service; the message may be queued.',
          }
        : { status: 'failed' as const, detail: 'The daemon rejected the message.' };
    });
  } catch (error) {
    if (error instanceof ManagerDeliveryRejected)
      return { status: 'failed', detail: error.message };
    throw error; // The store records ambiguity; never fall back to raw input.
  }
};
