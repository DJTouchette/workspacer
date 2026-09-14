import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-automation-tests' }));
import { IntentWorkspaceStore, captureIntentSessions } from './intentWorkspaceStore';
import type { IntentAutomationEffects } from './intentAutomationStore';
import type { IntentRun } from '../shared/intentAutomation';
import type { IntentWorkspace } from '../shared/intentWorkspace';
const databases: DatabaseSync[] = [],
  roots: string[] = [];
afterEach(() => {
  for (const db of databases.splice(0)) if (db.isOpen) db.close();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  vi.useRealTimers();
});
function open(file = ':memory:') {
  const db = new DatabaseSync(file);
  databases.push(db);
  return { db, store: new IntentWorkspaceStore(db) };
}
const session = {
  sessionId: 'manager',
  hub: '',
  cwd: '/project',
  label: 'Intent manager',
  provider: 'codex',
};
const live = [{ ...session, ambientState: 'idle', status: 'active' }];
function fixture(file?: string) {
  const { db, store } = open(file);
  const result = store.request({
    action: 'create',
    projectRoot: '/project',
    fields: {
      title: 'Export',
      outcome: 'Export filtered rows',
      constraints: 'No publishing',
      successCriteria: 'Valid CSV',
      sourceUrl: '',
      status: 'draft',
    },
  });
  if (result.action !== 'create') throw new Error('create');
  const effects: IntentAutomationEffects = {
    spawn: vi.fn(async () => session),
    send: vi.fn(async () => ({ status: 'accepted' as const, detail: 'accepted' })),
    interrupt: vi.fn(async () => ({ status: 'accepted', detail: 'accepted' })),
  };
  return { db, store, workspace: result.workspace, effects };
}
function activate(store: IntentWorkspaceStore, workspace: IntentWorkspace) {
  const result = store.request({
    action: 'activateIntent',
    id: workspace.id,
    expectedRevision: workspace.revision,
  });
  if (result.action !== 'activateIntent' || !result.run) throw new Error('activation');
  return result.run;
}
function report(store: IntentWorkspaceStore, run: IntentRun, state: string, summary: string) {
  store.capture(
    captureIntentSessions([
      {
        ...session,
        ambientState: 'idle',
        conversation: [
          {
            role: 'assistant',
            content:
              '```intent-report\n' +
              JSON.stringify({ runId: run.id, revision: run.intentRevision, state, summary }) +
              '\n```',
          },
        ],
      },
    ]),
  );
}
it('activates once, asks a question, resumes the same manager and hands off to human review', async () => {
  const f = fixture();
  const run = activate(f.store, f.workspace);
  await Promise.all([
    f.store.automation.tick(live, f.effects),
    f.store.automation.tick(live, f.effects),
  ]);
  expect(f.effects.spawn).toHaveBeenCalledTimes(1);
  expect(activate(f.store, f.workspace).id).toBe(run.id);
  expect(f.effects.spawn).toHaveBeenCalledWith(
    '/project',
    'Intent: Export',
    expect.stringContaining(run.id),
  );
  report(f.store, run, 'waiting', 'All rows or current page? I recommend all filtered rows.');
  expect(f.store.automation.get(f.workspace.id)).toMatchObject({
    state: 'waiting',
    report: expect.stringContaining('All rows'),
  });
  const answer = f.store.request({
    action: 'answerIntent',
    id: f.workspace.id,
    runId: run.id,
    expectedRevision: 1,
    text: 'All filtered rows',
  });
  if (answer.action !== 'answerIntent' || !answer.run) throw new Error('answer');
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledTimes(1);
  expect(f.effects.spawn).toHaveBeenCalledTimes(1);
  report(f.store, run, 'review', 'Stale completion');
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('working');
  report(
    f.store,
    answer.run,
    'review',
    'Implemented CSV export. 12 tests passed. Check the saved diff.',
  );
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ status: 'review', revision: 1 }],
  });
  expect(f.store.request({ action: 'directions', id: f.workspace.id })).toMatchObject({
    directions: [{ attempts: [{ status: 'accepted' }] }],
  });
  expect(() =>
    f.store.request(
      {
        action: 'recordReview',
        id: f.workspace.id,
        expectedRevision: 1,
        proposalId: f.store.completions.view(f.workspace.id).currentProposalId,
        reviewId: 'accept',
        decision: 'accept',
        reason: 'Looks good',
        evidenceIds: [],
      },
      live,
    ),
  ).toThrow('user-verified');
  f.store.request({
    action: 'addEvidence',
    id: f.workspace.id,
    expectedRevision: 1,
    evidenceId: 'verified',
    criterionId: 'r1:c1',
    assessment: 'user-verified',
    note: 'Opened export and checked rows',
    reference: '',
  });
  f.store.request(
    {
      action: 'recordReview',
      id: f.workspace.id,
      expectedRevision: 1,
      proposalId: f.store.completions.view(f.workspace.id).currentProposalId,
      reviewId: 'accept',
      decision: 'accept',
      reason: 'Verified CSV',
      evidenceIds: ['verified'],
    },
    live,
  );
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ status: 'complete', revision: 1 }],
  });
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('complete');
});
it('saves Active as activation and preserves evidence across status changes', async () => {
  const f = fixture();
  f.store.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, status: 'active' },
    reason: 'Start',
  });
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('queued');
  await f.store.automation.tick(live, f.effects);
  f.store.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, status: 'review' },
    reason: 'Review',
  });
  expect(f.store.request({ action: 'history', id: f.workspace.id })).toMatchObject({
    revisions: [{ revision: 1 }],
    statusEvents: expect.arrayContaining([expect.objectContaining({ status: 'review' })]),
  });
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ revision: 1, status: 'review' }],
  });
});
it('never replays uncertain launches across connections and restart', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'intent-run-'));
  roots.push(root);
  const file = path.join(root, 'work.sqlite');
  const f = fixture(file);
  activate(f.store, f.workspace);
  f.effects.spawn = vi.fn(async () => {
    throw new Error('lost ACK');
  });
  await f.store.automation.tick(live, f.effects);
  f.db.close();
  const reopened = open(file).store;
  await reopened.automation.tick(live, f.effects);
  expect(f.effects.spawn).toHaveBeenCalledTimes(1);
  expect(() => activate(reopened, f.workspace)).toThrow('unconfirmed');
  const run = reopened.automation.get(f.workspace.id)!;
  reopened.request({
    action: 'linkExecution',
    id: f.workspace.id,
    executionId: run.executionId,
    session,
  });
  expect(reopened.automation.get(f.workspace.id)).toMatchObject({ state: 'paused', session });
  activate(reopened, f.workspace);
  await reopened.automation.tick(live, f.effects);
  expect(f.effects.spawn).toHaveBeenCalledTimes(1);
  expect(f.effects.send).toHaveBeenCalledTimes(1);
});
it('handles a pause during spawn and does not launch another manager', async () => {
  const f = fixture();
  const run = activate(f.store, f.workspace);
  let finish!: (value: typeof session) => void;
  f.effects.spawn = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const starting = f.store.automation.tick(live, f.effects);
  f.store.request({ action: 'pauseIntent', id: f.workspace.id, runId: run.id });
  finish(session);
  await starting;
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.interrupt).toHaveBeenCalledOnce();
  expect(f.store.automation.get(f.workspace.id)).toMatchObject({
    state: 'paused',
    session,
    needsInterrupt: false,
  });
});
it('keeps review feedback and continuation durable without re-running accepted reviews', async () => {
  const f = fixture();
  const run = activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  report(f.store, run, 'review', 'Ready');
  const request = {
    action: 'recordReview',
    id: f.workspace.id,
    expectedRevision: 1,
    proposalId: f.store.completions.view(f.workspace.id).currentProposalId,
    reviewId: 'changes',
    decision: 'changes-requested',
    reason: 'Handle Unicode',
    evidenceIds: [],
  };
  f.store.request(request);
  await f.store.automation.tick(live, f.effects);
  f.store.request(request);
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledOnce();
  expect(f.effects.send).toHaveBeenCalledWith(session, expect.stringContaining('Handle Unicode'));
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ status: 'active', revision: 1 }],
  });
});
it('pauses at the time limit and preserves uncertain continuation receipts', async () => {
  vi.useFakeTimers();
  const f = fixture();
  const run = activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  vi.setSystemTime(Date.parse(run.deadline) + 1);
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.interrupt).toHaveBeenCalledOnce();
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('paused');
  activate(f.store, f.workspace);
  f.effects.send = vi.fn(async () => ({ status: 'unknown', detail: 'Lost acknowledgment' }));
  await f.store.automation.tick(live, f.effects);
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledOnce();
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('uncertain');
});
it('does not mistake ordinary idle reports or other sessions for completion', async () => {
  const f = fixture();
  activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  f.store.capture(
    captureIntentSessions([
      { ...session, ambientState: 'idle', conversation: [{ role: 'assistant', content: 'Done!' }] },
    ]),
  );
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('working');
  f.store.capture(
    captureIntentSessions([
      { ...session, pendingQuestions: [{ question: 'Which export?' }], ambientState: 'idle' },
    ]),
  );
  expect(f.store.automation.get(f.workspace.id)).toMatchObject({
    state: 'waiting',
    report: 'Which export?',
  });
});

