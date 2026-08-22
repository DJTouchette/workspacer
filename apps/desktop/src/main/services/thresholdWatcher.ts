/**
 * The live ThresholdWatcher singleton, wired to the real session store and the
 * real wake channel. Split from thresholdWatch.ts so the logic there stays pure
 * and testable — the store and claudemon are injected, not imported.
 */
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { ThresholdWatcher, type WatchableSession } from './thresholdWatch';

export const thresholdWatcher = new ThresholdWatcher(
  (sessionId, text) => claudemonSessionClient.message(sessionId, text),
  () => claudeSessionStore.getAllSnapshots() as unknown as WatchableSession[],
);
