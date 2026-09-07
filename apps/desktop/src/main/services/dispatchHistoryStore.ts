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
        this.pruneOne(this.tasks!);
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
    if (
      stage !== undefined &&
      !TASK_STAGES.includes(stage) &&
      (input.taskId || input.workflowStepId || input.afterDispatchId)
    )
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
    if (
      task?.workflow &&
      (!input.workflowStepId || !task.workflow.steps.some((s) => s.id === input.workflowStepId))
    )
      throw new Error('Workflow-bound task requires an explicit workflowStepId');
    if (input.workflowStepId && !task?.workflow)
      throw new Error('Workflow step requires a pinned task');
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
      workflowStepId: input.workflowStepId,
      dispatchId: randomUUID(),
      sessionId: input.sessionId,
      kind: source ? 'retry' : 'fresh',
      ...(source ? { retryOfDispatchId: source.attempt.dispatchId } : {}),
      stage: input.stage && TASK_STAGES.includes(input.stage) ? input.stage : source?.attempt.stage,
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
    const step = task.workflow?.steps.find((s) => s.id === input.workflowStepId);
    if (step) {
      step.state = 'dispatched';
      step.sessionId = input.sessionId;
      step.dispatchId = attempt.dispatchId;
      delete step.reason;
    }
    this.attemptsBySessionID.set(attempt.sessionId, { task, attempt });
    this.flush();
    return { taskId: task.taskId, dispatchId: attempt.dispatchId };
  }
  private find(sessionId: string): { task: DispatchTask; attempt: DispatchAttempt } | undefined {
    this.load();
    return this.attemptsBySessionID.get(sessionId);
  }
  task(taskId: string): DispatchTask | undefined {
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
      (source.task.workflow && !input.workflowStepId) ||
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
    const run = this.find(s.sessionId)?.task.workflow?.steps.find(
      (r) => r.sessionId === s.sessionId,
    );
    if (run && a.resultContract === 'absent') {
      run.state =
        a.lifecycle === 'needs-decision'
          ? 'blocked'
          : s.status === 'ended'
            ? 'failed'
            : 'dispatched';
      run.reason =
        a.lifecycle === 'needs-decision'
          ? 'Worker needs a decision'
          : s.status === 'ended'
            ? 'Worker ended without a validated result'
            : undefined;
    }
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
    outcome?: unknown,
  ): void {
    const a = this.find(sessionId)?.attempt;
    if (!a) return;
    a.resultContract = resultContract;
    const step = this.find(sessionId)?.task.workflow?.steps.find((s) => s.sessionId === sessionId);
    if (step) {
      step.state =
        resultContract === 'valid'
          ? 'completed'
          : resultContract === 'escalated'
            ? 'blocked'
            : 'failed';
      step.reason =
        resultContract === 'valid' ? undefined : `Worker result contract: ${resultContract}`;
      step.outcome = outcome;
    }
    if (evidenceId) a.reviewEvidenceId = evidenceId;
    this.flush();
  }
  startWorkflow(
    owner: Owner,
    projectCwd: string,
    title: string,
    workflow: import('../shared/fleetWorkflow').WorkflowPin,
  ): DispatchTask {
    if (!owner.isWakeTarget || owner.status === 'ended' || owner.hub)
      throw new Error('Workflow requires a live local manager');
    const task: DispatchTask = {
      taskId: randomUUID(),
      ownerSessionId: owner.sessionId,
      ownerLabel: owner.label ?? owner.sessionId,
      projectCwd,
      title: title.slice(0, 300),
      createdAt: new Date().toISOString(),
      attempts: [],
      workflow: structuredClone(workflow),
    };
    const previous = [...this.load()];
    this.load().push(task);
    this.index();
    try {
      this.flush();
    } catch (error) {
      this.tasks = previous;
      this.index();
      throw error;
    }
    return structuredClone(task);
  }
  workflowDecision(taskId: string, stepId: string, run: boolean, reason: string): void {
    const task = this.task(taskId);
    const index = task?.workflow?.steps.findIndex((s) => s.id === stepId) ?? -1;
    if (!task?.workflow || index < 0) throw new Error('Workflow step unavailable');
    const step = task.workflow.steps[index];
    const definition = task.workflow.definition.steps[index];
    if (
      step.state !== 'planned' ||
      step.decision !== undefined ||
      task.workflow.steps.slice(0, index).some((s) => !['completed', 'skipped'].includes(s.state))
    )
      throw new Error('Only the next planned conditional step accepts a decision');
    if (definition.when !== 'material_risk' && !definition.repairOf)
      throw new Error('Required step cannot be skipped');
    if (!reason.trim() || reason.length > 2000)
      throw new Error('A decision requires a reason (at most 2000 characters)');
    step.decision = run;
    step.reason = reason;
    if (!run) step.state = 'skipped';
    this.flush();
  }
  adoptWorkflowTasks(oldOwner: string, newOwner: string): void {
    for (const task of this.load())
      if (task.workflow && task.ownerSessionId === oldOwner) {
        task.ownerSessionId = newOwner;
        task.ownerLabel = newOwner;
      }
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
  private pruneOne(tasks: DispatchTask[]): void {
    const index = tasks.findIndex(
      (t) =>
        !t.workflow || t.workflow.steps.every((s) => ['completed', 'skipped'].includes(s.state)),
    );
    if (index < 0)
      throw new Error('Workflow history capacity reached; active tasks cannot be evicted');
    tasks.splice(index, 1);
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
      this.pruneOne(tasks);
    while (
      tasks.length &&
      Buffer.byteLength(JSON.stringify({ version: 1, tasks })) > this.limits.bytes
    )
      this.pruneOne(tasks);
    this.index();
    fs.mkdirSync(path.dirname(this.filename()), { recursive: true, mode: 0o700 });
    atomicWriteFileSync(this.filename(), JSON.stringify({ version: 1, tasks }), { mode: 0o600 });
  }
}
export const dispatchHistoryStore = new DispatchHistoryStore(() =>
  path.join(getConfigDir(), 'dispatch-history.json'),
);
