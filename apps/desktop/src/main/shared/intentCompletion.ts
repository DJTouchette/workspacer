import type { IntentLiveSession, IntentSessionRef } from './intentWorkspace';

export const INTENT_REPORT_LIMIT = 4000;
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

/** No transcript, tool input or model call. Preserve report formatting except
 * bounded known credential patterns; disclose any redaction to the reviewer. */
export function boundIntentReport(text: string): { report: string; redacted: boolean } {
  const bounded = text.slice(0, INTENT_REPORT_LIMIT);
  const report = bounded
    .replace(
      /-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*(?:-----END [^-]*PRIVATE KEY-----|$)/g,
      '[redacted private key]',
    )
    .replace(/\b(?:Bearer|Basic)\s+[A-Za-z0-9+/_=.-]+/gi, '[redacted authorization]')
    .replace(
      /\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9_]{16,}|github_pat_[A-Za-z0-9_]+)/g,
      '[redacted token]',
    )
    .replace(
      /((?:password|api[_-]?key|access[_-]?token|secret)\s*[=:]\s*)[^\s,;"}]+/gi,
      '$1[redacted]',
    )
    .replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/gi, '$1[redacted]@');
  return { report, redacted: report !== bounded };
}
