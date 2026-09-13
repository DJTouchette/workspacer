import { expect, it } from 'vitest';
import { intentSessionAttention, summarizeIntent } from './intentSummary';
import type { IntentWorkspace } from './intentWorkspace';
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
const workspace: IntentWorkspace = {
  id: 'work',
  projectRoot: '/project',
  revision: 2,
  title: 'Feature',
  outcome: 'Outcome',
  constraints: '',
  successCriteria: 'First criterion\nSecond criterion',
  sourceUrl: '',
  status: 'active',
  createdAt: '',
  updatedAt: '',
};
it('projects live pending slots only for qualified linked identities and deduplicates execution links', () => {
  const local = {
    ...target,
    pendingApproval: { toolName: 'Bash' },
    pendingQuestions: [{ question: 'Choose an option' }],
  };
  const peer = { ...local, hub: 'peer', pendingApproval: { toolName: 'Edit' } };
  expect(intentSessionAttention([target, target], [local, peer])).toEqual([
    { target, approval: 'Bash', questions: ['Choose an option'] },
  ]);
  expect(intentSessionAttention([target], [peer])).toEqual([]);
  for (const sample of [
    { ...local, hubOffline: true },
    { ...local, status: 'ended' },
    { ...local, status: 'stopped' },
  ])
    expect(intentSessionAttention([target], [sample])).toEqual([]);
  expect(intentSessionAttention([target], [{ ...target, ambientState: 'idle' }])).toEqual([]);
});
it('keeps user review, reported evidence and unresolved evidence distinct and scopes all coverage to current intent', () => {
  const record = {
    id: 'evidence',
    workspaceId: 'work',
    intentRevision: 2,
    criterion: { id: 'r2:c1', intentRevision: 2, text: 'First criterion' },
    kind: 'manual' as const,
    author: 'user' as const,
    assessment: 'user-verified' as const,
    note: 'Verified',
    reference: '',
    createdAt: '',
  };
  const summary = summarizeIntent(workspace, {
    evidence: {
      action: 'evidence',
      criteria: [],
      evidence: [
        record,
        { ...record, id: 'unresolved', assessment: 'unresolved' },
        {
          ...record,
          id: 'old',
          intentRevision: 1,
          criterion: { id: 'r1:c2', intentRevision: 1, text: 'Second criterion' },
        },
      ],
      reviews: [
        {
          id: 'old-review',
          workspaceId: 'work',
          intentRevision: 1,
          author: 'user',
          decision: 'accept',
          reason: 'Earlier criteria',
          evidenceIds: [],
          createdAt: '',
        },
      ],
    },
  });
  expect(summary).toMatchObject({
    verifiedCriteria: 1,
    uncoveredCriteria: 1,
    unresolvedCriteria: 1,
    review: undefined,
  });
  expect(summary.coverage[0]).toMatchObject({ verified: 1, unresolved: 1 });
});
it('does not infer review acceptance from idle execution or retained completion prose', () => {
  const summary = summarizeIntent(workspace, {
    executions: [
      {
        id: 'run',
        workspaceId: 'work',
        intentRevision: 2,
        kind: 'attached',
        state: 'linked',
        task: '',
        contextPacket: null,
        session: target,
        createdAt: '',
        updatedAt: '',
        lastObservation: {
          state: 'idle',
          summary: 'Everything is complete',
          cwd: '/project',
          observedAt: '',
        },
      },
    ],
  });
  expect(summary.review).toBeUndefined();
  expect(summary.verifiedCriteria).toBe(0);
  expect(summary.unconfirmedExecutions).toBe(0);
});
