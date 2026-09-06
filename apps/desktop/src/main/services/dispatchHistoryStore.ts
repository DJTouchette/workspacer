import { randomUUID } from 'crypto';
import fs from 'fs';
import path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { getConfigDir } from './configService';
import {
  TASK_STAGES,
  type DispatchAttempt,
  type DispatchLink,
  type DispatchTask,
} from '../shared/dispatchHistory';
import type { ClaudeSessionState } from './claudeSessionStore';

export const DISPATCH_LIMITS = { tasks: 200, attempts: 1000, bytes: 2 * 1024 * 1024 };
type Owner = {
  sessionId: string;
  isWakeTarget?: boolean;
  status: string;
  hub?: string;
  label?: string;
};
type Admission = DispatchLink & {
  owner?: Owner | null;
  projectCwd: string;
  retrySourceSessionId?: string;
};
const finite = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n) && n >= 0;
/** No transcripts, parent reconstruction, schema parsing, or filesystem grants. */
export class DispatchHistoryStore {
  private tasks?: DispatchTask[];
  // Session updates are the hot path. Keep these small indexes alongside the
  // bounded in-memory history instead of re-scanning every task on each update.
  private tasksByID = new Map<string, DispatchTask>();
  private attemptsBySessionID = new Map<string, { task: DispatchTask; attempt: DispatchAttempt }>();
  private timer?: ReturnType<typeof setTimeout>;
  constructor(
    private filename: () => string,
    private limits = DISPATCH_LIMITS,
  ) {}
  private index(): void {
    this.tasksByID.clear();
    this.attemptsBySessionID.clear();
    for (const task of this.tasks ?? []) {
      this.tasksByID.set(task.taskId, task);
      for (const attempt of task.attempts)
        this.attemptsBySessionID.set(attempt.sessionId, { task, attempt });
    }
  }
  private load(): DispatchTask[] {
    if (this.tasks) return this.tasks;
    this.tasks = [];
    try {
      if (fs.statSync(this.filename()).size > this.limits.bytes) return this.tasks;
      const state = JSON.parse(fs.readFileSync(this.filename(), 'utf8'));
      if (state.version !== 1 || !Array.isArray(state.tasks)) return this.tasks;
      // Reject corrupt storage wholesale, rather than partially invent identity.
      for (const t of state.tasks) {
        if (
          !t.taskId ||
          !t.ownerSessionId ||
          typeof t.projectCwd !== 'string' ||
          !Array.isArray(t.attempts)
        )
          return this.tasks;
        for (const a of t.attempts)
          if (!a.dispatchId || !a.sessionId || !a.metrics) return this.tasks;
      }
      this.tasks = state.tasks;
      while (
        this.tasks!.length > this.limits.tasks ||
        this.tasks!.reduce((n, t) => n + t.attempts.length, 0) > this.limits.attempts
      )
        this.tasks!.shift();
      for (const t of this.tasks!)
        for (const a of t.attempts) {
          a.stale = true;
          a.live = false;
        }
    } catch {
      /* absent/corrupt history is not reconstructed */
    }
    this.index();
    return this.tasks!;
  }
  private sameProject(task: DispatchTask, cwd: string): boolean {
    return (
      task.projectCwd === cwd ||
      task.attempts.some((a) => a.executionCwd === cwd && a.worktree?.allocated)
    );
  }
  validate(input: Admission): void {
    const { owner, taskId, stage, afterDispatchId } = input;
    if (stage !== undefined && !TASK_STAGES.includes(stage))
      throw new Error('Unknown dispatch stage');
    if (!owner?.isWakeTarget || owner.status === 'ended' || owner.hub) {
      // Stage describes a launch; host retry provenance is automatic. Neither
      // may turn an otherwise valid spawn into a manager-attributed one. Explicit
      // task/predecessor links are different: they request existing authority.
      if (taskId || afterDispatchId) throw new Error('Task links require a live local manager');
      return;
    }
    const task = taskId ? this.task(taskId) : undefined;
    if (
      taskId &&
      (!task ||
        task.ownerSessionId !== owner.sessionId ||
        !this.sameProject(task, input.projectCwd))
    )
      throw new Error('Task is unavailable or belongs to another manager/project');
    if (afterDispatchId && !task?.attempts.some((a) => a.dispatchId === afterDispatchId))
      throw new Error('Predecessor must belong to the same task');
    // A retry source is host provenance, not a caller-authorized task link.
    // Known foreign/departed sources deliberately degrade to a fresh unlinked
    // attempt below; they must never lend their owner or history to this launch.
  }
  accept(
    input: Admission & {
      sessionId: string;
      title?: string;
      executionCwd: string;
      requestedProvider?: string;
      provider?: string;
      requestedModel?: string;
      role?: string;
      worktree?: DispatchAttempt['worktree'];
    },
  ): { taskId: string; dispatchId: string } | undefined {
    this.validate(input);
    if (!input.owner?.isWakeTarget || input.owner.status === 'ended' || input.owner.hub) return;
    const existing = this.find(input.sessionId);
    if (existing) return { taskId: existing.task.taskId, dispatchId: existing.attempt.dispatchId };
    const source = this.retrySource(input);
    const now = new Date().toISOString();
    let task = source?.task ?? (input.taskId ? this.task(input.taskId) : undefined);
    if (!task) {
      task = {
        taskId: randomUUID(),
        ownerSessionId: input.owner.sessionId,
        ownerLabel: (input.owner.label ?? input.owner.sessionId).slice(0, 200),
        projectCwd: input.projectCwd,
        title: (input.title ?? 'Unclassified dispatch').slice(0, 300),
        createdAt: now,
        attempts: [],
      };
      this.load().push(task);
      this.tasksByID.set(task.taskId, task);
    }
    const attempt: DispatchAttempt = {
      dispatchId: randomUUID(),
      sessionId: input.sessionId,
      kind: source ? 'retry' : 'fresh',
      ...(source ? { retryOfDispatchId: source.attempt.dispatchId } : {}),
      stage: input.stage ?? source?.attempt.stage,
      afterDispatchId: source?.attempt.dispatchId ?? input.afterDispatchId,
      acceptedAt: now,
      observedAt: now,
      executionCwd: input.executionCwd,
      requestedProvider: input.requestedProvider,
      provider: input.provider,
      requestedModel: input.requestedModel,
      role: input.role,
      worktree: input.worktree,
      lifecycle: 'starting',
      stale: false,
      live: true,
      resultContract: 'absent',
      metrics: {},
    };
    task.attempts.push(attempt);
    this.attemptsBySessionID.set(attempt.sessionId, { task, attempt });
    this.flush();
    return { taskId: task.taskId, dispatchId: attempt.dispatchId };
  }
  private find(sessionId: string): { task: DispatchTask; attempt: DispatchAttempt } | undefined {
    this.load();
    return this.attemptsBySessionID.get(sessionId);
  }
  private task(taskId: string): DispatchTask | undefined {
    this.load();
    return this.tasksByID.get(taskId);
  }
  private retrySource(
    input: Admission,
  ): { task: DispatchTask; attempt: DispatchAttempt } | undefined {
    if (
      !input.retrySourceSessionId ||
      !input.owner?.isWakeTarget ||
      input.owner.status === 'ended' ||
      input.owner.hub
    )
      return undefined;
    const source = this.find(input.retrySourceSessionId);
    if (
      !source ||
      source.task.ownerSessionId !== input.owner.sessionId ||
      !this.sameProject(source.task, input.projectCwd) ||
      (input.taskId !== undefined && source.task.taskId !== input.taskId)
    )
      return undefined;
    return source;
  }
  /** Absolute cumulative snapshots replace prior values. Never add process generations. */
  observe(
    s: Pick<
      ClaudeSessionState,
      | 'sessionId'
      | 'status'
      | 'ambientState'
      | 'pendingApproval'
      | 'pendingQuestions'
      | 'usage'
      | 'statusLine'
      | 'hub'
    >,
  ): void {
    if (s.hub) return;
    const a = this.find(s.sessionId)?.attempt;
    if (!a) return;
    const now = new Date().toISOString();
    a.observedAt = now;
    a.stale = false;
    a.live = s.status !== 'ended';
    a.lifecycle =
      s.status === 'ended'
        ? 'ended'
        : s.pendingApproval || s.pendingQuestions?.length
          ? 'needs-decision'
          : s.status === 'starting'
            ? 'starting'
            : s.ambientState === 'idle'
              ? 'idle'
              : 'running';
    if (s.status === 'ended') a.endedAt ??= now;
    else delete a.endedAt;
    a.metrics.wallMs = Math.max(0, Date.parse(a.endedAt ?? now) - Date.parse(a.acceptedAt));
    const sl = s.statusLine;
    const u = s.usage;
    // Empty usage accumulators use zero sentinels; only positive values there
    // establish reporting. Explicit statusLine zeros remain real observations.
    for (const [key, value] of Object.entries({
      inputTokens: sl?.totalInputTokens ?? (u?.totalInputTokens || undefined),
      outputTokens: sl?.totalOutputTokens ?? (u?.totalOutputTokens || undefined),
      costUSD: sl?.costUSD ?? (u?.costUSD || undefined),
    })) {
      if (finite(value)) (a.metrics as Record<string, unknown>)[key] = value;
    }
    if (u?.cache) a.metrics.cache = { ...u.cache };
    if (finite(sl?.cachedInputTokens)) a.metrics.cachedInputTokens = sl.cachedInputTokens;
    const model = sl?.modelDisplay || u?.model;
    if (model) a.reportedModel = model;
    if (sl?.contextHealth)
      a.metrics.context = {
        used: sl.contextHealth.usedTokens,
        limit: sl.contextHealth.windowTokens,
        observedAt: sl.contextHealth.observedAt,
      };
    else if (sl?.receivedAt && finite(sl.contextUsedPct) && finite(sl.contextWindowSize))
      a.metrics.context = {
        used: Math.round((Math.min(100, sl.contextUsedPct) * sl.contextWindowSize) / 100),
        limit: sl.contextWindowSize,
        observedAt: sl.receivedAt,
      };
    else if (
      u?.contextTokens &&
      (a.metrics.context?.used !== u.contextTokens ||
        a.metrics.context?.limit !== (u.contextLimit ?? undefined))
    )
      a.metrics.context = {
        used: u.contextTokens,
        limit: u.contextLimit ?? undefined,
        observedAt: now,
      };
    this.schedule();
  }
  validated(
    sessionId: string,
    resultContract: DispatchAttempt['resultContract'],
    evidenceId?: string,
  ): void {
    const a = this.find(sessionId)?.attempt;
    if (!a) return;
    a.resultContract = resultContract;
    if (evidenceId) a.reviewEvidenceId = evidenceId;
    this.flush();
  }
  list(): DispatchTask[] {
    const tasks = structuredClone(this.load()).reverse();
    for (const task of tasks)
      for (const a of task.attempts) {
        if (a.live && !a.stale)
          a.metrics.wallMs = Math.max(0, Date.now() - Date.parse(a.acceptedAt));
      }
    return tasks;
  }
  private schedule(): void {
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      try {
        this.flush();
      } catch (err) {
        console.warn('[dispatch-history] persistence unavailable', err);
      }
    }, 500);
    this.timer.unref();
  }
  flush(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    const tasks = this.load();
    while (
      tasks.length > this.limits.tasks ||
      tasks.reduce((n, t) => n + t.attempts.length, 0) > this.limits.attempts
    )
      tasks.shift();
    while (
      tasks.length &&
      Buffer.byteLength(JSON.stringify({ version: 1, tasks })) > this.limits.bytes
    )
      tasks.shift();
    this.index();
    fs.mkdirSync(path.dirname(this.filename()), { recursive: true, mode: 0o700 });
    atomicWriteFileSync(this.filename(), JSON.stringify({ version: 1, tasks }), { mode: 0o600 });
  }
}
export const dispatchHistoryStore = new DispatchHistoryStore(() =>
  path.join(getConfigDir(), 'dispatch-history.json'),
);
