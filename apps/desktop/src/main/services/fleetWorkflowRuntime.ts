import { managerReplacementState } from './managerReplacementState';
import fs from 'fs';
import path from 'path';
/** Pinned-task interpretation and spawn admission. No Library initialization on lifecycle imports. */
import { configService } from './configService';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { claudeSessionStore } from './claudeSessionStore';
import { validateDispatchTemplateParams } from '../lib/dispatchTemplate';
import {
  reviewPolicy,
  type WorkflowTemplate,
  type WorkflowDispatchPlan,
} from '../shared/fleetWorkflow';
import type { DispatchTask } from '../shared/dispatchHistory';
import { taskDependencyState } from '../shared/managerRequests';
export const workflowBusy = new Set<string>();
/**
 * The single ownership gate for manager-facing task operations: a live, local,
 * wake-target manager session, addressing its OWN task in that task's exact project.
 * `requireWorkflow` is only relaxed for operations that read or annotate a task
 * without interpreting pinned workflow policy (task references).
 */
export function ownerTask(
  taskId: string | undefined,
  caller: string | undefined,
  cwd: string | undefined,
  requireWorkflow = true,
): DispatchTask {
  managerReplacementState.assertAvailable(caller);
  const owner = caller ? claudeSessionStore.getSnapshot(caller) : undefined;
  const task = taskId ? dispatchHistoryStore.task(taskId) : undefined;
  if (
    !owner?.isWakeTarget ||
    owner.status === 'ended' ||
    owner.hub ||
    !task ||
    (requireWorkflow && !task.workflow) ||
    task.ownerSessionId !== caller ||
    task.projectCwd !== cwd
  )
    throw new Error('Workflow task unavailable or belongs to another manager/project');
  return task;
}
export function workflowInstructions(task: DispatchTask): string {
  const state = taskDependencyState(
    task,
    task.dependsOn?.length ? dispatchHistoryStore.list() : [],
  );
  if (state !== 'ready')
    return `Task ${task.taskId} is ${state}. Await explicit accepted dependency evidence; do not dispatch or bypass its pinned workflow. Use manager_context on your next wake for current task state.`;
  const pin = task.workflow!;
  const i = pin.steps.findIndex((s) => !['completed', 'skipped', 'waived'].includes(s.state));
  const run = pin.steps[i];
  const step = pin.definition.steps[i];
  const waivers = task.audit?.filter((a) => a.action === 'waive') ?? [];
  const header = `Host-user skips: ${JSON.stringify(waivers)}. These are waivers, never passing results. Fleet workflow ${pin.definition.name} (${pin.definition.id}@${pin.definition.revision}, snapshot ${pin.hash}). ${reviewPolicy(pin.definition)}. Task ${task.taskId}, project ${task.projectCwd}.`;
  if (task.dispatchReservation)
    return `${header}\nStep ${task.dispatchReservation.stepId} is being dispatched. Its host must finish or recover that dispatch before launching another worker.`;
  if (!run)
    return `${header}\nAll configured steps have returned valid result contracts or explicit skips. This is NOT a passing verdict: inspect each reported outcome: ${JSON.stringify(pin.steps.map((s) => ({ id: s.id, state: s.state, outcome: s.outcome, reason: s.reason })))}.`;
  if (run.state !== 'planned')
    return `${header}\nStep ${step.id} is ${run.state}. Do not launch another step or infer completion from idle/ended. ${run.reason ?? 'Wait for the host result wake.'} Reported outcome: ${JSON.stringify(run.outcome ?? null)}. Escalate failed/blocked steps; v1 does not auto-retry.`;
  if ((step.when === 'material_risk' || step.repairOf) && run.decision === undefined)
    return `${header}\nConditional decision required. Call dispatch_workflow_step with taskId=${task.taskId}, cwd=${task.projectCwd}, stepId=${step.id}, expectedTaskRevision=${task.revision ?? 0}, run=true/false and a concrete reason. For run=true fill templateParams ${JSON.stringify(pin.templates[step.template].params)}; run=false records only this skip and stops. decide_workflow_step is available for a separate decision. ${step.repairOf ? 'This is the sole bounded repair linked to review ' + step.repairOf + '. Inspect that reported outcome first.' : 'Decide whether material architecture, security or compatibility risk warrants this step.'}`;
  const missing = missingWorkflowEvidence(task, i);
  if (missing) return `${header}\n${missing}. Do not dispatch this step or invent artifacts.`;
  const t = pin.templates[step.template];
  return `${header}\nNext: ${step.label}. Call dispatch_workflow_step with taskId=${task.taskId}, cwd=${task.projectCwd}, stepId=${step.id}, expectedTaskRevision=${task.revision ?? 0}, and templateParams filling ${JSON.stringify(t.params)}. It uses optional modelSelection:{provider,model,effort} for an explicit user choice, otherwise automatic routing, and binds pinned spawn metadata; do not also call select_model or spawn_agent. The host preserves ceilings, grants, isolation and delivery policy. ${step.kind === 'review' ? 'Give the fresh reviewer criteria, diff and closing handoff only; never implementer reasoning/transcript.' : ''}\nStep instructions: ${step.instructions}\nAfter dispatch end your turn; the host wakes you. No polling or automatic launches.`;
}

