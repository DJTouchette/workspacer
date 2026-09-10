/** Production Task Inspector and Settings, synthetic IPC only. Never touches a live store. */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import TaskInspector from '../components/TaskInspector';
import FleetWorkflowsSection from '../components/settings/FleetWorkflowsSection';
import { applyTheme, resolveTheme } from '../themes';
import { createBridgedBackend } from '../backend/bridgedBackend';
import { POLICY_SETTINGS_EVENT } from '../lib/settingsBus';
import { INSPECTOR_OPEN_EVENT, requestInspector } from '../lib/watchBus';
import { WORKFLOW_STARTERS } from '../../../main/shared/fleetWorkflow';
import {
  validateTaskLinks,
  taskSkipDisabledReason,
  type DispatchTask,
  type TaskEditRequest,
  type TaskOpenRequest,
} from '../../../main/shared/dispatchHistory';
import type { ElectronAPI } from '../types/electron';
const params = new URLSearchParams(location.search);
const mode = params.get('mode');
applyTheme(resolveTheme(params.get('theme') ?? 'everforest'));
const definition = structuredClone(WORKFLOW_STARTERS[0]);
const task: DispatchTask = {
  taskId: 'task-current',
  revision: 1,
  ownerSessionId: 'manager',
  ownerLabel: 'Project manager',
  title: 'Build Task Inspector',
  projectCwd: '/project/alpha',
  createdAt: '2026-09-09T00:00:00Z',
  workflow: {
    definition,
    hash: 'fixture-pin',
    templates: Object.fromEntries(
      definition.steps.map((s) => [
        s.template,
        {
          id: s.template,
          body: '{{task}}',
          params: [{ name: 'task', required: true }],
          resultSchema: { type: 'object' },
        },
      ]),
    ),
    steps: [
      { id: 'scout', state: 'skipped', reason: 'No material risk' },
      { id: 'implement', state: 'dispatched', sessionId: 'worker', dispatchId: 'attempt' },
      { id: 'review', state: 'planned' },
    ],
  },
  attempts: [
    {
      dispatchId: 'attempt',
      sessionId: 'worker',
      workflowStepId: 'implement',
      kind: 'fresh',
      acceptedAt: '2026-09-09T00:00:00Z',
      observedAt: '2026-09-09T00:01:00Z',
      executionCwd: '/worktrees/recorded',
      worktree: {
        requested: true,
        allocated: true,
        fallback: false,
        branch: 'recorded-branch',
        directoryIdentity: { dev: 1, ino: 1 },
      },
      lifecycle: 'running',
      live: true,
      stale: false,
      resultContract: 'absent',
      metrics: {},
    },
  ],
};
if (mode === 'links') {
  task.links = {
    pullRequest: {
      number: '9492',
      url: 'https://dev.azure.test/org/project/_git/repo/pullrequest/9492',
    },
    tickets: [{ id: 'WKS-412', url: 'https://jira.test/browse/WKS-412' }, { id: 'SUP-77' }],
    references: [{ label: 'Design notes', url: 'https://example.test/design' }],
  };
  task.workflow!.steps[0].reason =
    'Scout skipped: the change is confined to one renderer component';
  task.workflow!.steps[1].state = 'completed';
  task.workflow!.steps[1].outcome = { commit: 'abc1234' };
  task.workflow!.steps[2].state = 'planned';
}
if (mode === 'failed' || mode === 'stale') {
  task.workflow!.steps[1].state = 'failed';
  task.workflow!.steps[1].outcome = { failure: 'Original failure evidence' };
  task.workflow!.steps[1].reason = 'Worker result contract: invalid';
  task.attempts[0].live = false;
  task.attempts[0].lifecycle = 'ended';
  task.attempts[0].resultContract = 'invalid';
  task.attempts[0].stale = mode === 'stale';
}
const recent: DispatchTask = {
  taskId: 'recent',
  revision: 0,
  ownerSessionId: 'old-manager',
  ownerLabel: 'Previous manager',
  title: 'Recent standalone task',
  projectCwd: '/project/beta',
  createdAt: '2026-09-08T00:00:00Z',
  attempts: [],
};
const calls: unknown[] = [];
Object.assign(window, {
  taskCalls: calls,
  fixtureTask: task,
  fixtureChange: () => {
    task.revision!++;
    task.ownerLabel = 'Replacement manager';
    task.links = { ...task.links, tickets: [...(task.links?.tickets ?? []), { id: 'EXTERNAL-1' }] };
    task.audit = [
      ...(task.audit ?? []),
      {
        id: `manager-links-${task.revision}`,
        actor: 'manager',
        action: 'links',
        reason: 'Task references edited by manager',
        createdAt: '2026-09-09T03:00:00Z',
      },
    ];
  },
});
const ipc = {
  platform: 'linux',
  ...(mode !== 'old'
    ? {
        dispatchHistoryRead: async () => {
          if (mode === 'error') throw new Error('Fixture read failure');
          if (mode === 'loading') await new Promise((r) => setTimeout(r, 10000));
          return {
            available: true,
            currentOwnerSessionId: mode === 'no-manager' ? undefined : 'manager',
            tasks: mode === 'empty' ? [] : structuredClone([recent, task]),
          };
        },
        taskInspectorEdit: async (request: TaskEditRequest) => {
          calls.push(structuredClone(request));
          if (request.expectedTaskRevision !== task.revision)
            return {
              ok: false,
              code: 'conflict',
              error: 'This task changed. Reload before applying your edit.',
              task: structuredClone(task),
            };
          if (request.action === 'waive') {
            const disabled = taskSkipDisabledReason(task, request.stepId);
            if (disabled) return { ok: false, code: 'ineligible', error: disabled };
            const audit = {
              id: 'waiver',
              actor: 'host-user' as const,
              action: 'waive' as const,
              stepId: request.stepId,
              reason: request.reason ?? 'Skipped for this task by you',
              createdAt: '2026-09-09T04:00:00Z',
            };
            const run = task.workflow!.steps.find((s) => s.id === request.stepId)!;
            run.state = 'waived';
            run.waiverId = audit.id;
            task.audit = [...(task.audit ?? []), audit];
          } else if (request.action === 'handoff-disposition') {
            const attempt = task.attempts.find((a) => a.dispatchId === request.dispatchId);
            if (!attempt?.handoff)
              return { ok: false, code: 'ineligible', error: 'No verified handoff' };
            attempt.handoff.disposition = request.keep ? 'keep' : 'accepted';
          } else {
            task.links = validateTaskLinks(request.links);
            task.audit = [
              ...(task.audit ?? []),
              {
                id: `host-links-${task.revision}`,
                actor: 'host-user',
                action: 'links',
                reason: 'Task references edited by you',
                createdAt: '2026-09-09T04:00:00Z',
              },
            ];
          }
          task.revision!++;
          return { ok: true, task: structuredClone(task) };
        },
        taskInspectorOpen: async (request: TaskOpenRequest) => {
          calls.push(request);
          return { ok: true };
        },
      }
    : {}),
  fleetWorkflowRequest: async (request: {
    op: string;
    cwd?: string;
    workflowId?: string;
    expectedRevision?: number;
  }) => {
    calls.push(request);
    if (request.op === 'list')
      return {
        ok: true,
        catalog: {
          available: true,
          definitions: WORKFLOW_STARTERS,
          templates: [],
          defaultId: WORKFLOW_STARTERS[0].id,
          projects: {},
          selectionRevision: 4,
        },
      };
    if (request.op === 'select') return { ok: true };
    return { ok: false, error: 'Fixture request unavailable' };
  },
} as unknown as ElectronAPI;
window.electronAPI = createBridgedBackend(ipc, 'fixture', `ws://${location.host}/fixture-bus`);
function Harness() {
  const [settings, setSettings] = useState(false);
  const [target, setTarget] = useState<string>();
  React.useEffect(() => {
    const show = () => setSettings(true);
    const inspect = (event: Event) => setTarget((event as CustomEvent).detail.taskId);
    window.addEventListener(POLICY_SETTINGS_EVENT, show);
    window.addEventListener(INSPECTOR_OPEN_EVENT, inspect);
    return () => {
      window.removeEventListener(POLICY_SETTINGS_EVENT, show);
      window.removeEventListener(INSPECTOR_OPEN_EVENT, inspect);
    };
  }, []);
  return (
    <div
      style={{
        // The rail is narrow in production. `width` lets a screenshot reproduce
        // the real 360-480px sidebar as well as a wide pane.
        width: params.get('width') ? Number(params.get('width')) : undefined,
        maxWidth: params.get('width') ? undefined : 500,
        margin: params.get('width') ? 0 : 'auto',
        background: 'var(--wks-bg-base)',
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'auto',
      }}
    >
      {mode === 'route' && (
        <button onClick={() => requestInspector({ taskId: 'recent' })}>Inspect recent task</button>
      )}
      {settings ? (
        <FleetWorkflowsSection />
      ) : (
        <TaskInspector
          taskId={target}
          sessionId={params.get('worker') ?? undefined}
          remote={mode === 'remote'}
        />
      )}
    </div>
  );
}
createRoot(document.getElementById('root')!).render(<Harness />);
