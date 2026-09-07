import fs from 'fs';
import path from 'path';
/** Pinned-task interpretation and spawn admission. No Library initialization on lifecycle imports. */
import { configService } from './configService';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { claudeSessionStore } from './claudeSessionStore';
import { validateDispatchTemplateParams } from '../lib/dispatchTemplate';
import { reviewPolicy, type WorkflowTemplate } from '../shared/fleetWorkflow';
import type { DispatchTask } from '../shared/dispatchHistory';
export const workflowBusy = new Set<string>();
export function ownerTask(
  taskId: string | undefined,
  caller: string | undefined,
  cwd: string | undefined,
): DispatchTask {
  const owner = caller ? claudeSessionStore.getSnapshot(caller) : undefined;
  const task = taskId ? dispatchHistoryStore.task(taskId) : undefined;
  if (
    !owner?.isWakeTarget ||
    owner.status === 'ended' ||
    owner.hub ||
    !task?.workflow ||
    task.ownerSessionId !== caller ||
    task.projectCwd !== cwd
  )
    throw new Error('Workflow task unavailable or belongs to another manager/project');
  return task;
}
export function workflowInstructions(task: DispatchTask): string {
  const pin = task.workflow!;
  const i = pin.steps.findIndex((s) => !['completed', 'skipped'].includes(s.state));
  const run = pin.steps[i];
  const step = pin.definition.steps[i];
  const header = `Fleet workflow ${pin.definition.name} (${pin.definition.id}@${pin.definition.revision}, snapshot ${pin.hash}). ${reviewPolicy(pin.definition)}. Task ${task.taskId}, project ${task.projectCwd}.`;
  if (!run)
    return `${header}\nAll configured steps have returned valid result contracts or explicit skips. This is NOT a passing verdict: inspect each reported outcome: ${JSON.stringify(pin.steps.map((s) => ({ id: s.id, state: s.state, outcome: s.outcome, reason: s.reason })))}.`;
  if (run.state !== 'planned')
    return `${header}\nStep ${step.id} is ${run.state}. Do not launch another step or infer completion from idle/ended. ${run.reason ?? 'Wait for the host result wake.'} Reported outcome: ${JSON.stringify(run.outcome ?? null)}. Escalate failed/blocked steps; v1 does not auto-retry.`;
  if ((step.when === 'material_risk' || step.repairOf) && run.decision === undefined)
    return `${header}\nCall decide_workflow_step with taskId, cwd, stepId=${step.id}, run=true/false and a concrete reason. ${step.repairOf ? 'This is the sole bounded repair linked to review ' + step.repairOf + '. Inspect that reported outcome first.' : 'Decide whether material architecture, security or compatibility risk warrants this step.'}`;
  const t = pin.templates[step.template];
  const previous = pin.steps
    .slice(0, i)
    .reverse()
    .find((s) => s.dispatchId);
  return `${header}\nNext: ${step.label}. Call select_model with role=${step.role} and cwd=${task.projectCwd}; then spawn_agent with its explicit provider/model/effort/capability/decisionId, role=${step.role}, taskId=${task.taskId}, workflowStepId=${step.id}, stage=${step.stage}, parentSessionId=${task.ownerSessionId}, cwd=${task.projectCwd}${previous ? ', afterDispatchId=' + previous.dispatchId : ''}, template=${step.template}, toolScope=${['research', 'review', 'validate'].includes(step.kind) ? 'view' : 'operator'}, and templateParams filling ${JSON.stringify(t.params)}. The host uses the PINNED template/result contract, derives ${['research', 'review', 'validate'].includes(step.kind) ? 'view scope' : 'operator scope with an isolated worktree'}, and preserves routing ceilings, grants and delivery policy. Always a fresh worker: no resume or retry reuse. ${step.kind === 'review' ? 'Give the reviewer the acceptance criteria, diff and closing handoff only; never the implementer reasoning/transcript.' : ''}\nStep instructions: ${step.instructions}\nAfter dispatch end your turn; the host wakes you. No polling or automatic launches.`;
}
/** Explicit binding only. A wrapper keeps the admission lock across asynchronous allocation. */
export function workflowSpawn<T>(
  spawn: (params: unknown) => Promise<T>,
): (params: unknown) => Promise<T> {
  return async (raw) => {
    const p = { ...(raw as Record<string, unknown>) };
    if (!p.workflowStepId) return spawn(p);
    const task = ownerTask(p.taskId as string, p.dispatchOwnerSessionId as string, p.cwd as string);
    if (workflowBusy.has(task.taskId))
      throw new Error('Workflow step dispatch already in progress');
    const i = task.workflow!.steps.findIndex((s) => !['completed', 'skipped'].includes(s.state));
    const run = task.workflow!.steps[i];
    const step = task.workflow!.definition.steps[i];
    if (
      !step ||
      step.id !== p.workflowStepId ||
      run.state !== 'planned' ||
      ((step.when === 'material_risk' || step.repairOf) && run.decision !== true)
    )
      throw new Error('Workflow step is not eligible; call next_workflow_step');
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
      !p.decisionId ||
      p.message ||
      p.resultSchema
    )
      throw new Error(
        'Workflow dispatch requires exact next-step metadata, routing decision and fresh session; message/schema overrides are not accepted',
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
    workflowBusy.add(task.taskId);
    try {
      return await spawn(p);
    } finally {
      workflowBusy.delete(task.taskId);
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
  return tasks.length ? '\n\n' + tasks.map(workflowInstructions).join('\n\n') : '';
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
