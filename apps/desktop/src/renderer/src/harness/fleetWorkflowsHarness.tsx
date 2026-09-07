/** Production workflow forms + pinned-task projection. Only the IPC boundary is synthetic. */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { applyTheme, resolveTheme } from '../themes';
import FleetWorkflowsSection, {
  ProjectWorkflowSelector,
} from '../components/settings/FleetWorkflowsSection';
import FleetWorkflowTask from '../components/FleetWorkflowTask';
import { createBridgedBackend } from '../backend/bridgedBackend';
import {
  WORKFLOW_STARTERS,
  validateWorkflow,
  type WorkflowCatalog,
  type WorkflowRequest,
  type WorkflowResponse,
} from '../../../main/shared/fleetWorkflow';
import type { ElectronAPI } from '../types/electron';
const params = new URLSearchParams(location.search);
applyTheme(resolveTheme(params.get('theme') ?? 'dracula'));
const catalog: WorkflowCatalog = {
  available: true,
  definitions: structuredClone(WORKFLOW_STARTERS),
  templates: ['ship-task', 'review-task', 'scout-task'].map((id) => ({
    id,
    body: '{{task}}',
    resultSchema: { type: 'object' },
    params: [{ name: 'task', required: true }],
  })),
  defaultId: WORKFLOW_STARTERS[0].id,
  projects: {},
  selectionRevision: 0,
};
const calls: WorkflowRequest[] = [];
Object.assign(window, { workflowCalls: calls });
const request = async (r: WorkflowRequest): Promise<WorkflowResponse> => {
  calls.push(structuredClone(r));
  if (r.op === 'list') return { ok: true, catalog: structuredClone(catalog) };
  if (r.op === 'clone') {
    const old = catalog.definitions.find((d) => d.id === r.id)!;
    const definition = {
      ...structuredClone(old),
      id: 'custom-fixture',
      name: r.name!,
      revision: 1,
    };
    catalog.definitions.push(definition);
    return { ok: true, definition };
  }
  if (r.op === 'select') {
    if (r.expectedRevision !== catalog.selectionRevision)
      return { ok: false, code: 'conflict', error: 'Selection changed' };
    catalog.selectionRevision++;
    if (r.cwd) {
      if (r.workflowId === null) delete catalog.projects[r.cwd];
      else catalog.projects[r.cwd] = r.workflowId!;
    } else catalog.defaultId = r.workflowId!;
    return { ok: true };
  }
  if (r.op === 'create' || r.op === 'update') {
    if (params.has('conflict'))
      return { ok: false, code: 'conflict', error: 'Workflow changed; reload revision 2' };
    try {
      const definition = validateWorkflow(r.definition);
      if (r.op === 'update') {
        const i = catalog.definitions.findIndex((d) => d.id === r.id);
        definition.revision++;
        catalog.definitions[i] = definition;
      } else catalog.definitions.push(definition);
      return { ok: true, definition };
    } catch (e) {
      return { ok: false, code: 'invalid', error: String(e) };
    }
  }
  return { ok: false, code: 'unavailable', error: 'Fixture operation unavailable' };
};
const ipc = {
  platform: 'linux',
  ...(params.has('unavailable') ? {} : { fleetWorkflowRequest: request }),
} as unknown as ElectronAPI;
window.electronAPI = createBridgedBackend(ipc, 'fixture', `ws://${location.host}/fixture-bus`);
const definition = structuredClone(WORKFLOW_STARTERS[0]);
createRoot(document.getElementById('root')!).render(
  <main
    style={{
      padding: 12,
      maxWidth: 800,
      margin: '0 auto',
      height: '100vh',
      overflow: 'auto',
      boxSizing: 'border-box',
      fontFamily: 'var(--wks-font-sans)',
      color: 'var(--wks-text-primary)',
    }}
  >
    <ProjectWorkflowSelector cwd="/project/fixture" />
    <FleetWorkflowsSection />
    <FleetWorkflowTask
      task={{
        taskId: 'pinned-task',
        ownerSessionId: 'manager',
        ownerLabel: 'Manager',
        projectCwd: '/project/fixture',
        createdAt: '2026-09-07',
        title: 'Pinned task',
        attempts: [],
        workflow: {
          definition,
          hash: 'fixture-snapshot',
          templates: Object.fromEntries(catalog.templates.map((t) => [t.id, t])),
          steps: [
            { id: 'scout', state: 'skipped', reason: 'No unresolved material risk' },
            {
              id: 'implement',
              state: 'completed',
              sessionId: 'implementer',
              outcome: { commit: 'fixture' },
            },
            { id: 'review', state: 'planned' },
          ],
        },
      }}
    />
  </main>,
);
