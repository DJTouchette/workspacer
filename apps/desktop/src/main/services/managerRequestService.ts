import { createHash, randomUUID } from 'crypto';
import path from 'path';
import type { DispatchTask } from '../shared/dispatchHistory';
import type { WorkflowPin } from '../shared/fleetWorkflow';
import {
  REQUEST_CAPTURE_UNAVAILABLE,
  taskDependencyState,
  type ManagerRequest,
  type ManagerRequestOperation,
  type RequestCapture,
  type RequestContext,
  type RequestIntent,
} from '../shared/managerRequests';
import { DispatchHistoryStore } from './dispatchHistoryStore';

type Owner = {
  sessionId: string;
  cwd: string;
  label?: string;
  isWakeTarget?: boolean;
  status: string;
  hub?: string;
};
const digest = (text: string) => createHash('sha256').update(text).digest('hex');
const canonical = (v: unknown): string =>
  JSON.stringify(v, function (_key, value) {
    return value && typeof value === 'object' && !Array.isArray(value)
      ? Object.fromEntries(
          Object.keys(value)
            .sort()
            .map((k) => [k, value[k]]),
        )
      : value;
  });
export function requestContext(request: ManagerRequest, content = false): RequestContext {
  const { userContent, ...host } = structuredClone(request);
  return {
    host,
    ...(content && userContent !== undefined
      ? { userContent: { text: userContent, trust: 'user' as const } }
      : {}),
  };
}
function text(v: unknown, limit: number): asserts v is string {
  if (typeof v !== 'string' || !v.trim() || v.length > limit)
    throw new Error(`Expected nonempty text, at most ${limit} characters`);
}
function ownedTask(
  tasks: DispatchTask[],
  owner: string,
  cwd: string | undefined,
  id: string | undefined,
): DispatchTask {
  const task = tasks.find((t) => t.taskId === id);
  if (!task || task.ownerSessionId !== owner || task.projectCwd !== cwd)
    throw new Error('Task unavailable for this manager/project');
  return task;
}
function audit(task: DispatchTask, action: 'request' | 'outcome', reason: string): void {
  task.audit = [
    ...(task.audit ?? []),
    { id: randomUUID(), actor: 'manager', action, reason, createdAt: new Date().toISOString() },
  ];
}
function resolvedTasks(request: ManagerRequest, tasks: DispatchTask[]): DispatchTask[] {
  return (request.intents ?? []).flatMap((intent) => {
    const task = tasks.find(
      (t) => t.taskId === intent.taskId && t.ownerSessionId === request.ownerSessionId,
    );
    return task ? [task] : [];
  });
}

/** Desktop-only authority. No provider calls, transcript matching, or authority grants. */
export class ManagerRequestService {
  constructor(
    private store: DispatchHistoryStore,
    private owner: (id: string) => Owner | undefined,
    private pin: (cwd: string) => WorkflowPin,
  ) {}

