import { DatabaseSync } from 'node:sqlite';
import { afterEach, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-completion-tests' }));
import { IntentWorkspaceStore, captureIntentSessions } from './intentWorkspaceStore';
import type { IntentLiveSession } from '../shared/intentWorkspace';
import type { IntentAutomationEffects } from './intentAutomationStore';

const databases: DatabaseSync[] = [];
const dirs: string[] = [];
afterEach(() => {
  for (const db of databases.splice(0)) {
    try {
      db.close();
    } catch {}
  }
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
async function fixture(file = ':memory:') {
  const db = new DatabaseSync(file);
  databases.push(db);
  const store = new IntentWorkspaceStore(db);
  const created = store.request({
    action: 'create',
    projectRoot: '/project',
    fields: {
      title: 'Outcome',
      outcome: 'Implement export',
      constraints: '',
      successCriteria: 'Export works',
      sourceUrl: '',
      status: 'draft',
    },
  });
  if (created.action !== 'create') throw new Error('create');
  const id = created.workspace.id;
  store.request({ action: 'activateIntent', id, expectedRevision: 1 });
  const session = {
    sessionId: 'manager',
    hub: '',
    cwd: '/project',
    label: 'Manager',
    provider: 'codex',
    status: 'active',
    ambientState: 'idle',
  };
  const effects: IntentAutomationEffects = {
    spawn: vi.fn(async () => session),
    send: vi.fn(async () => ({ status: 'accepted', detail: 'sent' })),
    interrupt: vi.fn(async () => ({ status: 'accepted', detail: 'interrupted' })),
  };
  await store.automation.tick([], effects);
  const run = store.automation.get(id)!;
  const content =
    'Verbatim report.\n```intent-report\n' +
    JSON.stringify({
      runId: run.id,
      revision: 1,
      state: 'review',
      summary: 'Implemented export',
      checks: ['vitest: 12 passed'],
      artifacts: ['export.ts'],
      caveats: ['No live provider tested'],
      followUps: [],
    }) +
    '\n```';
  const live = { ...session, conversation: [{ role: 'assistant', content }] };
  const observe = (patch: Partial<IntentLiveSession> = {}, children: IntentLiveSession[] = []) =>
    store.capture(captureIntentSessions([{ ...live, ...patch }, ...children]));
  const proposal = () =>
    store.completions
      .view(id)
      .proposals.find((p) => p.id === store.completions.view(id).currentProposalId)!;
  const verify = () =>
    store.request({
      action: 'addEvidence',
      id,
      expectedRevision: 1,
      evidenceId: 'verified',
      criterionId: 'r1:c1',
      note: 'I inspected export output',
      reference: '',
      assessment: 'user-verified',
    });
  const request = (decision = 'accept') => ({
    action: 'recordReview',
    id,
    expectedRevision: 1,
    proposalId: proposal()?.id,
    reviewId: 'review-operation',
    decision,
    reason: 'Human review',
    evidenceIds: decision === 'accept' ? ['verified'] : [],
  });
  return { db, store, id, run, live, effects, observe, proposal, verify, request };
}
it.each(['thinking', 'streaming', 'background', 'approval', 'waiting-input', 'unknown'])(
  'does not complete %s even with a valid agent assertion',
  async (state) => {
    const f = await fixture();
    f.observe({ ambientState: state });
    expect(f.proposal()).toBeUndefined();
    expect(f.store.automation.get(f.id)?.state).not.toBe('review');
  },
);
it.each([
  { status: 'ended' },
  { status: 'stopped' },
  { pendingApproval: { toolName: 'Bash' } },
  { pendingQuestions: [{ question: 'Allow?' }] },
  { subagents: [{ status: 'running' }] },
  { activeToolCalls: [{}] },
  { hubOffline: true },
])('does not approve unavailable, blocked or background work: %j', async (patch) => {
  const f = await fixture();
  f.observe(patch);
  expect(f.proposal()).toBeUndefined();
});
it('requires descendant drain and rejects interrupted and ordinary idle replies', async () => {
  const f = await fixture();
  f.observe({}, [
    { ...f.live, sessionId: 'child', parentSessionId: 'manager', ambientState: 'background' },
  ]);
  expect(f.proposal()).toBeUndefined();
  f.observe({
    conversation: [
      ...f.live.conversation,
      { role: 'user', content: '[Request interrupted by user]' },
    ],
  });
  expect(f.proposal()).toBeUndefined();
  f.observe({ conversation: [{ role: 'assistant', content: 'I will continue shortly.' }] });
  expect(f.proposal()).toMatchObject({ completedAt: null, reportState: 'malformed' });
  expect(f.store.automation.get(f.id)?.state).toBe('working');
  f.observe();
  expect(f.proposal()).toMatchObject({
    reportState: 'reported',
    report: f.live.conversation[0].content,
    checks: ['vitest: 12 passed'],
  });
});
it.each([
  ['', 'missing'],
  ['```intent-report\n{bad}\n```', 'malformed'],
  ['x'.repeat(5000), 'oversized'],
])('persists explicit bounded report state %s', async (text, state) => {
  const f = await fixture();
  f.observe({ conversation: [{ role: 'assistant', content: text }] });
  expect(f.proposal()).toMatchObject({ reportState: state, completedAt: null });
  expect(f.proposal().report.length).toBeLessThanOrEqual(4000);
  f.verify();
  expect(() => f.store.request(f.request(), [f.live])).toThrow();
});
it('redacts credentials without importing user or tool transcript text', async () => {
  const f = await fixture();
  f.observe({
    conversation: [
      { role: 'user', content: 'USER SECRET' },
      { role: 'tool', content: 'TOOL SECRET' },
      { role: 'assistant', content: 'Bearer abc123.secret\n' + f.live.conversation[0].content },
    ],
  });
  const p = f.proposal();
  expect(p.redacted).toBe(true);
  expect(JSON.stringify(p)).not.toMatch(/abc123|USER SECRET|TOOL SECRET/);
});
it('persists an immutable proposal over duplicate samples and restart', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-proposal-'));
  dirs.push(dir);
  const file = path.join(dir, 'state.sqlite');
  const f = await fixture(file);
  f.observe();
  const proposal = f.proposal();
  f.observe();
  expect(f.store.completions.view(f.id).proposals).toHaveLength(1);
  f.db.close();
  const db = new DatabaseSync(file);
  databases.push(db);
  const restarted = new IntentWorkspaceStore(db);
  expect(restarted.completions.view(f.id).proposals).toEqual([proposal]);
});
it('atomically gates evidence, proposal and live lifecycle; exact replay records acceptance only once', async () => {
  const f = await fixture();
  f.observe();
  const request = f.request();
  expect(() => f.store.request({ ...request, evidenceIds: [] }, [f.live])).toThrow('user-verified');
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_reviews').get()?.n).toBe(0);
  f.verify();
  expect(() => f.store.request(request, [{ ...f.live, status: 'ended' }])).toThrow('unavailable');
  f.store.request(request, [f.live]);
  f.store.request(request, []);
  expect(f.store.automation.get(f.id)?.state).toBe('complete');
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ status: 'complete', revision: 1 }],
  });
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_reviews').get()?.n).toBe(1);
  expect(() => f.store.request({ ...request, reason: 'Different replay' }, [f.live])).toThrow();
});
it.each(['accepted', 'failed', 'unknown'] as const)(
  'changes requested preserves %s direction receipt and never repeats delivery',
  async (status) => {
    const f = await fixture();
    f.observe();
    const request = f.request('changes-requested');
    f.store.request(request);
    const next = f.store.automation.get(f.id)!;
    expect(next.id).not.toBe(f.run.id);
    f.effects.send = vi.fn(async () => ({ status, detail: 'Synthetic transport outcome' }));
    await f.store.automation.tick([f.live], f.effects);
    f.store.request(request);
    await f.store.automation.tick([f.live], f.effects);
    expect(f.effects.send).toHaveBeenCalledOnce();
    expect(f.store.request({ action: 'directions', id: f.id })).toMatchObject({
      directions: [{ id: next.id, attempts: [{ status }] }],
    });
    expect(f.store.request({ action: 'evidence', id: f.id })).toMatchObject({
      reviews: [{ proposalId: request.proposalId, directionId: next.id }],
    });
    expect(f.store.completions.view(f.id).currentProposalId).toBeNull();
  },
);
it('retains feedback when the session cannot resume', async () => {
  const f = await fixture();
  f.observe();
  f.store.request(f.request('changes-requested'));
  await f.store.automation.tick([], f.effects);
  expect(f.effects.send).not.toHaveBeenCalled();
  expect(f.store.automation.get(f.id)).toMatchObject({
    state: 'paused',
    message: expect.stringContaining('Human review'),
    report: expect.stringContaining('unavailable'),
  });
});
it('supersedes proposals on requirement revision, new execution and stale CAS', async () => {
  const f = await fixture();
  f.observe();
  f.verify();
  const request = f.request();
  expect(() => f.store.request({ ...request, expectedRevision: 2 }, [f.live])).toThrow('changed');
  const result = f.store.request({ action: 'list' });
  if (result.action !== 'list') throw new Error('list');
  f.store.request({
    action: 'update',
    id: f.id,
    expectedRevision: 1,
    fields: { ...result.workspaces[0], successCriteria: 'Unicode export works', status: 'draft' },
    reason: 'Changed requirements',
  });
  expect(f.store.completions.view(f.id).currentProposalId).toBeNull();
  expect(f.store.completions.view(f.id).proposals).toHaveLength(1);
  expect(() => f.store.request(request, [f.live])).toThrow();
});