it('continues an idle manager only after its workers finish, with a bounded backstop', async () => {
  vi.useFakeTimers();
  const f = fixture();
  activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  const idleReport = () =>
    f.store.capture(
      captureIntentSessions([
        {
          ...session,
          ambientState: 'idle',
          conversation: [{ role: 'assistant', content: 'I have a partial implementation.' }],
        },
      ]),
    );
  idleReport();
  vi.advanceTimersByTime(11_000);
  await f.store.automation.tick(
    [
      ...live,
      { sessionId: 'worker', parentSessionId: session.sessionId, ambientState: 'streaming' },
    ],
    f.effects,
  );
  expect(f.effects.send).not.toHaveBeenCalled();
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledOnce();
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledOnce();
  for (let i = 0; i < 3; i++) {
    f.store.capture(
      captureIntentSessions([
        {
          ...session,
          ambientState: 'idle',
          conversation: [{ role: 'assistant', content: `Partial implementation ${i}` }],
        },
      ]),
    );
    vi.advanceTimersByTime(11_000);
    await f.store.automation.tick(live, f.effects);
  }
  expect(f.effects.send).toHaveBeenCalledTimes(3);
  expect(f.store.automation.get(f.workspace.id)?.state).toBe('waiting');
});
it('sends saved requirement changes while retaining the exact original kickoff and revision history', async () => {
  const f = fixture();
  const run = activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  const initial = f.store.request({ action: 'executions', id: f.workspace.id });
  if (initial.action !== 'executions') throw new Error('executions');
  expect(f.effects.spawn).toHaveBeenCalledWith(
    '/project',
    'Intent: Export',
    initial.executions[0].contextPacket,
  );
  f.store.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, status: 'active', outcome: 'Export all filtered Unicode rows' },
    reason: 'Expanded requirement',
  });
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.spawn).toHaveBeenCalledOnce();
  expect(f.effects.send).toHaveBeenCalledWith(session, expect.stringContaining('Unicode rows'));
  report(f.store, run, 'review', 'Old revision ready');
  expect(f.store.request({ action: 'list' })).toMatchObject({
    workspaces: [{ revision: 2, status: 'active' }],
  });
  expect(f.store.request({ action: 'executions', id: f.workspace.id })).toMatchObject({
    executions: [{ contextPacket: initial.executions[0].contextPacket, intentRevision: 1 }],
  });
});
it('requires a confirmed stopped manager before explicitly starting a replacement', async () => {
  const f = fixture();
  const run = activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  f.store.request({ action: 'pauseIntent', id: f.workspace.id, runId: run.id });
  await f.store.automation.tick(live, f.effects);
  const restart = {
    action: 'restartIntent',
    id: f.workspace.id,
    runId: run.id,
    expectedRevision: 1,
  };
  expect(() => f.store.request(restart, live)).toThrow('confirmed stopped');
  f.store.capture(captureIntentSessions([{ ...session, status: 'ended' }]));
  f.store.request(restart, [{ ...session, status: 'ended' }]);
  await f.store.automation.tick([], f.effects);
  expect(f.effects.spawn).toHaveBeenCalledTimes(2);
  expect(f.effects.spawn).toHaveBeenLastCalledWith(
    '/project',
    'Intent: Export',
    expect.stringContaining('previous manager session manager ended'),
  );
});

