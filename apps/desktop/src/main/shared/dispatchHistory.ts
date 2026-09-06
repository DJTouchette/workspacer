/** Local desktop dispatch projection. Absence is unknown, never a zero or success. */
export const TASK_STAGES = [
  'scout',
  'implement',
  'review',
  'fix',
  'validate',
  'land',
  'other',
] as const;
export type TaskStage = (typeof TASK_STAGES)[number];
export interface DispatchLink {
  taskId?: string;
  stage?: TaskStage;
  afterDispatchId?: string;
}
export interface DispatchMetrics {
  wallMs?: number;
  inputTokens?: number;
  outputTokens?: number;
  costUSD?: number;
  cache?: { fresh: number; write: number; read: number };
  cachedInputTokens?: number;
  context?: { used?: number; limit?: number; observedAt: string };
}
export interface DispatchAttempt extends DispatchLink {
  dispatchId: string;
  sessionId: string;
  kind: 'fresh' | 'retry';
  retryOfDispatchId?: string;
  acceptedAt: string;
  observedAt: string;
  endedAt?: string;
  executionCwd: string;
  worktree?: {
    requested: boolean;
    allocated: boolean;
    fallback: boolean;
    branch?: string;
    error?: string;
  };
  requestedProvider?: string;
  provider?: string;
  requestedModel?: string;
  reportedModel?: string;
  role?: string;
  lifecycle: 'starting' | 'running' | 'idle' | 'needs-decision' | 'ended';
  stale: boolean;
  live: boolean;
  resultContract: 'absent' | 'valid' | 'invalid' | 'escalated';
  metrics: DispatchMetrics;
  reviewEvidenceId?: string;
}
export interface DispatchTask {
  taskId: string;
  ownerSessionId: string;
  ownerLabel: string;
  projectCwd: string;
  title: string;
  createdAt: string;
  attempts: DispatchAttempt[];
}
export type DispatchHistoryResponse =
  | { available: true; currentOwnerSessionId?: string; tasks: DispatchTask[] }
  | { available: false; reason: string };
