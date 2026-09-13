import { describe, expect, it, vi } from 'vitest';
import { launchIntentExecution, type IntentRequest } from '../src/lib/intentExecution';
import type { IntentExecution, IntentWorkspace } from '../../main/shared/intentWorkspace';

const workspace: IntentWorkspace = {
  id: 'intent',
  projectRoot: '/project',
  title: 'Export',
  outcome: 'Export data',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'draft',
  revision: 2,
  createdAt: 'now',
  updatedAt: 'now',
};
const execution: IntentExecution = {
  id: 'run',
  workspaceId: 'intent',
  intentRevision: 2,
  kind: 'launch',
  state: 'launching',
  task: 'Implement',
  contextPacket: 'Exact pinned payload\nwith newlines',
  session: null,
  lastObservation: null,
  createdAt: 'now',
  updatedAt: 'now',
};
const session = {
  sessionId: 'new-session',
  hub: 'peer',
  label: 'Agent',
  provider: 'codex',
  cwd: '/project/worktree',
};
const target = { workspace, executionId: 'run' };

describe('intent launch boundary', () => {
  it('persists the claim before dispatch and sends the exact pinned first message', async () => {
    const order: string[] = [];
    const request = vi.fn<IntentRequest>().mockImplementation(async (input) => {
      order.push(input.action);
      if (input.action === 'prepareExecution')
        return { action: 'prepareExecution', execution, created: true };
      return { action: 'linkExecution', execution: { ...execution, session, state: 'linked' } };
    });
    const spawn = vi.fn(async (message, ready) => {
      order.push('spawn');
      expect(message).toBe(execution.contextPacket);
      await ready(session);
    });
    const result = await launchIntentExecution(target, 'Implement', request, spawn);
    expect(order).toEqual(['prepareExecution', 'spawn', 'linkExecution']);
    expect(request).toHaveBeenLastCalledWith({
      action: 'linkExecution',
      id: 'intent',
      executionId: 'run',
      session,
    });
    expect(result.execution.session).toEqual(session);
    expect(result.warning).toBeUndefined();
  });

  it('does not spawn when saving the launch claim fails', async () => {
    const spawn = vi.fn();
    await expect(
      launchIntentExecution(
        target,
        'Implement',
        vi.fn().mockRejectedValue(new Error('Storage unavailable')),
        spawn,
      ),
    ).rejects.toThrow('Storage unavailable');
    expect(spawn).not.toHaveBeenCalled();
  });

  it('never replays a claimed launch, including after a lost prepare response', async () => {
    const spawn = vi.fn();
    const result = await launchIntentExecution(
      target,
      'Retry',
      vi.fn().mockResolvedValue({ action: 'prepareExecution', execution, created: false }),
      spawn,
    );
    expect(spawn).not.toHaveBeenCalled();
    expect(result.warning).toContain('already recorded');
  });

  it('records uncertainty instead of turning a rejected spawn into an automatic retry', async () => {
    const request = vi
      .fn<IntentRequest>()
      .mockImplementation(async (input) =>
        input.action === 'prepareExecution'
          ? { action: 'prepareExecution', execution, created: true }
          : { action: 'markExecutionUnknown', execution: { ...execution, state: 'unknown' } },
      );
    const spawn = vi.fn().mockRejectedValue(new Error('timeout secret=not-public'));
    const result = await launchIntentExecution(target, 'Implement', request, spawn);
    expect(result.execution.state).toBe('unknown');
    expect(result.warning).not.toContain('secret');
    expect(spawn).toHaveBeenCalledTimes(1);
  });

  it('does not throw through an accepted spawn if the link write fails', async () => {
    const request = vi.fn<IntentRequest>().mockImplementation(async (input) => {
      if (input.action === 'prepareExecution')
        return { action: 'prepareExecution', execution, created: true };
      throw new Error('Storage unavailable');
    });
    const spawn = vi.fn(async (_message, ready) => {
      await expect(ready(session)).resolves.toBeUndefined();
    });
    const result = await launchIntentExecution(target, 'Implement', request, spawn);
    expect(result.warning).toContain('started');
    expect(result.warning).toContain('Link to this attempt');
    expect(spawn).toHaveBeenCalledTimes(1);
  });
});
