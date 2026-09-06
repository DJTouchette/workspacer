import { TASK_STAGES, type DispatchTask } from '../../../main/shared/dispatchHistory';
export function recentAgentsFixture(): DispatchTask[] {
  const at = '2026-09-06T12:00:00Z';
  const loop: DispatchTask = {
    taskId: 'loop',
    title: 'Ship the full parser task loop with a deliberately long descriptive title',
    ownerSessionId: 'manager',
    ownerLabel: 'Current manager',
    projectCwd: '/project/alpha',
    createdAt: at,
    attempts: [],
  };
  loop.attempts = [
    ...TASK_STAGES.slice(0, 6),
    ...Array.from({ length: 12 }, () => 'fix' as const),
  ].map((stage, i) => ({
    dispatchId: `attempt-${i}`,
    sessionId: `worker-${i}`,
    stage,
    kind: i > 5 ? 'retry' : 'fresh',
    retryOfDispatchId: i > 5 ? `attempt-${i - 1}` : undefined,
    afterDispatchId: i ? `attempt-${i - 1}` : undefined,
    acceptedAt: at,
    observedAt: at,
    executionCwd: `/project/alpha/worktrees/long-isolated-worktree-name-${i}`,
    lifecycle: i === 0 ? 'running' : 'ended',
    live: i === 0,
    stale: i > 5,
    requestedProvider: 'codex',
    provider: 'codex',
    requestedModel: 'requested-model',
    reportedModel: i % 2 ? 'reported-model' : undefined,
    role: 'implementer',
    resultContract: i === 4 ? 'valid' : 'absent',
    worktree: { requested: true, allocated: true, fallback: false, branch: `wks/task-${i}` },
    metrics:
      i % 2
        ? {}
        : {
            inputTokens: 1000 + i,
            outputTokens: 100,
            costUSD: 0.03,
            wallMs: 9500,
            context: { used: 12000, limit: 200000, observedAt: at },
          },
    reviewEvidenceId: i === 0 ? 'evidence' : undefined,
  }));
  const newest: DispatchTask = {
    taskId: 'standalone',
    title: 'Unclassified standalone task',
    ownerSessionId: 'manager',
    ownerLabel: 'Current manager',
    projectCwd: '/project/beta',
    createdAt: '2026-09-06T13:00:00Z',
    attempts: [
      {
        ...loop.attempts[0],
        dispatchId: 'standalone-attempt',
        sessionId: 'standalone',
        stage: undefined,
        metrics: {},
        reviewEvidenceId: undefined,
        lifecycle: 'idle',
      },
    ],
  };
  const other: DispatchTask = {
    ...newest,
    taskId: 'other',
    title: 'Another manager task',
    ownerSessionId: 'old-manager',
    ownerLabel: 'Stopped manager',
    attempts: [
      {
        ...newest.attempts[0],
        dispatchId: 'other-attempt',
        reviewEvidenceId: 'foreign-evidence',
        live: false,
        stale: true,
      },
    ],
  };
  return [newest, loop, other];
}
