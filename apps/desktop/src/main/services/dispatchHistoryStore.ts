import { randomUUID } from 'crypto';
import fs from 'fs';
import path from 'path';
import { withConfigLock } from '../lib/configLock';
import {
  applyTaskReferences,
  taskSkipDisabledReason,
  validateTaskLinks,
  validateTaskUrl,
  type TaskEditRequest,
  type TaskEditResponse,
  type TaskOpenRequest,
} from '../shared/dispatchHistory';
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
  private writing = false;
  private fresh = new Map<string, string>();
  /** All writers reload under the same cross-process lock; no delayed stale flush. */
  private transaction<T>(fn: () => T): T {
    if (this.writing) return fn();
    return withConfigLock(this.filename(), () => {
      this.tasks = undefined;
      const before = structuredClone(this.load());
      const freshness = new Map(this.fresh);
      this.writing = true;
      try {
        const result = fn();
        for (const task of this.tasks!) {
          const prior = before.find((t) => t.taskId === task.taskId);
          if (JSON.stringify(prior) !== JSON.stringify(task))
            task.revision = (prior?.revision ?? 0) + 1;
        }
        if (JSON.stringify(before) !== JSON.stringify(this.tasks)) this.persist();
        return structuredClone(result);
      } catch (error) {
        this.tasks = before;
        this.fresh = freshness;
        this.index();
        throw error;
      } finally {
        this.writing = false;
      }
    });
  }
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
      if (fs.statSync(this.filename()).size > this.limits.bytes)
        throw new Error('Task history exceeds its size limit');
      const state = JSON.parse(fs.readFileSync(this.filename(), 'utf8'));
      if (state.version !== 1 || !Array.isArray(state.tasks))
        throw new Error('Task history format unavailable');
      // Reject corrupt storage wholesale, rather than partially invent identity.
      for (const t of state.tasks) {
        if (
          !t.taskId ||
          !t.ownerSessionId ||
          typeof t.projectCwd !== 'string' ||
          !Array.isArray(t.attempts)
        )
          throw new Error('Invalid task history record');
        for (const a of t.attempts)
          if (!a.dispatchId || !a.sessionId || !a.metrics) throw new Error('Invalid task attempt');
      }
      this.tasks = state.tasks;
      while (
        this.tasks!.length > this.limits.tasks ||
        this.tasks!.reduce((n, t) => n + t.attempts.length, 0) > this.limits.attempts
      )
        this.pruneOne(this.tasks!);
    } catch (error) {
      this.tasks = [];
      this.index();
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
        this.tasks = undefined;
        throw error;
      }
    }
    this.index();
    return this.tasks!;
  }
  private viewTask(task: DispatchTask): DispatchTask {
    const copy = structuredClone(task);
    for (const a of copy.attempts)
      if (this.fresh.get(a.sessionId) !== a.observedAt) {
        a.stale = true;
        a.live = false;
      }
    return copy;
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
    if (
      task?.workflow &&
      task.workflow.steps.find((s) => !['completed', 'skipped', 'waived'].includes(s.state))?.id !==
        input.workflowStepId
    )
      throw new Error('Workflow step is not next; earlier unfinished steps must finish first');
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
    if (!this.writing) return this.transaction(() => this.accept(input));
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
    // Capture directory identity at acceptance, never from a renderer request.
    if (attempt.worktree?.allocated && !attempt.worktree.fallback) {
      try {
        const canonical = fs.realpathSync(attempt.executionCwd);
        const stat = fs.statSync(canonical);
        if (canonical === path.resolve(attempt.executionCwd) && stat.isDirectory())
          attempt.worktree.directoryIdentity = { dev: stat.dev, ino: stat.ino };
      } catch {
        /* Historical path facts remain visible; opening is unavailable. */
      }
    }
    this.fresh.set(attempt.sessionId, attempt.observedAt);
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
    if (!this.writing) this.tasks = undefined;
    this.load();
    const task = this.tasksByID.get(taskId);
    return this.writing ? task : task ? this.viewTask(task) : undefined;
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
    if (!this.writing) return this.transaction(() => this.observe(s));
    if (s.hub) return;
    const a = this.find(s.sessionId)?.attempt;
    if (!a) return;
    const now = new Date().toISOString();
    a.observedAt = now;
    this.fresh.set(a.sessionId, now);
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
    if (run && run.state !== 'waived' && a.resultContract === 'absent') {
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
    this.flush();
  }
  validated(
    sessionId: string,
    resultContract: DispatchAttempt['resultContract'],
    evidenceId?: string,
    outcome?: unknown,
  ): void {
    if (!this.writing)
      return this.transaction(() => this.validated(sessionId, resultContract, evidenceId, outcome));
    const a = this.find(sessionId)?.attempt;
    if (!a) return;
    const step = this.find(sessionId)?.task.workflow?.steps.find((s) => s.sessionId === sessionId);
    if (step?.state === 'waived') return;
    a.resultContract = resultContract;
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
    if (!this.writing) {
      const id = this.transaction(
        () => this.startWorkflow(owner, projectCwd, title, workflow).taskId,
      );
      return this.task(id)!;
    }
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
    if (!this.writing)
      return this.transaction(() => this.workflowDecision(taskId, stepId, run, reason));
    const task = this.task(taskId);
    const index = task?.workflow?.steps.findIndex((s) => s.id === stepId) ?? -1;
    if (!task?.workflow || index < 0) throw new Error('Workflow step unavailable');
    const step = task.workflow.steps[index];
    const definition = task.workflow.definition.steps[index];
    if (
      step.state !== 'planned' ||
      step.decision !== undefined ||
      task.workflow.steps
        .slice(0, index)
        .some((s) => !['completed', 'skipped', 'waived'].includes(s.state))
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
  reserveWorkflowDispatch(taskId: string, revision: number, stepId: string): string {
    return this.transaction(() => {
      const task = this.task(taskId);
      if (!task || (task.revision ?? 0) !== revision)
        throw new Error('Task changed before dispatch; reload next step');
      if (task.dispatchReservation)
        throw new Error(
          'A workflow dispatch is already reserved; its host must finish or recover it',
        );
      const run = task.workflow?.steps.find(
        (s) => !['completed', 'skipped', 'waived'].includes(s.state),
      );
      if (run?.id !== stepId || run.state !== 'planned')
        throw new Error('Workflow step is no longer eligible');
      const token = randomUUID();
      task.dispatchReservation = { stepId, token, createdAt: new Date().toISOString() };
      return token;
    });
  }
  releaseWorkflowDispatch(taskId: string, token: string): void {
    this.transaction(() => {
      const task = this.task(taskId);
      if (task?.dispatchReservation?.token === token) delete task.dispatchReservation;
    });
  }
  private hostUserView(
    task: DispatchTask,
    session: (id: string) => { sessionId: string; status: string; hub?: string } | undefined,
  ): DispatchTask {
    const copy = this.viewTask(task);
    for (const a of copy.attempts) {
      const actual = session(a.sessionId);
      if (actual?.sessionId === a.sessionId && !actual.hub && actual.status === 'ended') {
        a.stale = false;
        a.live = false;
        a.lifecycle = 'ended';
      } else if (actual?.sessionId === a.sessionId && !actual.hub) a.live = true;
    }
    return copy;
  }
  listForHostUser(
    session: (id: string) => { sessionId: string; status: string; hub?: string } | undefined,
  ): DispatchTask[] {
    return this.list().map((t) => this.hostUserView(t, session));
  }
  /** Desktop-host user entry point. Deliberately absent from manager capabilities. */
  editByHostUser(
    request: TaskEditRequest,
    session: (id: string) => { sessionId: string; status: string; hub?: string } | undefined,
    busy: (taskId: string) => boolean,
  ): TaskEditResponse {
    try {
      this.transaction(() => {
        if (
          !request ||
          typeof request.taskId !== 'string' ||
          !Number.isSafeInteger(request.expectedTaskRevision) ||
          request.expectedTaskRevision < 0
        )
          throw new Error('Invalid task edit');
        const task = this.task(request.taskId);
        if (!task) throw new Error('Task is no longer available');
        if ((task.revision ?? 0) !== request.expectedTaskRevision) throw new TaskConflict();
        if (request.action === 'links') {
          task.links = validateTaskLinks(request.links);
        } else if (request.action === 'waive') {
          if (
            (!task.dispatchReservation && busy(task.taskId)) ||
            task.dispatchReservation?.stepId === request.stepId
          )
            throw new Error('A step is being dispatched. Try again after it starts.');
          const disabled = taskSkipDisabledReason(this.hostUserView(task, session), request.stepId);
          if (disabled) throw new Error(disabled);
          const run = task.workflow!.steps.find((s) => s.id === request.stepId)!;
          if (run.sessionId) {
            const actual = session(run.sessionId);
            if (
              !actual ||
              actual.sessionId !== run.sessionId ||
              actual.hub ||
              actual.status !== 'ended'
            )
              throw new Error('The exact attached worker has not been verified stopped');
          }
          const reason = request.reason ?? 'Skipped for this task by you';
          if (typeof reason !== 'string' || !reason.trim() || reason.length > 2000)
            throw new Error('Use a reason of 1–2000 characters');
          const audit = {
            id: randomUUID(),
            actor: 'host-user' as const,
            action: 'waive' as const,
            stepId: run.id,
            reason: reason.trim(),
            createdAt: new Date().toISOString(),
          };
          // Keep outcome, reason, attempt, and contract evidence unchanged.
          run.state = 'waived';
          run.waiverId = audit.id;
          task.audit = [...(task.audit ?? []), audit];
        } else throw new Error('Unknown task action');
        if (request.action === 'links')
          this.recordLinkAudit(task, 'host-user', 'Task references edited by you');
        else this.capLinkAudit(task);
      });
      return { ok: true, task: this.task(request.taskId)! };
    } catch (e) {
      return {
        ok: false,
        code: e instanceof TaskConflict ? 'conflict' : 'ineligible',
        error:
          e instanceof TaskConflict
            ? 'This task changed. Reload before applying your edit.'
            : String(e),
        task: request?.taskId ? this.task(request.taskId) : undefined,
      };
    }
  }
  /** Append a reference-edit audit row, then cap that history without touching waivers. */
  private recordLinkAudit(
    task: DispatchTask,
    actor: 'host-user' | 'manager',
    reason: string,
  ): void {
    task.audit = [
      ...(task.audit ?? []),
      { id: randomUUID(), actor, action: 'links', reason, createdAt: new Date().toISOString() },
    ];
    this.capLinkAudit(task);
  }
  /** Preserve all step waivers; cap editable-reference history independently. */
  private capLinkAudit(task: DispatchTask): void {
    const linkEntries = (task.audit ?? []).filter((a) => a.action === 'links').slice(-40);
    task.audit = (task.audit ?? []).filter((a) => a.action === 'waive' || linkEntries.includes(a));
  }
  /**
   * Manager-facing reference edit. Deliberately NARROWER than editByHostUser: it can
   * only upsert/remove exact reference entries under the caller's own revision CAS.
   * It never waives a step, never touches lifecycle, ownership, workflow or attempt
   * facts, and never replaces the whole reference set — the host user's other entries
   * survive a manager edit. Ownership/liveness is verified by the caller.
   */
  updateReferencesByManager(
    taskId: string,
    expectedTaskRevision: number,
    upsert: unknown,
    remove: unknown,
    reason: string,
    authorize: () => void,
  ): DispatchTask {
    return this.transaction(() => {
      // Recheck the live owner against the reloaded row under the write lock.
      authorize();
      if (!Number.isSafeInteger(expectedTaskRevision) || expectedTaskRevision < 0)
        throw new Error('Task reference updates require the current expectedTaskRevision');
      const task = this.task(taskId);
      if (!task) throw new Error('Task is no longer available');
      if ((task.revision ?? 0) !== expectedTaskRevision) throw new TaskConflict();
      const links = applyTaskReferences(task.links, upsert, remove);
      // Idempotent: an edit that changes nothing records no audit row and leaves the
      // revision alone, so a retried manager call cannot inflate the audit history.
      if (JSON.stringify(links) !== JSON.stringify(task.links ?? {})) {
        task.links = links;
        this.recordLinkAudit(task, 'manager', reason);
      }
      return this.task(taskId)!;
    });
  }
  /** Resolve only recorded, task-owned targets; renderer never supplies a path/URL. */
  openTarget(request: TaskOpenRequest): { kind: 'url' | 'worktree'; target: string } {
    const task = this.task(request?.taskId);
    if (!task) throw new Error('Task unavailable');
    if (request.kind === 'worktree') {
      const attempt = task.attempts.find((a) => a.dispatchId === request.dispatchId);
      if (
        !attempt?.worktree?.allocated ||
        attempt.worktree.fallback ||
        !path.isAbsolute(attempt.executionCwd)
      )
        throw new Error('No recorded allocated worktree for this attempt');
      if (!attempt.worktree.directoryIdentity)
        throw new Error(
          'Recorded worktree identity is unavailable; this older record cannot open a folder',
        );
      const target = fs.realpathSync(attempt.executionCwd);
      const stat = fs.statSync(target);
      if (
        target !== path.resolve(attempt.executionCwd) ||
        !stat.isDirectory() ||
        stat.dev !== attempt.worktree.directoryIdentity.dev ||
        stat.ino !== attempt.worktree.directoryIdentity.ino
      )
        throw new Error('Recorded worktree has moved or is unavailable');
      return { kind: 'worktree', target };
    }
    if (request.kind !== 'url') throw new Error('Unknown task target');
    const links = validateTaskLinks(task.links ?? {});
    const index = request.index;
    if (request.reference !== 'pullRequest' && (!Number.isSafeInteger(index) || index! < 0))
      throw new Error('Invalid reference index');
    const value =
      request.reference === 'pullRequest'
        ? links.pullRequest?.url
        : request.reference === 'tickets'
          ? links.tickets?.[index!]?.url
          : request.reference === 'references'
            ? links.references?.[index!]?.url
            : undefined;
    return { kind: 'url', target: validateTaskUrl(value) };
  }
  adoptWorkflowTasks(oldOwner: string, newOwner: string): void {
    // Historical API name: succession transfers ALL owned tasks. Mutate only
    // attribution so attempts, pinned policy and user metadata remain intact.
    if (!this.writing) return this.transaction(() => this.adoptWorkflowTasks(oldOwner, newOwner));
    // Check under the SAME file lock as ownership mutation. A reserved spawn
    // has allocated work but still owes accept() under its original owner.
    if (this.load().some((task) => task.ownerSessionId === oldOwner && task.dispatchReservation))
      throw new Error(
        'Manager adoption must wait for the source task dispatch reservation to settle',
      );
    for (const task of this.load())
      if (task.ownerSessionId === oldOwner) {
        task.ownerSessionId = newOwner;
        task.ownerLabel = newOwner;
      }
    this.flush();
  }
  list(): DispatchTask[] {
    if (!this.writing) this.tasks = undefined;
    const tasks = this.load()
      .map((t) => this.viewTask(t))
      .reverse();
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
        !t.workflow ||
        t.workflow.steps.every((s) => ['completed', 'skipped', 'waived'].includes(s.state)),
    );
    if (index < 0)
      throw new Error('Workflow history capacity reached; active tasks cannot be evicted');
    tasks.splice(index, 1);
  }
  flush(): void {
    if (!this.writing) this.transaction(() => undefined);
  }
  private persist(): void {
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
export class TaskConflict extends Error {}
export const dispatchHistoryStore = new DispatchHistoryStore(() =>
  path.join(getConfigDir(), 'dispatch-history.json'),
);
