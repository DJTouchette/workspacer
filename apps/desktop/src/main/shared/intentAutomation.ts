import type { IntentSessionRef } from './intentWorkspace';

export type IntentRunState =
  'queued' | 'starting' | 'working' | 'waiting' | 'paused' | 'review' | 'complete' | 'uncertain';
export interface IntentRun {
  id: string;
  workspaceId: string;
  intentRevision: number;
  executionId: string;
  state: IntentRunState;
  session: IntentSessionRef | null;
  message: string;
  report: string;
  deadline: string;
  updatedAt: string;
  /** Persisted before external effects; never replayed on restart. */
  operation?: 'spawn' | 'send' | 'interrupt';
  needsInterrupt?: boolean;
  automaticContinuations?: number;
  baselineSummary?: string;
}
export type IntentAutomationRequest =
  | { action: 'automation'; id: string }
  | {
      action: 'resumeInspectedIntent';
      id: string;
      runId: string;
      expectedRevision: number;
      text: string;
    }
  | { action: 'activateIntent'; id: string; expectedRevision: number; minutes?: number }
  | {
      action: 'restartIntent';
      id: string;
      runId: string;
      expectedRevision: number;
      minutes?: number;
    }
  | { action: 'pauseIntent'; id: string; runId: string }
  | { action: 'answerIntent'; id: string; runId: string; expectedRevision: number; text: string };
export type IntentAutomationResponse = {
  action: IntentAutomationRequest['action'];
  run: IntentRun | null;
};

/** A report routes work to human review; it cannot verify or approve it. */
export function readIntentRunReport(
  text: string,
  run: IntentRun,
): { state: 'waiting' | 'review'; summary: string } | undefined {
  const match = /```intent-report\s*\n([\s\S]*?)\n```\s*$/.exec(text);
  if (!match) return;
  try {
    const value = JSON.parse(match[1]);
    if (
      value.runId !== run.id ||
      value.revision !== run.intentRevision ||
      !['waiting', 'review'].includes(value.state) ||
      typeof value.summary !== 'string' ||
      !value.summary.trim() ||
      value.summary.length > 3000
    )
      return;
    return { state: value.state, summary: value.summary.trim() };
  } catch {
    return;
  }
}

export function intentRunInstructions(run: IntentRun): string {
  return [
    'You are dedicated to this intent only. Do not adopt unrelated workers, act on other projects, or read a standalone fleet handoff. The saved intent is your complete scope. Pursue this intent autonomously until its success criteria are ready for user review or you need a meaningful decision. Make routine implementation choices yourself. Delegate when useful and rely on worker wakes. Do not stop merely to offer to continue. Ask about scope, product decisions and missing authority with a recommendation. Stay within the saved constraints and existing permissions. Do not merge, deploy, publish or perform destructive actions without existing authorization. Do not claim user verification.',
    `You have until ${run.deadline}; stop and report a blocker when that limit is reached, including stopping outstanding workers.`,
    'When you need the user, or implementation and checks are ready for review, end your reply with the following fenced JSON. Keep the entire final reply under 3500 characters. The summary contains your question or the result, actual checks, changed artifacts, caveats and unresolved questions (under 3000 characters).',
    '```intent-report',
    JSON.stringify({
      runId: run.id,
      revision: run.intentRevision,
      state: 'waiting',
      summary: 'Your question and recommended answer',
    }),
    '```',
    'Optional JSON fields checks, artifacts, caveats and followUps are arrays of strings containing actual results only. Do not include secrets, transcript excerpts or tool inputs.',
    'Use state "review" only when ready for human review and no workers remain running. Regular progress and waiting for workers do not use this block. Reports are not acceptance.',
    run.message,
  ].join('\n');
}