it('does not restore approval eligibility from a sparse idle snapshot after interruption', async () => {
  const f = await fixture();
  f.observe();
  f.verify();
  const request = f.request();
  f.observe({
    conversation: [
      ...f.live.conversation,
      { role: 'user', content: '[Request interrupted by user]' },
    ],
  });
  const { conversation: _conversation, ...sparse } = f.live;
  f.store.capture(captureIntentSessions([sparse]));
  expect(() => f.store.request(request, [sparse])).toThrow('current completed');
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_reviews').get()?.n).toBe(0);
});
it('blocks acceptance while a separate direction delivery is unknown', async () => {
  const f = await fixture();
  f.observe();
  f.verify();
  f.store.request({
    action: 'prepareDirection',
    id: f.id,
    expectedRevision: 1,
    directionId: 'uncertain',
    executionId: f.run.executionId,
    text: 'Check one more edge case',
  });
  await f.store.steering.send(
    { action: 'sendDirection', id: f.id, directionId: 'uncertain', attemptId: 'attempt' },
    [f.live],
    async () => ({ status: 'unknown', detail: 'No receipt' }),
  );
  expect(() => f.store.request(f.request(), [f.live])).toThrow('delivery is unknown');
});
it('a newer execution supersedes the earlier proposal without rewriting it', async () => {
  const f = await fixture();
  f.observe();
  const before = f.proposal();
  f.store.request({
    action: 'prepareExecution',
    id: f.id,
    expectedRevision: 1,
    executionId: 'new-execution',
    task: 'New execution',
  });
  expect(f.store.completions.view(f.id).currentProposalId).toBeNull();
  expect(f.store.completions.view(f.id).proposals).toEqual([before]);
});
it('retains a valid summary while explicitly omitting malformed structured fields', async () => {
  const f = await fixture();
  f.observe({
    conversation: [
      {
        role: 'assistant',
        content:
          '```intent-report\n' +
          JSON.stringify({
            runId: f.run.id,
            revision: 1,
            state: 'review',
            summary: 'Ready',
            checks: 'passed',
            artifacts: ['export.ts'],
          }) +
          '\n```',
      },
    ],
  });
  expect(f.proposal()).toMatchObject({
    reportState: 'reported',
    structuredState: 'malformed',
    artifacts: ['export.ts'],
  });
  expect(f.proposal().checks).toBeUndefined();
});
