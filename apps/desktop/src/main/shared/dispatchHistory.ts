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
  workflowStepId?: string;
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
    directoryIdentity?: { dev: number; ino: number };
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
  sources?: import('./managerRequests').TaskSource[];
  dependsOn?: string[];
  cancelled?: boolean;
  acceptedOutcome?: import('./managerRequests').AcceptedTaskOutcome;
  /** Legacy rows read as revision zero. Every persisted mutation advances it. */
  revision?: number;
  /** Durable admission fence across asynchronous worktree allocation. */
  dispatchReservation?: { stepId: string; token: string; createdAt: string };
  links?: TaskLinks;
  audit?: TaskAudit[];
  workflow?: import('./fleetWorkflow').WorkflowPin;
  taskId: string;
  ownerSessionId: string;
  ownerLabel: string;
  projectCwd: string;
  title: string;
  createdAt: string;
  attempts: DispatchAttempt[];
}
export type DispatchHistoryResponse =
  | { available: true; currentOwnerSessionId?: string; tasks: DispatchTask[]; requests?: Array<{ ownerSessionId: string; requestId: string; delivery: string; resolved: boolean }> }
  | { available: false; reason: string };

export type TaskLinks = {
  pullRequest?: { number?: string; url?: string };
  tickets?: Array<{ id: string; url?: string }>;
  references?: Array<{ label: string; url: string }>;
};
export type TaskAudit = {
  id: string;
  /** Who made the edit. Waivers are always host-user; managers may only touch references. */
  actor: 'host-user' | 'manager';
  action: 'waive' | 'links' | 'request' | 'outcome';
  stepId?: string;
  reason: string;
  createdAt: string;
};
export type TaskEditRequest = { taskId: string; expectedTaskRevision: number } & (
  { action: 'waive'; stepId: string; reason?: string } | { action: 'links'; links: TaskLinks }
);
export type TaskEditResponse =
  | { ok: true; task: DispatchTask }
  | {
      ok: false;
      code: 'conflict' | 'unavailable' | 'ineligible';
      error: string;
      task?: DispatchTask;
    };
export type TaskOpenRequest = { taskId: string } & (
  | { kind: 'worktree'; dispatchId: string }
  | { kind: 'url'; reference: 'pullRequest' | 'tickets' | 'references'; index?: number }
);
export const TASK_INSPECTOR_UNAVAILABLE =
  'Task Inspector is available in the local desktop app only. Remote connections and older desktop versions cannot edit these tasks.';
export const terminalWorkflowStep = (state: string): boolean =>
  ['completed', 'skipped', 'waived'].includes(state);
