import type { IntentLiveSession, IntentSessionRef } from './intentWorkspace';

export { boundIntentReport, INTENT_REPORT_LIMIT } from './intentReport';
export interface IntentCompletionProposal {
  id: string;
  workspaceId: string;
  intentRevision: number;
  executionId: string;
  runId?: string;
  session: IntentSessionRef;
  completedAt: string | null;
  capturedAt: string;
  report: string;
  reportState: 'reported' | 'missing' | 'malformed' | 'oversized';
  structuredState?: 'provided' | 'missing' | 'malformed';
  summary?: string;
  checks?: string[];
  artifacts?: string[];
  caveats?: string[];
  followUps?: string[];
  provenance: 'owner-host-final-assistant/v1';
  redacted: boolean;
}
export interface IntentCompletionView {
  action: 'completionProposals';
  proposals: IntentCompletionProposal[];
  currentProposalId: string | null;
}

/** Only the owner host supplies these inputs. Idle is necessary, never sufficient:
 * an explicit, correlated outcome report is also required by the completion store.
 * Include descendants and embedded background tasks, even when the parent idles.
 */
export function intentCompletionIdle(
  session: IntentLiveSession,
  sessions: readonly IntentLiveSession[],
): boolean {
  const clear = (s: IntentLiveSession) =>
    !s.hubOffline &&
    !['ended', 'stopped'].includes(s.status || '') &&
    s.ambientState === 'idle' &&
    !s.pendingApproval &&
    !s.pendingQuestions?.length &&
    !s.subagents?.some((child) => child.status === 'running') &&
    !s.activeToolCalls?.length;
  if (!clear(session)) return false;
  const descendants = new Set([session.sessionId]);
  for (let changed = true; changed;) {
    changed = false;
    for (const child of sessions)
      if (
        (child.hub || '') === (session.hub || '') &&
        child.parentSessionId &&
        descendants.has(child.parentSessionId) &&
        !descendants.has(child.sessionId)
      ) {
        descendants.add(child.sessionId);
        changed = true;
      }
  }
  return !sessions.some(
    (s) =>
      s.sessionId !== session.sessionId &&
      (s.hub || '') === (session.hub || '') &&
      descendants.has(s.sessionId) &&
      !['ended', 'stopped'].includes(s.status || '') &&
      !clear(s),
  );
}
