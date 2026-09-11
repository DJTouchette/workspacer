import type { DispatchTask } from './dispatchHistory';

/** Host identities and receipts are separate from untrusted user content. */
export interface ManagerRequest {
  requestId: string;
  ownerSessionId: string;
  sourceSessionId: string;
  sourceCwd: string;
  createdAt: string;
  digest: string;
  bootstrap?: boolean;
  delivery: 'pending' | 'accepted' | 'rejected' | 'unknown';
  attempts: Array<{
    deliveryId: string;
    status: 'pending' | 'accepted' | 'rejected' | 'unknown';
    at: string;
  }>;
  /** Retained only until resolved; never used for identity or authority. */
  userContent?: string;
  revision: number;
  intents?: Array<RequestIntent & { taskId?: string }>;
  resolutionDigest?: string;
}
export type RequestIntent = {
  key: string;
  kind: 'create' | 'followUp' | 'update' | 'question' | 'none' | 'untracked';
  reason: string;
  /** New work or an undispatched task update: null omits workflow; an id chooses a policy. Omit to keep defaults/current policy. */
  workflowId?: string | null;
  provenance?: 'explicit' | 'inferred';
  cwd?: string;
  title?: string;
  taskId?: string;
  expectedTaskRevision?: number;
  dependsOn?: string[];
  /** Earlier work intent keys in this atomic resolution, converted to task IDs. */
  dependsOnKeys?: string[];
  cancel?: boolean;
  /** Explicit mapping from original submitted content; [] deliberately assigns no references. */
  references?: import('./dispatchHistory').TaskReferenceUpsert[];
  replacePullRequest?: boolean;
};
export type TaskSource = {
  requestId: string;
  intentKey: string;
  label: string;
  delivery?: ManagerRequest['delivery'];
};
export type AcceptedTaskOutcome = {
  acceptedBy: string;
  acceptedAt: string;
  reason: string;
  evidence: Array<{ stepId: string; dispatchId: string; outcome: unknown }>;
};
export type RequestCapture =
  | { available: true; requestId: string; delivery: ManagerRequest['delivery'] }
  | { available: false; reason: string };
export type RequestContext = {
  host: Omit<ManagerRequest, 'userContent'>;
  userContent?: { text: string; trust: 'user' };
};
export type ManagerRequestOperation = {
  op: 'requestInbox' | 'requestContent' | 'resolveRequest' | 'acceptTaskOutcome';
  /** Bounded unresolved inbox with original content; omits historical tasks. */
  view?: 'pending';
  requestId?: string;
  expectedRevision?: number;
  intents?: RequestIntent[];
  taskId?: string;
  cwd?: string;
  expectedTaskRevision?: number;
  reason?: string;
};
export const REQUEST_CAPTURE_UNAVAILABLE =
  'Automatic task capture is available only through the local desktop manager request inbox.';

/** Explicit acceptance is an interpretation of recorded evidence, never authority. */
export function taskOutcomeAccepted(task: DispatchTask): boolean {
  const accepted = task.acceptedOutcome;
  if (task.cancelled || !accepted || !accepted.evidence.length || !task.workflow) return false;
  if (
    task.sources?.length &&
    !task.sources.some((s) => s.delivery === 'accepted' || s.delivery === 'unknown')
  )
    return false;
  if (!task.workflow.steps.some((s) => s.state === 'completed')) return false;
  if (task.workflow.steps.some((s) => !['completed', 'skipped'].includes(s.state))) return false;
  return task.workflow.steps
    .filter((s) => s.state === 'completed')
    .every((s) => {
      const evidence = accepted.evidence.find((e) => e.stepId === s.id);
      return (
        !!evidence &&
        evidence.dispatchId === s.dispatchId &&
        JSON.stringify(evidence.outcome) === JSON.stringify(s.outcome) &&
        task.attempts.some(
          (a) =>
            a.dispatchId === s.dispatchId &&
            a.sessionId === s.sessionId &&
            a.resultContract === 'valid',
        )
      );
    });
}
export function taskDependencyState(
  task: DispatchTask,
  tasks: DispatchTask[],
  visited = new Set<string>(),
): 'blocked' | 'ready' | 'cancelled' {
  if (visited.has(task.taskId)) return 'blocked';
  const next = new Set(visited).add(task.taskId);
  if (task.cancelled) return 'cancelled';
  if (
    task.sources?.length &&
    !task.sources.some((s) => s.delivery === 'accepted' || s.delivery === 'unknown')
  )
    return 'blocked';
  return (task.dependsOn ?? []).every((id) => {
    const dependency = tasks.find((t) => t.taskId === id);
    return (
      dependency &&
      dependency.ownerSessionId === task.ownerSessionId &&
      dependency.projectCwd === task.projectCwd &&
      taskOutcomeAccepted(dependency) &&
      taskDependencyState(dependency, tasks, next) === 'ready'
    );
  })
    ? 'ready'
    : 'blocked';
}
