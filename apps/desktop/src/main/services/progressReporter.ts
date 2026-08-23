/**
 * The live ProgressReports singleton, wired to the real session store and the
 * real wake channel. Split from progressReports.ts so the logic there stays
 * pure and testable — the store and claudemon are injected, not imported.
 *
 * Same split, same reasons, as thresholdWatcher.ts beside it; the two are the
 * host-side and worker-side halves of "tell the manager without it polling".
 */
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { ProgressReports, type ReportableSession } from './progressReports';

export const progressReporter = new ProgressReports(
  // claudemon's /message QUEUES while the recipient is mid-turn and delivers
  // once its prompt settles, which is what makes an unsolicited wake safe to
  // send at a manager that is busy. Same channel every other fleet wake uses.
  (sessionId, text) => claudemonSessionClient.message(sessionId, text),
  () => claudeSessionStore.getAllSnapshots() as unknown as ReportableSession[],
);
