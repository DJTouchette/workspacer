import type { PendingApproval, PendingQuestion } from './claudeSession';

/**
 * A single thing an agent wants a human to do. The Triage Inbox and the Fleet
 * Deck are both pure projections of a list of these — derived from the live
 * ClaudeSessionSnapshot stream, never stored in the daemon (MVP). See
 * {@link ../hooks/useAttentionFeed}.
 */
export type AttentionKind =
  | 'approval' // pendingApproval — a tool wants permission
  | 'question' // pendingQuestions — AskUserQuestion picker is up
  | 'stuck' // classifier loop/idle detection (daemon items stream, future)
  | 'error' // classifier error detection (daemon items stream, future)
  | 'done' // working → idle transition: the agent finished a task
  | 'bigdiff'; // a large unreviewed change landed (future)

export type AttentionStatus = 'open' | 'resolved' | 'snoozed' | 'dismissed';

export type AttentionPayload =
  | { type: 'approval'; approval: PendingApproval }
  | { type: 'question'; questions: PendingQuestion[] }
  | { type: 'summary'; summary: string };

export interface AttentionItem {
  /** Stable identity == {@link signature}; re-arriving snapshots update, never duplicate. */
  id: string;
  agentId: string;
  agentName: string;
  sessionId: string;
  cwd?: string;
  kind: AttentionKind;
  /** Higher = more urgent. From {@link ../lib/attentionRouter.KIND_PRIORITY}. */
  priority: number;
  /** Epoch ms the item was first observed. */
  createdAt: number;
  status: AttentionStatus;
  /** Short headline, e.g. "Bash — npm test" or "Choose an option". */
  title: string;
  /** Secondary line, e.g. the question text or a tool-input preview. */
  detail?: string;
  payload: AttentionPayload;
  /** `${sessionId}:${kind}:${hash}` — idempotent dedupe key. */
  signature: string;
}