/** The machine-readable counterpart of the next-step instructions. Never grants authority. */
export function workflowDispatchPlan(task: DispatchTask): WorkflowDispatchPlan | undefined {
  if (!task.workflow || task.dispatchReservation || workflowBusy.has(task.taskId)) return;
  if (
    taskDependencyState(task, task.dependsOn?.length ? dispatchHistoryStore.list() : []) !== 'ready'
  )
    return;
  const pin = task.workflow;
  const index = pin.steps.findIndex((s) => !['completed', 'skipped', 'waived'].includes(s.state));
  const run = pin.steps[index];
  const step = pin.definition.steps[index];
  if (
    !step ||
    run.state !== 'planned' ||
    ((step.when === 'material_risk' || step.repairOf) && run.decision !== true) ||
    missingWorkflowEvidence(task, index)
  )
    return;
  const previous = pin.steps
    .slice(0, index)
    .reverse()
    .find((s) => s.dispatchId);
  const independent = step.independentOf
    ? pin.steps.find((s) => s.id === step.independentOf)
    : step.kind === 'review'
      ? previous
      : undefined;
  const provider =
    independent && task.attempts.find((a) => a.dispatchId === independent.dispatchId)?.provider;
  return {
    taskId: task.taskId,
    cwd: task.projectCwd,
    stepId: step.id,
    expectedTaskRevision: task.revision ?? 0,
    role: step.role,
    stage: step.stage,
    template: step.template,
    params: structuredClone(pin.templates[step.template].params),
    toolScope: ['research', 'review', 'validate'].includes(step.kind) ? 'view' : 'operator',
    ...(previous?.dispatchId ? { afterDispatchId: previous.dispatchId } : {}),
    ...(provider ? { previousProvider: provider } : {}),
  };
}
/** Explicit binding only. A wrapper keeps the admission lock across asynchronous allocation. */
export function workflowSpawn<T>(
  spawn: (params: unknown) => Promise<T>,
): (params: unknown) => Promise<T> {
  return async (raw) => {
    const p = { ...(raw as Record<string, unknown>) };
    if (!p.workflowStepId) return spawn(p);
    const task = ownerTask(p.taskId as string, p.dispatchOwnerSessionId as string, p.cwd as string);
    if (
      p.expectedTaskRevision !== undefined &&
      (!Number.isSafeInteger(p.expectedTaskRevision) ||
        p.expectedTaskRevision !== (task.revision ?? 0))
    )
      throw new Error('Task changed before dispatch; reload next step');
    if (workflowBusy.has(task.taskId))
      throw new Error('Workflow step dispatch already in progress');
    const i = task.workflow!.steps.findIndex(
      (s) => !['completed', 'skipped', 'waived'].includes(s.state),
    );
    const run = task.workflow!.steps[i];
    const step = task.workflow!.definition.steps[i];
    if (
      !step ||
      step.id !== p.workflowStepId ||
      run.state !== 'planned' ||
      ((step.when === 'material_risk' || step.repairOf) && run.decision !== true)
    )
      throw new Error('Workflow step is not eligible; call next_workflow_step');
    const missing = missingWorkflowEvidence(task, i);
    if (missing) throw new Error(missing);
    const previous = task
      .workflow!.steps.slice(0, i)
      .reverse()
      .find((s) => s.dispatchId);
    if (
      !['view', 'triage', 'operator'].includes(p.toolScope as string) ||
      p.manager ||
      p.resumeSessionId ||
      p.retrySourceSessionId ||
      p.parentSessionId !== task.ownerSessionId ||
      p.stage !== step.stage ||
      p.role !== step.role ||
      p.template !== step.template ||
      p.afterDispatchId !== previous?.dispatchId ||
      !p.provider ||
      (!p.decisionId && !p.model && !p.modelIdentity) ||
      p.message ||
      p.resultSchema
    )
      throw new Error(
        'Workflow dispatch requires exact next-step metadata, a routed or explicitly named model, and a fresh session; message/schema overrides are not accepted',
      );
    const t = task.workflow!.templates[step.template];
    const params = { ...((p.templateParams as Record<string, string>) ?? {}) };
    validateDispatchTemplateParams(t.body, params);
    if (step.instructions) {
      if (!t.params.some((p) => p.name === 'task'))
        throw new Error('Step instructions require a template task input');
      params.task = `${params.task}\n\n${step.instructions}`;
    }
    const project =
      configService.getConfig().projects?.[configuredWorkflowProjectKey(task.projectCwd)];
    if (t.params.some((p) => p.name === 'delivery'))
      params.delivery =
        project?.delivery === 'local'
          ? 'Commit on the isolated branch for an approved local merge. Do not push or merge without authority.'
          : 'Open a pull request for review only when authorized by the task. Do not merge.';
    p.templateParams = params;
    // The router already applied the authority ceiling. A definition can only narrow it.
    p.toolScope =
      ['research', 'review', 'validate'].includes(step.kind) || p.toolScope === 'view'
        ? 'view'
        : p.toolScope === 'triage'
          ? 'triage'
          : 'operator';
    p.worktree = !['research', 'review', 'validate'].includes(step.kind);
    const reservation = dispatchHistoryStore.reserveWorkflowDispatch(
      task.taskId,
      task.revision ?? 0,
      step.id,
    );
    workflowBusy.add(task.taskId);
    try {
      return await spawn(p);
    } finally {
      workflowBusy.delete(task.taskId);
      dispatchHistoryStore.releaseWorkflowDispatch(task.taskId, reservation);
    }
  };
}
export function pinnedWorkflowTemplate(taskId: string, stepId: string): WorkflowTemplate {
  const pin = dispatchHistoryStore.task(taskId)?.workflow;
  const step = pin?.definition.steps.find((s) => s.id === stepId);
  if (!pin || !step) throw new Error('Pinned workflow template unavailable');
  return structuredClone(pin.templates[step.template]);
}
export function workflowWakeInstructions(sessionIds: string[]): string {
  const tasks = dispatchHistoryStore
    .list()
    .filter((t) => t.workflow && t.attempts.some((a) => sessionIds.includes(a.sessionId)));
  return (
    (tasks.length ? '\n\n' + tasks.map(workflowInstructions).join('\n\n') : '') +
    '\n\nFleet event, not a new user request. Use manager_context once for pending requests and the tasks in this wake; it returns current evidence and next dispatch inputs. Preserve existing request/task lineage.'
  );
}

