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
  // Separate busy guard: slow provider I/O must not delay reactive manager delivery.
  let sourceBusy = false;
  let stopped = false;
  const sync = async () => {
    if (sourceBusy || stopped) return;
    sourceBusy = true;
    try {
      const store = await intentWorkspaceStoreIfUsed();
      if (!stopped) await store?.sources.tick();
    } catch {
      /* Source failures are persisted by the source store; no raw HTTP errors in logs. */
    } finally {
      sourceBusy = false;
    }
  };
  const sourceTimer = setInterval(() => void sync(), 5000);
  sourceTimer.unref();
  void sync();
  const timer = setInterval(() => void tick(), 5000);
  timer.unref();
  void tick();
  return () => {
    stopped = true;
    clearInterval(timer);
    clearInterval(sourceTimer);
  };
}