  private manager(id: string): Owner {
    const owner = this.owner(id);
    if (!owner?.isWakeTarget || owner.hub || owner.status === 'ended')
      throw new Error(REQUEST_CAPTURE_UNAVAILABLE);
    return owner;
  }
  prepare(ownerId: string, content: string, bootstrap = false): RequestCapture {
    const owner = this.manager(ownerId);
    text(content, 64 * 1024);
    const request: ManagerRequest = {
      requestId: randomUUID(),
      ownerSessionId: ownerId,
      sourceSessionId: ownerId,
      sourceCwd: owner.cwd,
      createdAt: new Date().toISOString(),
      digest: digest(content),
      delivery: 'pending',
      attempts: [],
      userContent: content,
      revision: 0,
      ...(bootstrap ? { bootstrap: true } : {}),
    };
    this.store.requestTransaction((requests) => {
      this.manager(ownerId);
      requests.push(request);
    });
    return { available: true, requestId: request.requestId, delivery: request.delivery };
  }
  /** Called only by local renderer delivery, never by an MCP argument. */
  beginDelivery(
    ownerId: string,
    requestId: string,
  ): { deliveryId: string; text: string; bootstrap?: boolean } | undefined {
    this.manager(ownerId);
    return this.store.requestTransaction((requests) => {
      const r = requests.find((r) => r.requestId === requestId && r.ownerSessionId === ownerId);
      if (!r) throw new Error('Request unavailable');
      // An interrupted host transaction is ambiguous after restart. Never replay.
      if (
        r.attempts.some(
          (a) => a.status === 'pending' || a.status === 'unknown' || a.status === 'accepted',
        )
      )
        return;
      if (r.userContent === undefined || r.intents) return;
      if (r.attempts.length >= 8) throw new Error('Request delivery retry limit reached');
      const deliveryId = randomUUID();
      r.attempts.push({ deliveryId, status: 'pending', at: new Date().toISOString() });
      r.delivery = 'unknown'; // persisted BEFORE IO; a crash never makes this replayable
      r.revision++;
      return { deliveryId, text: r.userContent, bootstrap: r.bootstrap };
    });
  }
  finishDelivery(
    requestId: string,
    deliveryId: string,
    status: 'pending' | 'accepted' | 'rejected' | 'unknown',
  ): void {
    this.store.requestTransaction((requests, tasks) => {
      const r = requests.find((r) => r.requestId === requestId);
      const attempt = r?.attempts.find((a) => a.deliveryId === deliveryId);
      if (!r || !attempt) throw new Error('Unknown request delivery attempt');
      if (attempt.status === 'accepted' || attempt.status === 'rejected') return;
      attempt.status = status;
      r.delivery = status;
      r.revision++;
      for (const task of tasks)
        for (const source of task.sources ?? [])
          if (source.requestId === requestId) source.delivery = status;
    });
  }
  request(owner: string, id: string): ManagerRequest {
    this.manager(owner);
    const request = this.store.listRequests(owner).find((r) => r.requestId === id);
    if (!request) throw new Error('Request unavailable for this manager');
    return request;
  }
  handle(input: ManagerRequestOperation, caller: string): Record<string, unknown> {
    try {
      this.manager(caller);
      if (JSON.stringify(input).length > 100 * 1024) throw new Error('Request too large');
      if (input.op === 'requestInbox')
        return {
          ok: true,
          available: true,
          requests: this.store.listRequests(caller).map((r) => requestContext(r)),
          tasks: this.store
            .list()
            .filter((t) => t.ownerSessionId === caller)
            .map((t) => ({
              taskId: t.taskId,
              title: t.title,
              cwd: t.projectCwd,
              revision: t.revision ?? 0,
              state: taskDependencyState(t, this.store.list()),
              sources: t.sources,
              dependsOn: t.dependsOn,
            })),
          instructions:
            'Host metadata identifies inbox requests, not consumed provider turns. Fetch exact user content before resolving. Unknown delivery may be resolved here; never replay it. Wakes are events, not new requests. Ready tasks still require normal authorized dispatch.',
        };
      if (input.op === 'requestContent')
        return { ok: true, ...requestContext(this.request(caller, input.requestId!), true) };
      if (input.op === 'acceptTaskOutcome') return this.acceptOutcome(input, caller);
      if (input.op !== 'resolveRequest') throw new Error('Unknown request operation');
      const intents = this.validateIntents(input.intents);
      const fingerprint = digest(
        canonical(
          intents
            .map((i) => ({
              ...i,
              dependsOn: i.dependsOn?.slice().sort(),
              dependsOnKeys: i.dependsOnKeys?.slice().sort(),
            }))
            .sort((a, b) => a.key.localeCompare(b.key)),
        ),
      );
      const previous = this.request(caller, input.requestId!);
      if (previous.resolutionDigest === fingerprint)
        return {
          ok: true,
          request: requestContext(previous),
          tasks: resolvedTasks(previous, this.store.list()),
        };
      if (previous.intents || previous.revision !== input.expectedRevision)
        return { ok: false, code: 'conflict', request: requestContext(previous) };
      const pins = new Map<string, WorkflowPin>();
      // Resolve policy before taking the task lock. Pins are immutable snapshots.
      for (const i of intents)
        if (i.kind === 'create' || i.kind === 'followUp') pins.set(i.key, this.pin(i.cwd!));
      return this.store.requestTransaction((requests, tasks) => {
        const owner = this.manager(caller);
        const r = requests.find(
          (r) => r.requestId === input.requestId && r.ownerSessionId === caller,
        );
        if (!r) throw new Error('Request unavailable for this manager');
        if (r.resolutionDigest === fingerprint)
          return {
            ok: true,
            request: requestContext(r),
            tasks: resolvedTasks(r, tasks),
          };
        if (r.intents || r.revision !== input.expectedRevision)
          return { ok: false, code: 'conflict', request: requestContext(r) };
        if (!['accepted', 'unknown'].includes(r.delivery) || !r.attempts.length)
          throw new Error('Request has not been submitted or was explicitly rejected');
        // Check every CAS before applying ANY intent. A conflict must not commit
        // the first task in an otherwise rejected multi-intent resolution.
        for (const i of intents)
          if (i.kind === 'update') {
            const task = ownedTask(tasks, caller, i.cwd, i.taskId);
            if ((task.revision ?? 0) !== i.expectedTaskRevision)
              return { ok: false, code: 'conflict', request: requestContext(r), task };
          }
        const resolved: RequestIntent[] = [];
        for (const i of intents) {
          if (i.kind === 'none' || i.kind === 'question') {
            resolved.push(i);
            continue;
          }
          let task: DispatchTask;
          if (i.kind === 'update') {
            task = ownedTask(tasks, caller, i.cwd, i.taskId);
            if (task.dispatchReservation) throw new Error('Task dispatch is in progress');
            if (i.title) task.title = i.title;
            if (i.cancel !== undefined) task.cancelled = i.cancel;
            delete task.acceptedOutcome;
          } else {
            task = {
              taskId: randomUUID(),
              ownerSessionId: caller,
              ownerLabel: owner.label ?? caller,
              projectCwd: i.cwd!,
              title: i.title!,
              createdAt: new Date().toISOString(),
              attempts: [],
              workflow: structuredClone(pins.get(i.key)!),
            };
            tasks.push(task);
          }
          if (i.dependsOn || i.dependsOnKeys) {
            const dependencies = [
              ...(i.dependsOn ?? []),
              ...(i.dependsOnKeys ?? []).map((key) => {
                const earlier = resolved.find((intent) => intent.key === key);
                if (!earlier?.taskId)
                  throw new Error(
                    'Dependency intent key must name earlier work in this resolution',
                  );
                return earlier.taskId;
              }),
            ];
            if (dependencies.length > 8 || new Set(dependencies).size !== dependencies.length)
              throw new Error('Use at most eight distinct task dependencies');
            for (const id of dependencies) ownedTask(tasks, caller, task.projectCwd, id);
            task.dependsOn = dependencies;
          }
          const visiting = new Set<string>();
          const check = (id: string): void => {
            if (visiting.has(id))
              throw new Error('Task dependencies cannot contain cycles or self references');
            visiting.add(id);
            for (const dep of tasks.find((t) => t.taskId === id)?.dependsOn ?? []) check(dep);
            visiting.delete(id);
          };
          check(task.taskId);
          task.sources = [
            ...(task.sources ?? []),
            {
              requestId: r.requestId,
              intentKey: i.key,
              delivery: r.delivery,
              label: `Request ${r.createdAt.slice(0, 16).replace('T', ' ')}`,
            },
          ];
          audit(task, 'request', i.reason);
          resolved.push({ ...i, taskId: task.taskId });
        }
        r.intents = resolved;
        r.resolutionDigest = fingerprint;
        r.revision++;
        delete r.userContent;
        return {
          ok: true,
          request: requestContext(r),
          tasks: resolvedTasks(r, tasks),
        };
      });
    } catch (e) {
      return { ok: false, code: 'unavailable', error: e instanceof Error ? e.message : String(e) };
    }
  }
  private validateIntents(value: unknown): RequestIntent[] {
    if (!Array.isArray(value) || !value.length || value.length > 8)
      throw new Error('Resolve 1–8 stable intent keys together');
    const keys = new Set<string>();
    const tasks = new Set<string>();
    for (const i of value) {
      if (
        !i ||
        typeof i !== 'object' ||
        Object.keys(i).some(
          (k) =>
            ![
              'key',
              'kind',
              'reason',
              'provenance',
              'cwd',
              'title',
              'taskId',
              'expectedTaskRevision',
              'dependsOn',
              'dependsOnKeys',
              'cancel',
            ].includes(k),
        )
      )
        throw new Error('Invalid intent fields');
      text(i.key, 64);
      text(i.reason, 2000);
      if (keys.has(i.key)) throw new Error('Duplicate intent key');
      keys.add(i.key);
      if (!['create', 'followUp', 'update', 'none', 'question'].includes(i.kind))
        throw new Error('Unknown intent kind');
      if (['none', 'question'].includes(i.kind)) {
        if (Object.keys(i).some((k) => !['key', 'kind', 'reason'].includes(k)))
          throw new Error('Conversational resolutions cannot mutate tasks');
        continue;
      }
      if (typeof i.cwd !== 'string' || !path.isAbsolute(i.cwd))
        throw new Error('Task requires an absolute project cwd');
      if (i.title !== undefined) text(i.title, 300);
      if (i.kind === 'update') {
        text(i.taskId, 100);
        if (
          !Number.isSafeInteger(i.expectedTaskRevision) ||
          i.expectedTaskRevision < 0 ||
          tasks.has(i.taskId)
        )
          throw new Error('Update needs unique task and current revision');
        tasks.add(i.taskId);
        if (i.cancel !== undefined && typeof i.cancel !== 'boolean')
          throw new Error('cancel must be boolean');
      } else {
        text(i.title, 300);
        if (
          !['explicit', 'inferred'].includes(i.provenance) ||
          i.taskId ||
          i.cancel !== undefined ||
          i.expectedTaskRevision !== undefined
        )
          throw new Error('New task requires provenance and a host-minted task id');
      }
      if (i.kind === 'followUp' && !i.dependsOn?.length && !i.dependsOnKeys?.length)
        throw new Error('Followup needs concrete task dependencies');
      if (
        i.dependsOn !== undefined &&
        (!Array.isArray(i.dependsOn) ||
          i.dependsOn.length > 8 ||
          new Set(i.dependsOn).size !== i.dependsOn.length ||
          i.dependsOn.some((id: unknown) => typeof id !== 'string'))
      )
        throw new Error('Use at most eight distinct dependency task IDs');
      if (
        i.dependsOnKeys !== undefined &&
        (!Array.isArray(i.dependsOnKeys) ||
          i.dependsOnKeys.length > 8 ||
          i.dependsOnKeys.some((key: unknown) => typeof key !== 'string' || !key))
      )
        throw new Error('Dependency intent keys must be nonempty strings');
    }
    return structuredClone(value);
  }
  private acceptOutcome(input: ManagerRequestOperation, caller: string): Record<string, unknown> {
    text(input.reason, 2000);
    return this.store.requestTransaction((_requests, tasks) => {
      this.manager(caller);
      const task = ownedTask(tasks, caller, input.cwd, input.taskId);
      if ((task.revision ?? 0) !== input.expectedTaskRevision)
        return { ok: false, code: 'conflict', task };
      if (
        !task.workflow ||
        task.dispatchReservation ||
        taskDependencyState(task, tasks) !== 'ready' ||
        task.workflow.steps.some((s) => !['completed', 'skipped'].includes(s.state))
      )
        throw new Error('Unfinished, failed, blocked or waived policy cannot be accepted');
      const evidence = task.workflow.steps
        .filter((s) => s.state === 'completed')
        .map((s) => {
          if (
            s.outcome === undefined ||
            !s.dispatchId ||
            !task.attempts.some(
              (a) =>
                a.dispatchId === s.dispatchId &&
                a.sessionId === s.sessionId &&
                a.resultContract === 'valid',
            )
          )
            throw new Error('Concrete recorded outcome evidence required');
          return { stepId: s.id, dispatchId: s.dispatchId, outcome: structuredClone(s.outcome) };
        });
      if (!evidence.length) throw new Error('Skipped policy is not accepted evidence');
      task.acceptedOutcome = {
        acceptedBy: caller,
        acceptedAt: new Date().toISOString(),
        reason: input.reason!,
        evidence,
      };
      audit(task, 'outcome', input.reason!);
      return {
        ok: true,
        task,
        readyTasks: tasks
          .filter(
            (t) => t.dependsOn?.includes(task.taskId) && taskDependencyState(t, tasks) === 'ready',
          )
          .map((t) => ({ taskId: t.taskId, title: t.title })),
        instructions:
          'Accepted evidence recorded. Ready tasks require a manager decision and existing authorized workflow; nothing was dispatched or published.',
      };
    });
  }
}

// The workflow runtime supplies lazy closures after module initialization. No
// CommonJS require of source TS, and no eager session-store/library cycle.
let singleton: ManagerRequestService | undefined;
let factory: (() => ManagerRequestService) | undefined;
export function configureManagerRequests(create: () => ManagerRequestService): void {
  factory = create;
}
export function managerRequests(): ManagerRequestService {
  if (!factory) throw new Error(REQUEST_CAPTURE_UNAVAILABLE);
  singleton ??= factory();
  return singleton;
}
