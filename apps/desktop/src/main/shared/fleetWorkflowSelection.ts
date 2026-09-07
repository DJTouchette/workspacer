import { DEFAULT_WORKFLOW_ID } from './fleetWorkflow';
type SelectionConfig = {
  agents?: { defaultWorkflowId?: string; workflowSelectionRevision?: number };
  projects?: Record<string, { workflowId?: string }>;
};
export function workflowSelections(config: SelectionConfig): {
  defaultId: string;
  projects: Record<string, string>;
  selectionRevision: number;
} {
  return {
    defaultId: config.agents?.defaultWorkflowId ?? DEFAULT_WORKFLOW_ID,
    selectionRevision: config.agents?.workflowSelectionRevision ?? 0,
    projects: Object.fromEntries(
      Object.entries(config.projects ?? {})
        .filter(([, p]) => p.workflowId !== undefined)
        .map(([cwd, p]) => [cwd, p.workflowId!]),
    ),
  };
}
/** Generic config writes preserve selections. Only the revision-checked service can change them. */
export function preserveWorkflowSelections(
  current: SelectionConfig,
  partial: SelectionConfig,
): void {
  if (
    partial.agents != null &&
    (typeof partial.agents !== 'object' || Array.isArray(partial.agents)) &&
    (current.agents?.defaultWorkflowId !== undefined ||
      current.agents?.workflowSelectionRevision !== undefined)
  )
    throw new Error('Workflow selections cannot be removed by replacing agents');
  if (partial.agents) {
    for (const key of ['defaultWorkflowId', 'workflowSelectionRevision'] as const) {
      if (key in partial.agents && partial.agents[key] !== current.agents?.[key])
        throw new Error('Use the Fleet workflow selection API with expectedRevision');
    }
  }
  if (partial.projects)
    for (const [cwd, p] of Object.entries(partial.projects)) {
      const old = current.projects?.[cwd]?.workflowId;
      if (old !== undefined && (!p || typeof p !== 'object' || Array.isArray(p)))
        throw new Error('Set project workflow to inherit before replacing the selected project');
      if (p.workflowId !== undefined && p.workflowId !== old)
        throw new Error('Use the Fleet workflow selection API with expectedRevision');
      if (old !== undefined) p.workflowId = old;
    }
  if (partial.projects)
    for (const [cwd, p] of Object.entries(current.projects ?? {}))
      if (p.workflowId !== undefined && !partial.projects[cwd])
        throw new Error('Set project workflow to inherit before removing the selected project');
}
