import type { IntentLiveSession } from '../shared/intentWorkspace';
import {
  captureIntentSessions,
  intentWorkspaceStoreIfUsed,
} from '../services/intentWorkspaceStore';

/** Sparse brain snapshots intentionally carry no transcript. Read the daemon's
 * bounded data projection only for explicitly linked local sessions. No LLM call,
 * browser demand, or whole-transcript download is involved.
 */
async function report(
  daemonURL: string,
  sessionId: string,
): Promise<{ text: string; truncated: boolean; interrupted: boolean; redacted: boolean }> {
  const response = await fetch(
    `${daemonURL.replace(/\/$/, '')}/sessions/${encodeURIComponent(sessionId)}/conversation?completion_source=1`,
    {
      signal: AbortSignal.timeout(3000),
    },
  );
  if (!response.ok || !response.body) throw new Error(`Report read failed (${response.status})`);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > 26_000) {
        await reader.cancel();
        throw new Error('Daemon report projection exceeded its size limit');
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const source = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  if (
    source?.projection !== 'intent-completion-source/v1' ||
    source.sessionId !== sessionId ||
    source.redactionVersion !== 1 ||
    typeof source.redacted !== 'boolean' ||
    typeof source.text !== 'string' ||
    source.text.length > 4000 ||
    typeof source.truncated !== 'boolean' ||
    typeof source.interrupted !== 'boolean'
  )
    throw new Error('Update the daemon to support bounded completion capture');
  return {
    text: source.text,
    truncated: source.truncated,
    interrupted: source.interrupted,
    redacted: source.redacted,
  };
}

export async function captureHeadlessIntentSessions(
  sessions: readonly IntentLiveSession[],
  daemonURL?: string,
): Promise<void> {
  const store = await intentWorkspaceStoreIfUsed();
  if (!store) return;
  const tracked = new Set(
    store.trackedSessions().map((session) => JSON.stringify([session.hub, session.sessionId])),
  );
  const candidates = sessions.filter(
    (session) =>
      !session.hubOffline && tracked.has(JSON.stringify([session.hub || '', session.sessionId])),
  );
  const samples = captureIntentSessions(candidates, sessions);
  let index = 0;
  // Bound concurrency even for a large fleet. Existing native/full snapshots and
  // peer rows never cause a request against a same-ID local daemon session.
  await Promise.all(
    Array.from({ length: Math.min(4, candidates.length) }, async () => {
      while (index < candidates.length) {
        const i = index++,
          session = candidates[i];
        if (session.hub || session.conversation !== undefined) continue;
        if (!daemonURL) {
          store.reportCaptureResult(session.sessionId, 'Daemon report source unavailable');
          continue;
        }
        try {
          const final = await report(daemonURL, session.sessionId);
          store.reportCaptureResult(session.sessionId);
          samples[i].finalReport = final;
          samples[i].observation.completionIdle = !!samples[i].completionIdle && !final.interrupted;
          if (final.text && !session.pendingApproval && !session.pendingQuestions?.length)
            samples[i].observation.summary = final.text;
        } catch (error) {
          store.reportCaptureResult(
            session.sessionId,
            error instanceof Error ? error.message : String(error),
          );
        }
      }
    }),
  );
  store.capture(samples);
}
