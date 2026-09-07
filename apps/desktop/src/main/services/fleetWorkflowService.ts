import { workflowBusy, ownerTask, workflowInstructions } from './fleetWorkflowRuntime';
import path from 'path';
import fs from 'fs';
import { configService, getConfigDir } from './configService';
import { libraryService } from './libraryService';
import { dispatchTemplateParams } from '../lib/dispatchTemplate';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { claudeSessionStore } from './claudeSessionStore';
import { FleetWorkflowStore, WorkflowConflict } from './fleetWorkflowStore';
import {
  type WorkflowRequest,
  type WorkflowResponse,
  type WorkflowTemplate,
} from '../shared/fleetWorkflow';
import { workflowSelections } from '../shared/fleetWorkflowSelection';
const templates = (): WorkflowTemplate[] =>
  libraryService
    .list(undefined, (full) => {
      try {
        const root = fs.realpathSync(path.join(getConfigDir(), 'library'));
        const resolved = fs.realpathSync(full);
        const rel = path.relative(root, resolved);
        return rel && !rel.startsWith('..' + path.sep) && rel !== '..' && !path.isAbsolute(rel)
          ? resolved
          : null;
      } catch {
        return null;
      }
    })
    .filter((t) => t.kind === 'dispatch' && t.scope === 'global')
    .map((t) => ({
      id: t.id,
      body: t.body,
      resultSchema: t.resultSchema ?? {},
      params: dispatchTemplateParams(t.body),
    }));
export const fleetWorkflowStore = new FleetWorkflowStore(
  () => path.join(getConfigDir(), 'workflow-definitions.json'),
  templates,
  () => {
    const s = workflowSelections(configService.getConfig());
    return [s.defaultId, ...Object.values(s.projects)];
  },
);
function configuredProjectKey(cwd: string): string {
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
export function fleetWorkflowRequest(
  request: WorkflowRequest,
  callerSessionId?: string,
): WorkflowResponse {
  try {
    if (!request || typeof request !== 'object' || JSON.stringify(request).length > 100 * 1024)
      throw new Error('Invalid or oversized workflow request');
    if (request.cwd && (typeof request.cwd !== 'string' || !path.isAbsolute(request.cwd)))
      throw new Error('Workflow project cwd must be absolute');
    const { op, id, expectedRevision } = request;
    if (op === 'select' && request.cwd)
      request = { ...request, cwd: configuredProjectKey(request.cwd) };
    if (op === 'list')
      return {
        ok: true,
        catalog: {
          available: true,
          definitions: fleetWorkflowStore.list(),
          templates: templates(),
          ...workflowSelections(configService.getConfig()),
        },
      };
    if (op === 'get') {
      const definition = fleetWorkflowStore.list().find((d) => d.id === id);
      if (!definition) throw new Error('Workflow unavailable');
      return { ok: true, definition };
    }
    if (op === 'validate')
      return { ok: true, definition: fleetWorkflowStore.validate(request.definition) };
    if (['create', 'update', 'clone', 'disable', 'delete'].includes(op))
      return {
        ok: true,
        definition: fleetWorkflowStore.mutate(
          op as 'create',
          id,
          expectedRevision,
          request.definition,
          request.name,
        ),
      };
    if (op === 'select') {
      const select = () => {
        const config = configService.getConfig();
        const current = workflowSelections(config);
        if (expectedRevision !== current.selectionRevision)
          throw new WorkflowConflict(current.selectionRevision);
        const patch = request.cwd
          ? {
              projects: {
                ...config.projects,
                [request.cwd]: { ...config.projects?.[request.cwd] },
              },
            }
          : {};
        if (request.cwd) {
          if (request.workflowId === null) delete patch.projects![request.cwd].workflowId;
          else patch.projects![request.cwd].workflowId = request.workflowId!;
        }
        const saved = configService.saveConfig(
          {
            ...patch,
            agents: {
              ...config.agents,
              ...(!request.cwd ? { defaultWorkflowId: request.workflowId! } : {}),
              workflowSelectionRevision: current.selectionRevision + 1,
            },
          },
          true,
        );
        if (saved.agents.workflowSelectionRevision !== current.selectionRevision + 1)
          throw new Error('Workflow selection could not be persisted');
      };
      if (request.cwd && request.workflowId === null)
        fleetWorkflowStore.withDefinition(
          workflowSelections(configService.getConfig()).defaultId,
          select,
        );
      else if (typeof request.workflowId === 'string')
        fleetWorkflowStore.withDefinition(request.workflowId, select);
      else throw new Error('Global selection requires an enabled workflow id');
      return fleetWorkflowRequest({ op: 'list' });
    }
    if (op === 'start') {
      const owner = callerSessionId ? claudeSessionStore.getSnapshot(callerSessionId) : undefined;
      if (
        !owner?.isWakeTarget ||
        owner.hub ||
        owner.status === 'ended' ||
        !request.cwd ||
        !request.title?.trim() ||
        request.taskId
      )
        throw new Error('New workflow requires a live local manager, project cwd and title');
      const selections = workflowSelections(configService.getConfig());
      return fleetWorkflowStore.withDefinition(
        selections.projects[configuredProjectKey(request.cwd)] ?? selections.defaultId,
        (d) => {
          const task = dispatchHistoryStore.startWorkflow(
            owner,
            request.cwd!,
            request.title!,
            fleetWorkflowStore.pin(d),
          );
          return { ok: true, task, instructions: workflowInstructions(task) };
        },
      );
    }
    if (op === 'next' || op === 'decide') {
      const task = ownerTask(request.taskId, callerSessionId, request.cwd);
      if (op === 'decide') {
        if (workflowBusy.has(task.taskId)) throw new Error('Step dispatch in progress');
        if (
          typeof request.run !== 'boolean' ||
          !request.stepId ||
          typeof request.reason !== 'string'
        )
          throw new Error('Decision requires stepId, run and reason');
        dispatchHistoryStore.workflowDecision(
          task.taskId,
          request.stepId,
          request.run,
          request.reason,
        );
      }
      return { ok: true, task: structuredClone(task), instructions: workflowInstructions(task) };
    }
    throw new Error('Unknown workflow operation');
  } catch (e) {
    return {
      ok: false,
      code: e instanceof WorkflowConflict ? 'conflict' : 'unavailable',
      error: e instanceof Error ? e.message : String(e),
      ...(e instanceof WorkflowConflict ? { currentRevision: e.currentRevision } : {}),
    };
  }
}
