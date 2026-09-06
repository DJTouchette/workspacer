import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { DispatchHistoryStore } from './dispatchHistoryStore';
import type { ClaudeSessionState } from './claudeSessionStore';
const owner = { sessionId: 'manager', isWakeTarget: true, status: 'active', label: 'Manager' };
const dirs: string[] = [];
function fixture(limits?: { tasks: number; attempts: number; bytes: number }) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dispatch-history-'));
  dirs.push(dir);
  const filename = path.join(dir, 'history.json');
  return { filename, store: new DispatchHistoryStore(() => filename, limits) };
}
const admission = { owner, projectCwd: '/project', executionCwd: '/project' };
function observation(
  sessionId: string,
  overrides = {},
): Parameters<DispatchHistoryStore['observe']>[0] {
  return {
    sessionId,
    status: 'active',
    ambientState: 'idle',
    pendingApproval: null,
    pendingQuestions: null,
    usage: null,
    ...overrides,
  } as ClaudeSessionState;
}
afterEach(() => {
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
describe('private persisted dispatch history', () => {
  it('admits only accepted live local manager dispatches and never guesses membership', () => {
    const { store } = fixture();
    for (const nonowner of [
      undefined,
      { ...owner, isWakeTarget: false },
      { ...owner, status: 'ended' },
      { ...owner, hub: 'peer' },
    ]) {
      expect(
        store.accept({ ...admission, owner: nonowner, sessionId: 'excluded' }),
      ).toBeUndefined();
    }
    store.observe(observation('unknown'));
    const first = store.accept({ ...admission, sessionId: 'one', title: 'same label' })!;
    const second = store.accept({ ...admission, sessionId: 'two', title: 'same label' })!;
    expect(second.taskId).not.toBe(first.taskId);
    expect(store.list()).toHaveLength(2);
    expect(store.list()[0].attempts[0].stage).toBeUndefined();
  });
  it('checks exact ownership/project/predecessor before launch and preserves actual order', () => {
    const { store } = fixture();
    let prior = store.accept({ ...admission, sessionId: 'scout', stage: 'scout' })!;
    const taskId = prior.taskId;
    for (const stage of ['implement', 'review', 'fix', 'validate', 'land'] as const) {
      prior = store.accept({
        ...admission,
        sessionId: stage,
        taskId,
        stage,
        afterDispatchId: prior.dispatchId,
      })!;
    }
    expect(store.list()[0].attempts.map((a) => a.stage)).toEqual([
      'scout',
      'implement',
      'review',
      'fix',
      'validate',
      'land',
    ]);
    for (const invalid of [
      { owner: { ...owner, sessionId: 'other' } },
      { projectCwd: '/other' },
      { afterDispatchId: 'not-an-attempt' },
      { stage: 'fake' },
    ]) {
      expect(() => store.validate({ ...admission, taskId, ...invalid } as never)).toThrow();
    }
  });
  it('retries are known owner-scoped attempts; resumes and replacement generations replace snapshots', () => {
    const { store } = fixture();
    const first = store.accept({
      ...admission,
      sessionId: 'one',
      stage: 'implement',
      executionCwd: '/tree',
      worktree: { requested: true, allocated: true, fallback: false },
    })!;
    const retry = store.accept({
      ...admission,
      projectCwd: '/tree',
      executionCwd: '/tree',
      sessionId: 'two',
      retrySourceSessionId: 'one',
    })!;
    expect(retry.taskId).toBe(first.taskId);
    expect(store.list()[0].attempts[1]).toMatchObject({
      kind: 'retry',
      retryOfDispatchId: first.dispatchId,
      stage: 'implement',
    });
    const afterHandoff = store.accept({
      ...admission,
      owner: { ...owner, sessionId: 'foreign' },
      sessionId: 'after-handoff',
      retrySourceSessionId: 'one',
    })!;
    expect(afterHandoff.taskId).not.toBe(first.taskId);
    expect(store.list()[0].attempts[0].kind).toBe('fresh');
    expect(store.list()[0].attempts[0].retryOfDispatchId).toBeUndefined();
    store.observe(observation('one', { statusLine: { totalInputTokens: 42, costUSD: 0 } }));
    store.observe(observation('one', { statusLine: { totalInputTokens: 42, costUSD: 0 } }));
    store.observe(observation('one', { status: 'ended' }));
    store.observe(
      observation('one', { ambientState: 'streaming', statusLine: { totalInputTokens: 45 } }),
    );
    expect(store.accept({ ...admission, sessionId: 'one' })).toEqual(first);
    const original = store.list().find((task) => task.taskId === first.taskId)!;
    expect(original.attempts).toHaveLength(2);
    expect(original.attempts[0].metrics).toMatchObject({ inputTokens: 45, costUSD: 0 });
    expect(original.attempts[0].resultContract).toBe('absent');
    store.flush();
  });
  it('lets optional stages and automatic retry provenance degrade when no manager owns the launch', () => {
    const { store } = fixture();
    const first = store.accept({ ...admission, sessionId: 'source', stage: 'implement' })!;
    expect(() =>
      store.validate({
        ...admission,
        owner: undefined,
        stage: 'review',
        retrySourceSessionId: 'source',
      }),
    ).not.toThrow();
    expect(
      store.accept({
        ...admission,
        owner: undefined,
        sessionId: 'unattributed',
        stage: 'review',
        retrySourceSessionId: 'source',
      }),
    ).toBeUndefined();
    expect(() =>
      store.validate({
        ...admission,
        owner: undefined,
        taskId: first.taskId,
        afterDispatchId: first.dispatchId,
      }),
    ).toThrow(/Task links require a live local manager/);
  });
  it('retains closed rows, unknown metrics, contract validity and exact-only stale reattachment after reload', () => {
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.observe(observation('one', { status: 'ended' }));
    expect(store.list()[0].attempts[0].resultContract).toBe('absent');
    store.validated('one', 'valid', 'opaque-evidence');
    const reloaded = new DispatchHistoryStore(() => filename);
    expect(reloaded.list()[0].attempts[0]).toMatchObject({
      stale: true,
      live: false,
      lifecycle: 'ended',
      resultContract: 'valid',
      reviewEvidenceId: 'opaque-evidence',
    });
    expect(reloaded.list()[0].attempts[0].metrics.inputTokens).toBeUndefined();
    reloaded.observe(observation('different-id'));
    expect(reloaded.list()[0].attempts[0].stale).toBe(true);
    reloaded.observe(observation('one'));
    expect(reloaded.list()[0].attempts[0]).toMatchObject({
      stale: false,
      live: true,
      lifecycle: 'idle',
    });
    reloaded.flush();
    expect(fs.statSync(filename).mode & 0o777).toBe(0o600);
  });
  it('bounds whole tasks, attempt count and bytes without inventing missing steps', () => {
    const { store, filename } = fixture({ tasks: 2, attempts: 3, bytes: 2200 });
    for (let i = 0; i < 8; i++)
      store.accept({ ...admission, sessionId: String(i), title: 'x'.repeat(300) });
    expect(store.list().length).toBeLessThanOrEqual(2);
    expect(fs.statSync(filename).size).toBeLessThanOrEqual(2200);
    expect(store.list()[0].attempts[0].sessionId).toBe('7');
    const copy = store.list();
    copy[0].attempts[0].metrics.costUSD = 99;
    expect(store.list()[0].attempts[0].metrics.costUSD).toBeUndefined();
  });
});

it('evicts whole over-limit tasks and marks still-running reload rows stale', () => {
  const { store, filename } = fixture({ tasks: 200, attempts: 2, bytes: 20000 });
  const first = store.accept({ ...admission, sessionId: 'a' })!;
  store.accept({
    ...admission,
    sessionId: 'b',
    taskId: first.taskId,
    afterDispatchId: first.dispatchId,
  });
  const fresh = new DispatchHistoryStore(() => filename);
  expect(fresh.list()[0].attempts[0]).toMatchObject({
    stale: true,
    live: false,
    lifecycle: 'starting',
  });
  store.accept({ ...admission, sessionId: 'c' });
  expect(store.list().map((t) => t.attempts.map((a) => a.sessionId))).toEqual([['c']]);
  expect(() => store.validate({ ...admission, taskId: first.taskId })).toThrow();
});

it('advances live wall time without fabricating new telemetry and freezes ended/stale duration', () => {
  const { store, filename } = fixture();
  store.accept({ ...admission, sessionId: 'clock' });
  const accepted = Date.parse(store.list()[0].attempts[0].acceptedAt);
  const now = vi.spyOn(Date, 'now').mockReturnValue(accepted + 12000);
  try {
    expect(store.list()[0].attempts[0].metrics.wallMs).toBe(12000);
    expect(store.list()[0].attempts[0].metrics.inputTokens).toBeUndefined();
    store.observe(observation('clock', { status: 'ended' }));
    store.flush();
    const ended = store.list()[0].attempts[0].metrics.wallMs;
    now.mockReturnValue(accepted + 24000);
    expect(store.list()[0].attempts[0].metrics.wallMs).toBe(ended);
    expect(new DispatchHistoryStore(() => filename).list()[0].attempts[0].metrics.wallMs).toBe(
      ended,
    );
  } finally {
    now.mockRestore();
  }
});

it('flushes an empty bounded store so eviction persists instead of resurrecting history', () => {
  const { store, filename } = fixture({ tasks: 0, attempts: 0, bytes: 20000 });
  store.accept({ ...admission, sessionId: 'evicted' });
  expect(JSON.parse(fs.readFileSync(filename, 'utf8'))).toEqual({ version: 1, tasks: [] });
  expect(new DispatchHistoryStore(() => filename).list()).toEqual([]);
});
