import { callHub } from './hubClient';
import { claudeSessionStore } from './claudeSessionStore';
import { deliverIntentDirection } from './intentDirectionDelivery';
import { deliverIntentControl } from './intentControlDelivery';
import { resolveManagerProvider } from '../lib/roleProviders';
import type { IntentAutomationEffects } from './intentAutomationStore';
import { startIntentAutomationRuntime } from './intentAutomationRuntime';

export const nativeIntentAutomation: IntentAutomationEffects = {
  async spawn(cwd, label, message) {
    const provider = resolveManagerProvider();
    const result = (await callHub('agents.spawn', {
      cwd,
      label,
      message,
      provider,
      manager: true,
      toolScope: 'operator',
      transport: 'stream',
    })) as { sessionId?: string; messageQueued?: boolean };
    if (!result?.sessionId) throw new Error('No confirmed manager identity returned');
    const session = { sessionId: result.sessionId, hub: '', cwd, label, provider };
    // Do not repeat a possibly queued kickoff on older daemons.
    if (result.messageQueued !== true)
      throw new Error(
        `Manager ${result.sessionId} exists but kickoff is unconfirmed. Link and inspect it in Execution.`,
      );
    return session;
  },
  send: deliverIntentDirection,
  async interrupt(target) {
    const descendants = new Set([target.sessionId]);
    const sessions = claudeSessionStore.getAllSnapshots();
    for (let changed = true; changed;) {
      changed = false;
      for (const session of sessions)
        if (
          (session.hub || '') === target.hub &&
          session.parentSessionId &&
          descendants.has(session.parentSessionId) &&
          !descendants.has(session.sessionId)
        ) {
          descendants.add(session.sessionId);
          changed = true;
        }
    }
    for (const id of descendants) {
      const result = await deliverIntentControl({ ...target, sessionId: id }, 'interrupt', '');
      if (result.status !== 'accepted') return result;
    }
    return {
      status: 'accepted',
      detail:
        'Interrupt requested for the manager and its current workers. Inspect any external background work separately.',
    };
  },
};
let started = false;
export function ensureNativeIntentAutomation() {
  if (started) return;
  started = true;
  startIntentAutomationRuntime(() => claudeSessionStore.getAllSnapshots(), nativeIntentAutomation);
}