it('retains an uncertain receipt when the user explicitly resumes with a new inspected direction', async () => {
  const f = fixture();
  const initial = activate(f.store, f.workspace);
  await f.store.automation.tick(live, f.effects);
  report(f.store, initial, 'waiting', 'Choose export format');
  f.store.request({
    action: 'answerIntent',
    id: f.workspace.id,
    runId: initial.id,
    expectedRevision: 1,
    text: 'CSV',
  });
  f.effects.send = vi.fn(async () => ({ status: 'unknown', detail: 'No acknowledgment' }));
  await f.store.automation.tick(live, f.effects);
  const uncertain = f.store.automation.get(f.workspace.id)!;
  expect(() => activate(f.store, f.workspace)).toThrow('unconfirmed');
  f.store.request({
    action: 'resumeInspectedIntent',
    id: f.workspace.id,
    runId: uncertain.id,
    expectedRevision: 1,
    text: 'I saw the CSV instruction arrive. Continue checking its output.',
  });
  f.effects.send = vi.fn(async () => ({ status: 'accepted', detail: 'accepted' }));
  await f.store.automation.tick(live, f.effects);
  expect(f.effects.send).toHaveBeenCalledWith(
    session,
    expect.stringContaining('I saw the CSV instruction arrive'),
  );
  const archived = f.db
    .prepare('SELECT snapshot FROM intent_run_history WHERE id=?')
    .get(uncertain.id);
  expect(JSON.parse(String(archived?.snapshot))).toMatchObject({
    state: 'uncertain',
    operation: 'send',
  });
  const directions = f.store.request({ action: 'directions', id: f.workspace.id });
  if (directions.action !== 'directions') throw new Error('directions');
  expect(directions.directions.find((d) => d.id === uncertain.id)?.attempts[0].status).toBe(
    'unknown',
  );
});