export function taskIsActive(task: DispatchTask): boolean {
  if (task.cancelled) return false;
  return task.workflow
    ? task.workflow.steps.some((s) => !terminalWorkflowStep(s.state))
    : task.attempts.some((a) => a.lifecycle !== 'ended');
}
/** UI guidance only. The host independently verifies the exact attached session. */
export function taskSkipDisabledReason(task: DispatchTask, stepId: string): string | undefined {
  const run = task.workflow?.steps.find((s) => s.id === stepId);
  if (!run) return 'Step unavailable';
  if (task.dispatchReservation?.stepId === stepId)
    return 'A worker is being dispatched for this step';
  if (terminalWorkflowStep(run.state)) return 'This step has already finished';
  if (run.state === 'dispatched') return 'A worker has been dispatched for this step';
  if (!['planned', 'failed', 'blocked'].includes(run.state)) return 'Step state is unknown';
  if (!run.sessionId && !run.dispatchId && run.state === 'planned') return;
  const attempt = task.attempts.find(
    (a) =>
      a.dispatchId === run.dispatchId &&
      a.sessionId === run.sessionId &&
      a.workflowStepId === run.id,
  );
  if (!attempt || attempt.stale) return 'Worker status is unknown. Wait for a fresh status update.';
  if (attempt.live || attempt.lifecycle !== 'ended') return 'The attached worker is still live';
  const index = task.workflow!.steps.indexOf(run);
  if (task.workflow!.steps.slice(0, index).some((s) => !terminalWorkflowStep(s.state)))
    return 'An earlier step is unfinished';
}
export function validateTaskUrl(value: unknown): string {
  if (typeof value !== 'string' || value.length > 2048 || value !== value.trim())
    throw new Error('Enter an http(s) URL of at most 2048 characters');
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error('Enter a valid http(s) URL');
  }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password)
    throw new Error('Only http(s) URLs without credentials are supported');
  return url.href;
}
export function validateTaskLinks(value: unknown): TaskLinks {
  const object = (v: unknown, keys: string[]): Record<string, unknown> => {
    if (
      !v ||
      typeof v !== 'object' ||
      Array.isArray(v) ||
      Object.keys(v).some((k) => !keys.includes(k))
    )
      throw new Error('Invalid task reference fields');
    return v as Record<string, unknown>;
  };
  const label = (v: unknown): string => {
    if (typeof v !== 'string' || !v.trim() || v.length > 200)
      throw new Error('Reference labels must be 1–200 characters');
    return v.trim();
  };
  const input = object(value, ['pullRequest', 'tickets', 'references']);
  const links: TaskLinks = {};
  const urls = new Set<string>();
  const url = (v: unknown) => {
    const u = validateTaskUrl(v);
    if (urls.has(u)) throw new Error('Duplicate reference URL');
    urls.add(u);
    return u;
  };
  if (input.pullRequest !== undefined) {
    const pr = object(input.pullRequest, ['number', 'url']);
    if (pr.number === undefined && pr.url === undefined)
      throw new Error('Enter a PR number or URL');
    if (
      pr.number !== undefined &&
      (typeof pr.number !== 'string' || !/^[1-9][0-9]{0,9}$/.test(pr.number))
    )
      throw new Error('PR number must be a positive number of at most 10 digits');
    links.pullRequest = {
      ...(pr.number !== undefined ? { number: pr.number as string } : {}),
      ...(pr.url !== undefined ? { url: url(pr.url) } : {}),
    };
  }
  for (const key of ['tickets', 'references'] as const) {
    if (input[key] === undefined) continue;
    if (!Array.isArray(input[key]) || input[key].length > 20)
      throw new Error('Use at most 20 tickets or references');
    const labels = new Set<string>();
    const rows = input[key].map((v: unknown) => {
      const r = object(v, key === 'tickets' ? ['id', 'url'] : ['label', 'url']);
      const name = label(r[key === 'tickets' ? 'id' : 'label']);
      if (labels.has(name.toLowerCase())) throw new Error('Duplicate reference label');
      labels.add(name.toLowerCase());
      if (key === 'references' && r.url === undefined) throw new Error('Reference URL required');
      return {
        [key === 'tickets' ? 'id' : 'label']: name,
        ...(r.url !== undefined ? { url: url(r.url) } : {}),
      };
    });
    if (key === 'tickets') links.tickets = rows as TaskLinks['tickets'];
    else links.references = rows as TaskLinks['references'];
  }
  return links;
}

/** Additive reference mutation. Managers upsert/remove exact entries, never replace the set. */
export type TaskReferenceUpsert =
  | { kind: 'pullRequest'; number?: string; url?: string }
  | { kind: 'ticket'; id: string; url?: string }
  | { kind: 'reference'; label: string; url: string };
export type TaskReferenceRemove =
  { kind: 'pullRequest' } | { kind: 'ticket'; id: string } | { kind: 'reference'; label: string };
