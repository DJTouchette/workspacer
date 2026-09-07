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
