import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { DispatchHistoryStore, OBSERVATION_FLUSH_MS } from './dispatchHistoryStore';
import type { ClaudeSessionState } from './claudeSessionStore';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
vi.mock('../lib/atomicWriteFile', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/atomicWriteFile')>();
  return { ...actual, atomicWriteFileSync: vi.fn(actual.atomicWriteFileSync) };
});
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
  vi.restoreAllMocks();
  vi.useRealTimers();
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
describe('batched live observations', () => {
  it('reads all manager request summaries and tasks from one file version', () => {
    const { store } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.requestTransaction((requests) => {
      for (const ownerSessionId of ['manager', 'other', 'excluded'])
        requests.push({
          requestId: ownerSessionId,
          ownerSessionId,
          sourceSessionId: 'source',
          sourceCwd: '/project',
          createdAt: new Date().toISOString(),
          digest: 'a'.repeat(64),
          delivery: 'accepted',
          attempts: [],
          revision: 1,
        });
    });
    const reads = vi.spyOn(fs, 'readFileSync');
    const result = store.readForHostUser(() => undefined, ['manager', 'other']);
    expect(reads).toHaveBeenCalledTimes(1);
    expect(result.tasks).toHaveLength(1);
    expect(result.requests.map((r) => r.ownerSessionId)).toEqual(['manager', 'other']);
    expect(result.requests[0]).not.toHaveProperty('digest');
  });

  it('commits a headless lifecycle batch once and retries the whole batch after failure', () => {
    const { store, filename } = fixture();
    for (const sessionId of ['one', 'two']) store.accept({ ...admission, sessionId });
    const batch = ['one', 'two'].map((id) => observation(id, { status: 'ended' }));
    const writes = vi.mocked(atomicWriteFileSync).mockClear();
    writes.mockImplementationOnce(() => {
      throw new Error('disk unavailable');
    });
    expect(() => store.observeBatch(batch)).toThrow('disk unavailable');
    expect(store.list().every((task) => task.attempts[0].lifecycle === 'starting')).toBe(true);
    writes.mockClear();
    store.observeBatch(batch);
    expect(writes).toHaveBeenCalledTimes(1);
    expect(
      JSON.parse(fs.readFileSync(filename, 'utf8')).tasks.every(
        (task: any) => task.attempts[0].lifecycle === 'ended',
      ),
    ).toBe(true);
  });

  it('logs lock wait and write duration for an immediate lifecycle commit', () => {
    const { store } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const times = [0, 10, 30, 90, 100];
    vi.spyOn(performance, 'now').mockImplementation(() => times.shift() ?? 100);
    store.queueObservation(observation('one', { status: 'ended' }));
    expect(warn).toHaveBeenCalledWith('[dispatch-history-timing]', expect.any(String));
    expect(JSON.parse(warn.mock.calls[0][1])).toMatchObject({
      trigger: 'lifecycle',
      sessions: 1,
      durationMs: 100,
      lockWaitMs: 10,
      writeMs: 60,
      wrote: true,
      succeeded: true,
    });
  });

  it('coalesces fleet metrics into one durable write and captures mutable input', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    for (const sessionId of ['one', 'two']) {
      store.accept({ ...admission, sessionId });
      store.observe(observation(sessionId, { ambientState: 'streaming' }));
    }
    const writes = vi.mocked(atomicWriteFileSync).mockClear();
    const reads = vi.spyOn(fs, 'readFileSync');
    const statusLine = { totalOutputTokens: 0 };
    for (let i = 1; i <= 120; i++) {
      statusLine.totalOutputTokens = i;
      store.queueObservation(
        observation(i % 2 ? 'one' : 'two', {
          ambientState: 'streaming',
          statusLine,
        }),
      );
    }
    statusLine.totalOutputTokens = 999;
    expect(writes).not.toHaveBeenCalled();
    expect(reads).not.toHaveBeenCalled();
    vi.advanceTimersByTime(OBSERVATION_FLUSH_MS);
    expect(writes).toHaveBeenCalledTimes(1);
    expect(reads).toHaveBeenCalledTimes(1);
    const tasks = JSON.parse(fs.readFileSync(filename, 'utf8')).tasks;
    expect(tasks.map((t: any) => t.attempts[0].metrics.outputTokens)).toEqual([119, 120]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('ignores remote snapshots and reloads only once for a burst of untracked sessions', () => {
    vi.useFakeTimers();
    const { store } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    const reads = vi.spyOn(fs, 'readFileSync');
    const writes = vi.mocked(atomicWriteFileSync).mockClear();
    for (let i = 0; i < 120; i++) store.queueObservation(observation('remote', { hub: 'peer' }));
    expect(vi.getTimerCount()).toBe(0);
    for (let i = 0; i < 120; i++) store.queueObservation(observation('unknown'));
    expect(reads).not.toHaveBeenCalled();
    vi.advanceTimersByTime(OBSERVATION_FLUSH_MS);
    expect(reads).toHaveBeenCalledTimes(1);
    expect(writes).not.toHaveBeenCalled();
  });

  it('commits idle, approval and ended transitions immediately without a stale timer replay', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.observe(observation('one', { ambientState: 'streaming' }));
    for (const [patch, lifecycle] of [
      [{ ambientState: 'idle' }, 'idle'],
      [{ pendingApproval: { toolName: 'Bash' } }, 'needs-decision'],
      [{ status: 'ended' }, 'ended'],
    ] as const) {
      store.queueObservation(
        observation('one', { ambientState: 'streaming', statusLine: { totalOutputTokens: 42 } }),
      );
      store.queueObservation(
        observation('one', { ...patch, statusLine: { totalOutputTokens: 43 } }),
      );
      const attempt = JSON.parse(fs.readFileSync(filename, 'utf8')).tasks[0].attempts[0];
      expect(attempt.lifecycle).toBe(lifecycle);
      expect(attempt.metrics.outputTokens).toBe(43);
      expect(vi.getTimerCount()).toBe(0);
    }
  });

  it('rebases queued metrics on another writer edits and discovers newly admitted attempts', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.queueObservation(observation('two', { statusLine: { totalOutputTokens: 17 } }));
    const peer = new DispatchHistoryStore(() => filename);
    peer.accept({ ...admission, sessionId: 'two' });
    peer.requestTransaction((_requests, tasks) => {
      tasks[0].title = 'Edited elsewhere';
    });
    store.flush();
    const tasks = peer.list();
    expect(tasks.find((t) => t.attempts[0].sessionId === 'one')?.title).toBe('Edited elsewhere');
    expect(
      tasks.find((t) => t.attempts[0].sessionId === 'two')?.attempts[0].metrics.outputTokens,
    ).toBe(17);
  });

  it('retains queued observations after a failed write and drains them before explicit mutations', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.observe(observation('one', { ambientState: 'streaming' }));
    store.queueObservation(
      observation('one', { ambientState: 'streaming', statusLine: { totalOutputTokens: 99 } }),
    );
    vi.mocked(atomicWriteFileSync).mockImplementationOnce(() => {
      throw new Error('disk unavailable');
    });
    expect(() => store.flush()).toThrow('disk unavailable');
    // A final observation must win over the pending running snapshot.
    store.observe(observation('one', { status: 'ended' }));
    vi.advanceTimersByTime(OBSERVATION_FLUSH_MS);
    const attempt = JSON.parse(fs.readFileSync(filename, 'utf8')).tasks[0].attempts[0];
    expect(attempt.lifecycle).toBe('ended');
    expect(attempt.metrics.outputTokens).toBe(99);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('does not replay an older running observation over another writer final state', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.observe(observation('one', { ambientState: 'streaming' }));
    store.queueObservation(
      observation('one', { ambientState: 'streaming', statusLine: { totalOutputTokens: 10 } }),
    );
    vi.setSystemTime(Date.now() + 10);
    const peer = new DispatchHistoryStore(() => filename);
    peer.observe(observation('one', { status: 'ended', statusLine: { totalOutputTokens: 20 } }));
    store.flush();
    const attempt = JSON.parse(fs.readFileSync(filename, 'utf8')).tasks[0].attempts[0];
    expect(attempt.lifecycle).toBe('ended');
    expect(attempt.metrics.outputTokens).toBe(20);
  });

  it('retries a failed background commit without losing its observation timestamp', () => {
    vi.useFakeTimers();
    const { store, filename } = fixture();
    store.accept({ ...admission, sessionId: 'one' });
    store.observe(observation('one', { ambientState: 'streaming' }));
    const observedAt = new Date().toISOString();
    store.queueObservation(
      observation('one', { ambientState: 'streaming', statusLine: { totalOutputTokens: 77 } }),
    );
    vi.mocked(atomicWriteFileSync).mockImplementationOnce(() => {
      throw new Error('disk unavailable');
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.advanceTimersByTime(OBSERVATION_FLUSH_MS);
    expect(warn).toHaveBeenCalled();
    vi.advanceTimersByTime(OBSERVATION_FLUSH_MS);
    const attempt = JSON.parse(fs.readFileSync(filename, 'utf8')).tasks[0].attempts[0];
    expect(attempt.observedAt).toBe(observedAt);
    expect(attempt.metrics.outputTokens).toBe(77);
    expect(vi.getTimerCount()).toBe(0);
  });
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

it('untracked dispatch needs no task store and cannot hide conflicting task links', () => {
  const { store, filename } = fixture();
  fs.writeFileSync(filename, 'unavailable history');
  expect(store.accept({ ...admission, sessionId: 'adhoc', trackTask: false })).toBeUndefined();
  expect(fs.readFileSync(filename, 'utf8')).toBe('unavailable history');
  for (const link of [
    { taskId: 'foreign' },
    { workflowStepId: 'review' },
    { afterDispatchId: 'old' },
    { retrySourceSessionId: 'old' },
  ]) {
    expect(() => store.validate({ ...admission, ...link, trackTask: false })).toThrow(
      'Untracked dispatch must omit',
    );
  }
  expect(() => store.validate({ ...admission, trackTask: 'false' as any })).toThrow(
    'trackTask must be boolean',
  );
});