export const TASK_REFERENCE_KINDS = ['pullRequest', 'ticket', 'reference'] as const;
const same = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase();
function referenceKind(value: unknown, removing = false): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Each reference must be an object');
  const row = value as Record<string, unknown>;
  if (typeof row.kind !== 'string' || !TASK_REFERENCE_KINDS.includes(row.kind as 'ticket'))
    throw new Error('Reference kind must be pullRequest, ticket or reference');
  const allowed =
    row.kind === 'pullRequest'
      ? removing
        ? []
        : ['number', 'url']
      : row.kind === 'ticket'
        ? removing
          ? ['id']
          : ['id', 'url']
        : removing
          ? ['label']
          : ['label', 'url'];
  for (const key of Object.keys(row))
    if (key !== 'kind' && !allowed.includes(key))
      throw new Error(`Unknown reference field: ${key}`);
  return row;
}
/**
 * Apply additive upserts/removals to a task's existing references and revalidate the
 * whole result. Entries the caller did not name are preserved exactly, so a manager
 * edit never silently discards what the host user entered. All URL scheme, credential,
 * length, count and duplicate rules come from validateTaskLinks — there is no second
 * validator to drift.
 */
export function applyTaskReferences(
  current: TaskLinks | undefined,
  upsert: unknown,
  remove: unknown,
): TaskLinks {
  const list = (v: unknown, what: string): unknown[] => {
    if (v === undefined) return [];
    if (!Array.isArray(v) || v.length > 20) throw new Error(`Use at most 20 ${what} entries`);
    return v;
  };
  const upserts = list(upsert, 'upsert');
  const removals = list(remove, 'remove');
  if (!upserts.length && !removals.length)
    throw new Error('Supply at least one reference to upsert or remove');
  // Start from the validated stored value so a legacy or hand-edited row cannot
  // smuggle unvalidated data through an unrelated additive edit.
  const links: TaskLinks = structuredClone(validateTaskLinks(current ?? {}));
  for (const raw of removals) {
    const row = referenceKind(raw, true);
    if (row.kind === 'pullRequest') delete links.pullRequest;
    else if (row.kind === 'ticket') {
      if (typeof row.id !== 'string') throw new Error('Removing a ticket requires its id');
      links.tickets = (links.tickets ?? []).filter((t) => !same(t.id, row.id as string));
      if (!links.tickets.length) delete links.tickets;
    } else {
      if (typeof row.label !== 'string') throw new Error('Removing a reference requires its label');
      links.references = (links.references ?? []).filter(
        (r) => !same(r.label, row.label as string),
      );
      if (!links.references.length) delete links.references;
    }
  }
  for (const raw of upserts) {
    const row = referenceKind(raw);
    if (row.kind === 'pullRequest') {
      if (row.number === undefined && row.url === undefined)
        throw new Error('A pull request reference needs a number or a URL');
      links.pullRequest = {
        ...links.pullRequest,
        ...(row.number !== undefined ? { number: row.number as string } : {}),
        ...(row.url !== undefined ? { url: row.url as string } : {}),
      };
    } else if (row.kind === 'ticket') {
      if (typeof row.id !== 'string' || !row.id.trim())
        throw new Error('A ticket reference needs an id');
      const tickets = [...(links.tickets ?? [])];
      const at = tickets.findIndex((t) => same(t.id, row.id as string));
      const next = {
        id: row.id.trim(),
        ...(row.url !== undefined
          ? { url: row.url as string }
          : at >= 0 && tickets[at].url
            ? { url: tickets[at].url }
            : {}),
      };
      if (at >= 0) tickets[at] = next;
      else tickets.push(next);
      links.tickets = tickets;
    } else {
      if (typeof row.label !== 'string' || !row.label.trim())
        throw new Error('A named reference needs a label');
      if (row.url === undefined) throw new Error('A named reference needs a URL');
      const references = [...(links.references ?? [])];
      const at = references.findIndex((r) => same(r.label, row.label as string));
      const next = { label: row.label.trim(), url: row.url as string };
      if (at >= 0) references[at] = next;
      else references.push(next);
      links.references = references;
    }
  }
  return validateTaskLinks(links);
}