export function workflowResultSchema(sessionId: string): Record<string, unknown> | undefined {
  const task = dispatchHistoryStore
    .list()
    .find((t) => t.workflow && t.attempts.some((a) => a.sessionId === sessionId));
  const run = task?.workflow?.steps.find((s) => s.sessionId === sessionId);
  const step = task?.workflow?.definition.steps.find((s) => s.id === run?.id);
  return step ? task!.workflow!.templates[step.template].resultSchema : undefined;
}

export function configuredWorkflowProjectKey(cwd: string): string {
  const canonical = (value: string) => {
    try {
      return fs.realpathSync(value);
    } catch {
      return path.resolve(value);
    }
  };
  return (
    Object.keys(configService.getConfig().projects ?? {}).find(
      (key) => canonical(key) === canonical(cwd),
    ) ?? cwd
  );
}

/** Waivers remove obligations, never produce predecessor artifacts. */
export function missingWorkflowEvidence(task: DispatchTask, index: number): string | undefined {
  const pin = task.workflow!;
  const step = pin.definition.steps[index];
  for (const id of [step.independentOf, step.repairOf].filter((s): s is string => !!s)) {
    const run = pin.steps.find((s) => s.id === id);
    const attempt = task.attempts.find(
      (a) =>
        a.dispatchId === run?.dispatchId &&
        a.sessionId === run?.sessionId &&
        a.workflowStepId === id,
    );
    if (!run || !attempt || attempt.resultContract !== 'valid' || run.outcome === undefined)
      return `Required evidence from step ${id} is unavailable for ${step.label}`;
  }
}
