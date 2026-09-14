import { intentWorkspaceStoreIfUsed, captureIntentSessions } from './intentWorkspaceStore';
import type { IntentAutomationEffects } from './intentAutomationStore';
import type { IntentLiveSession } from '../shared/intentWorkspace';

/** Owner process only. Timers perform no model calls, and continue without a viewer. */
export function startIntentAutomationRuntime(
  sessions: () => readonly IntentLiveSession[],
  effects: IntentAutomationEffects,
) {
  let busy = false;
  const tick = async () => {
    if (busy) return;
    busy = true;
    try {
      const store = await intentWorkspaceStoreIfUsed();
      if (store) {
        const live = sessions();
        store.capture(captureIntentSessions(live));
        await store.automation.tick(live, effects);
      }
    } catch (error) {
      console.warn('[intent-automation]', error);
    } finally {
      busy = false;
    }
  };
  const timer = setInterval(() => void tick(), 5000);
  timer.unref();
  void tick();
  return () => clearInterval(timer);
}
