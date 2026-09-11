import {
  workflowBusy,
  ownerTask,
  workflowInstructions,
  workflowDispatchPlan,
  configuredWorkflowProjectKey,
} from './fleetWorkflowRuntime';
import path from 'path';
import fs from 'fs';
import { configService, getConfigDir } from './configService';
import { libraryService } from './libraryService';
import { dispatchTemplateParams, validateDispatchTemplateParams } from '../lib/dispatchTemplate';
import { dispatchHistoryStore, TaskConflict } from './dispatchHistoryStore';
import { claudeSessionStore } from './claudeSessionStore';
import { FleetWorkflowStore, WorkflowConflict } from './fleetWorkflowStore';
import {
  type WorkflowRequest,
  type WorkflowResponse,
  type WorkflowTemplate,
} from '../shared/fleetWorkflow';
import { workflowSelections } from '../shared/fleetWorkflowSelection';
import { taskDependencyState } from '../shared/managerRequests';
import { managerReplacementState } from './managerReplacementState';
import {
  configureManagerRequests,
  ManagerRequestService,
  managerRequests,
} from './managerRequestService';
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
configureManagerRequests(
  () =>
    new ManagerRequestService(
      dispatchHistoryStore,
      (id) => claudeSessionStore.getSnapshot(id) ?? undefined,
      (cwd, workflowId) => {
        const selections = workflowSelections(configService.getConfig());
        return fleetWorkflowStore.withDefinition(
          workflowId ??
            selections.projects[configuredWorkflowProjectKey(cwd)] ??
            selections.defaultId,
          (definition) => fleetWorkflowStore.pin(definition),
        );
      },
    ),
);
function freeformTaskInstructions(task: import('../shared/dispatchHistory').DispatchTask): string {
  return `Task ${task.taskId} has no pinned workflow. Dispatch with spawn_agent using taskId=${task.taskId}, cwd=${task.projectCwd}, parentSessionId=${task.ownerSessionId} and the task-specific message/role. Honor explicit provider/model choices; otherwise use select_model. Do not invent workflow steps or mandatory review for this freeform task.`;
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
    if (
      op === 'resolveRequest' &&
      callerSessionId &&
      managerReplacementState.activeCount(callerSessionId)
    )
      throw new Error('A dispatch is being admitted; resolve the inbox request after it settles');
    if (['requestInbox', 'requestContent', 'resolveRequest', 'acceptTaskOutcome'].includes(op)) {
      const response = managerRequests().handle(
        request as import('../shared/managerRequests').ManagerRequestOperation,
        callerSessionId ?? '',
      );
      if (response.ok && (op === 'resolveRequest' || op === 'acceptTaskOutcome')) {
        const tasks = (op === 'resolveRequest' ? response.tasks : response.readyTasks) as
          Array<{ taskId: string }> | undefined;
        const ids = [...new Set((tasks ?? []).map((t) => t.taskId))];
        response.nextActionsRemaining = Math.max(0, ids.length - 4);
        response.nextActions = ids.slice(0, 4).map((taskId) => {
          try {
            const task = dispatchHistoryStore.task(taskId);
            if (!task || task.ownerSessionId !== callerSessionId)
              return { taskId, unavailable: true };
            if (!task.workflow)
              return {
                taskId,
                cwd: task.projectCwd,
                revision: task.revision ?? 0,
                instructions: freeformTaskInstructions(task),
              };
            return {
              taskId,
              cwd: task.projectCwd,
              revision: task.revision ?? 0,
              instructions: workflowInstructions(task),
              dispatch: workflowDispatchPlan(task),
            };
          } catch {
            // An optional projection must never disguise a committed resolution
            // or evidence acceptance as a failed mutation.
            return { taskId, unavailable: true };
          }
        });
      }
      return response as WorkflowResponse;
    }
    if (op === 'select' && request.cwd)
      request = { ...request, cwd: configuredWorkflowProjectKey(request.cwd) };
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
        selections.projects[configuredWorkflowProjectKey(request.cwd)] ?? selections.defaultId,
        (d) => {
          const task = dispatchHistoryStore.startWorkflow(
            owner,
            request.cwd!,
            request.title!,
            fleetWorkflowStore.pin(d),
          );
          return {
            ok: true,
            task,
            instructions: workflowInstructions(task),
            dispatch: workflowDispatchPlan(task),
          };
        },
      );
    }
    if (op === 'taskReferences' || op === 'setTaskReferences') {
      // Reference edits need ownership, not pinned policy, so an unpinned owned task
      // can still carry the PR/ticket the user pasted into chat.
      let task = ownerTask(request.taskId, callerSessionId, request.cwd, false);
      if (op === 'setTaskReferences') {
        if (workflowBusy.has(task.taskId)) throw new Error('Step dispatch in progress');
        task = dispatchHistoryStore.updateReferencesByManager(
          task.taskId,
          request.expectedTaskRevision as number,
          request.upsert,
          request.remove,
          'Task references recorded by your manager from the conversation',
          () => {
            ownerTask(request.taskId, callerSessionId, request.cwd, false);
          },
        );
        // Re-run the ownership gate against the committed row.
        task = ownerTask(task.taskId, callerSessionId, request.cwd, false);
      }
      return {
        ok: true,
        task: structuredClone(task),
        references: structuredClone(task.links ?? {}),
        taskRevision: task.revision ?? 0,
      };
    }
    if (op === 'prepareDispatch') {
      // No provider/network IO or routing inside this transaction. Check ownership/revision and
      // an optional conditional decision against the same locked task row.
      dispatchHistoryStore.requestTransaction(() => {
        const task = ownerTask(request.taskId, callerSessionId, request.cwd);
        if (
          !Number.isSafeInteger(request.expectedTaskRevision) ||
          request.expectedTaskRevision! < 0
        )
          throw new Error('Dispatch requires expectedTaskRevision');
        if ((task.revision ?? 0) !== request.expectedTaskRevision) throw new TaskConflict();
        if (taskDependencyState(task, dispatchHistoryStore.list()) !== 'ready')
          throw new Error('Task dependencies or cancellation prevent dispatch');
        const index = task.workflow!.steps.findIndex(
          (s) => !['completed', 'skipped', 'waived'].includes(s.state),
        );
        const step = task.workflow!.definition.steps[index];
        if (
          !step ||
          step.id !== request.stepId ||
          task.workflow!.steps[index].state !== 'planned' ||
          task.dispatchReservation ||
          workflowBusy.has(task.taskId)
        )
          throw new Error('Requested workflow step is not available for dispatch');
        if (request.run !== false)
          validateDispatchTemplateParams(
            task.workflow!.templates[step.template].body,
            request.templateParams ?? {},
          );
        if (request.run !== undefined) {
          if (typeof request.run !== 'boolean' || typeof request.reason !== 'string')
            throw new Error('Conditional decision requires run and reason');
          dispatchHistoryStore.workflowDecision(task.taskId, step.id, request.run, request.reason);
        }
      });
      const task = ownerTask(request.taskId, callerSessionId, request.cwd);
      const dispatch = request.run === false ? undefined : workflowDispatchPlan(task);
      if (!dispatch && request.run !== false)
        return { ok: false, code: 'ineligible', error: workflowInstructions(task) };
      return {
        ok: true,
        ...(dispatch ? { dispatch } : { skipped: true }),
        taskRevision: task.revision ?? 0,
        instructions: workflowInstructions(task),
      };
    }
    if (op === 'next' || op === 'decide') {
      let task = ownerTask(request.taskId, callerSessionId, request.cwd, op === 'decide');
      if (!task.workflow) return { ok: true, task, instructions: freeformTaskInstructions(task) };
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
        // Store reads are snapshots; the transaction commits a different row.
        // Return that committed revision and derive instructions from it.
        task = ownerTask(task.taskId, callerSessionId, request.cwd);
      }
      return {
        ok: true,
        task: structuredClone(task),
        instructions: workflowInstructions(task),
        dispatch: workflowDispatchPlan(task),
      };
    }
    throw new Error('Unknown workflow operation');
  } catch (e) {
    const conflict = e instanceof TaskConflict;
    return {
      ok: false,
      code: e instanceof WorkflowConflict || conflict ? 'conflict' : 'unavailable',
      error: conflict
        ? 'This task changed. Read its current references and revision, then reapply your edit.'
        : e instanceof Error
          ? e.message
          : String(e),
      ...(e instanceof WorkflowConflict ? { currentRevision: e.currentRevision } : {}),
      ...(conflict && request?.taskId
        ? (() => {
            let current;
            try {
              current = ownerTask(request.taskId, callerSessionId, request.cwd, false);
            } catch {
              return {};
            }
            return current
              ? { currentRevision: current.revision ?? 0, references: current.links ?? {} }
              : {};
          })()
        : {}),
    };
  }
}
