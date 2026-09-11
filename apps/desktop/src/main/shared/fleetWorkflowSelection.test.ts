import { DEFAULT_WORKFLOW_ID, WORKFLOW_STARTERS } from './fleetWorkflow';
import { describe, it, expect } from 'vitest';
import { preserveWorkflowSelections, workflowSelections } from './fleetWorkflowSelection';
describe('workflow selection guard on generic config saves', () => {
  const current = {
    agents: { defaultWorkflowId: 'custom', workflowSelectionRevision: 3 },
    projects: { '/repo': { workflowId: 'research' } },
  };
  it('preserves project selection through wholesale identity edits', () => {
    const patch = { projects: { '/repo': { label: 'New label' } } };
    preserveWorkflowSelections(current, patch as never);
    expect(workflowSelections(patch as never).projects).toEqual({ '/repo': 'research' });
  });
  it('rejects changes, removals and type replacements without revision authority', () => {
    for (const patch of [
      { agents: { defaultWorkflowId: 'other' } },
      { agents: { workflowSelectionRevision: 0 } },
      { agents: [] },
      { agents: false },
      { projects: {} },
      { projects: { '/repo': [] } },
      { projects: { '/repo': null } },
      { projects: { '/repo': { workflowId: 'other' } } },
    ])
      expect(() => preserveWorkflowSelections(current, patch as never)).toThrow();
  });
});

describe('default fleet workflow', () => {
  it('starts with implementer-led discovery and retains an independent review', () => {
    expect(workflowSelections({}).defaultId).toBe('implement-review');
    const workflow = WORKFLOW_STARTERS.find((d) => d.id === DEFAULT_WORKFLOW_ID)!;
    expect(workflow.steps.map((s) => s.id)).toEqual(['implement', 'review']);
    expect(workflow.steps[0].instructions).toContain('brief discovery');
    expect(workflow.steps[1]).toMatchObject({
      when: 'always',
      role: 'reviewer',
      independentOf: 'implement',
    });
  });
  it('preserves explicit selections and the optional scout workflow', () => {
    expect(
      workflowSelections({
        agents: { defaultWorkflowId: 'scout-implement-review' },
        projects: { '/repo': { workflowId: 'custom' } },
      }),
    ).toMatchObject({
      defaultId: 'scout-implement-review',
      projects: { '/repo': 'custom' },
    });
    expect(
      WORKFLOW_STARTERS.find((d) => d.id === 'scout-implement-review')!.steps[0],
    ).toMatchObject({ id: 'scout', when: 'material_risk' });
  });
});
