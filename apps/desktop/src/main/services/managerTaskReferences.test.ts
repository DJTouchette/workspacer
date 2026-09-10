/**
 * The manager-facing task reference capability.
 *
 * It is deliberately narrower than the host-user Inspector edit: additive
 * upsert/remove of exact reference entries on a task the caller actually owns,
 * under the task-row revision CAS. It must never become a way for an agent to
 * waive a step, edit another manager's task, replace the human's entries, or
 * launder an unvalidated URL into the store.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { DispatchHistoryStore } from './dispatchHistoryStore';
import { applyTaskReferences } from '../shared/dispatchHistory';
import { WORKFLOW_STARTERS, type WorkflowPin } from '../shared/fleetWorkflow';

const dirs: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  dirs.splice(0).forEach((d) => fs.rmSync(d, { recursive: true, force: true }));
});
const owner = { sessionId: 'manager', isWakeTarget: true, status: 'active' };
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'manager-task-refs-'));
  dirs.push(dir);
  const store = new DispatchHistoryStore(() => path.join(dir, 'history.json'));
  const definition = structuredClone(WORKFLOW_STARTERS[0]);
  const pin: WorkflowPin = {
    definition,
    hash: 'frozen-hash',
    templates: {
      'ship-task': { id: 'ship-task', body: '{{task}}', params: [], resultSchema: {} },
    },
    steps: definition.steps.map((s) => ({ id: s.id, state: 'planned' })),
  };
  const task = store.startWorkflow(owner, dir, 'Task', pin);
  const rev = () => store.task(task.taskId)!.revision ?? 0;
  const set = (upsert?: unknown, remove?: unknown, revision = rev()) =>
    store.updateReferencesByManager(task.taskId, revision, upsert, remove, 'from chat');
  return { dir, store, task, rev, set };
}

describe('additive reference semantics', () => {
  it('adds without replacing what the host user already entered', () => {
    const before = {
      pullRequest: { number: '1', url: 'https://example.test/pr/1' },
      tickets: [{ id: 'HUMAN-1', url: 'https://jira.test/browse/HUMAN-1' }],
      references: [{ label: 'Design', url: 'https://example.test/design' }],
    };
    const after = applyTaskReferences(before, [{ kind: 'ticket', id: 'WKS-2' }], undefined);
    expect(after.pullRequest).toEqual(before.pullRequest);
    expect(after.references).toEqual(before.references);
    expect(after.tickets).toEqual([before.tickets[0], { id: 'WKS-2' }]);
  });
  it('upserts an exact entry by identity, case-insensitively, and keeps a URL it was not given', () => {
    const before = { tickets: [{ id: 'WKS-2', url: 'https://jira.test/browse/WKS-2' }] };
    expect(
      applyTaskReferences(before, [{ kind: 'ticket', id: 'wks-2' }], undefined).tickets,
    ).toEqual([{ id: 'wks-2', url: 'https://jira.test/browse/WKS-2' }]);
    expect(
      applyTaskReferences(
        before,
        [{ kind: 'ticket', id: 'WKS-2', url: 'https://x.test/2' }],
        undefined,
      ).tickets,
    ).toEqual([{ id: 'WKS-2', url: 'https://x.test/2' }]);
  });
  it('merges a pull request rather than clobbering the half it was not given', () => {
    const before = { pullRequest: { url: 'https://dev.azure.test/_git/r/pullrequest/9492' } };
    expect(
      applyTaskReferences(before, [{ kind: 'pullRequest', number: '9492' }], undefined),
    ).toEqual({
      pullRequest: { number: '9492', url: 'https://dev.azure.test/_git/r/pullrequest/9492' },
    });
  });
  it('removes only the named entry', () => {
    const before = { tickets: [{ id: 'A' }, { id: 'B' }], pullRequest: { number: '3' } };
    const after = applyTaskReferences(before, undefined, [{ kind: 'ticket', id: 'a' }]);
    expect(after.tickets).toEqual([{ id: 'B' }]);
    expect(after.pullRequest).toEqual({ number: '3' });
    expect(applyTaskReferences(before, undefined, [{ kind: 'pullRequest' }]).pullRequest).toBe(
      undefined,
    );
  });
  it('covers non-GitHub providers as plain recorded strings, with no provider inference', () => {
    const links = applyTaskReferences(
      {},
      [
        {
          kind: 'pullRequest',
          number: '9492',
          url: 'https://dev.azure.test/o/p/_git/r/pullrequest/9492',
        },
        { kind: 'ticket', id: 'WKS-412', url: 'https://jira.test/browse/WKS-412' },
        { kind: 'ticket', id: 'SUP-77' },
        {
          kind: 'reference',
          label: 'GitLab MR',
          url: 'https://gitlab.test/g/p/-/merge_requests/8',
        },
      ],
      undefined,
    );
    expect(links.pullRequest!.url).toContain('dev.azure.test');
    expect(links.tickets).toHaveLength(2);
    expect(links.references![0].label).toBe('GitLab MR');
  });
});

describe('reference validation is the host validator, not a second one', () => {
  const bad: [string, unknown][] = [
    ['non-http scheme', [{ kind: 'reference', label: 'x', url: 'file:///etc/passwd' }]],
    ['javascript scheme', [{ kind: 'reference', label: 'x', url: 'javascript:alert(1)' }]],
    ['embedded credentials', [{ kind: 'reference', label: 'x', url: 'https://u:p@example.test/' }]],
    [
      'oversized url',
      [{ kind: 'reference', label: 'x', url: 'https://e.test/' + 'a'.repeat(2100) }],
    ],
    ['oversized label', [{ kind: 'ticket', id: 'a'.repeat(201) }]],
    ['non-numeric pr number', [{ kind: 'pullRequest', number: '12a' }]],
    ['unknown field', [{ kind: 'ticket', id: 'A', extra: 'x' }]],
    ['unknown kind', [{ kind: 'waive', id: 'A' }]],
  ];
  for (const [name, upsert] of bad)
    it(`refuses ${name}`, () => {
      expect(() => applyTaskReferences({}, upsert, undefined)).toThrow();
    });
  it('refuses a duplicate URL across collections and an over-long collection', () => {
    expect(() =>
      applyTaskReferences(
        { pullRequest: { url: 'https://e.test/1' } },
        [{ kind: 'reference', label: 'dup', url: 'https://e.test/1' }],
        undefined,
      ),
    ).toThrow(/Duplicate/);
    expect(() =>
      applyTaskReferences(
        { tickets: Array.from({ length: 20 }, (_, i) => ({ id: `T-${i}` })) },
        [{ kind: 'ticket', id: 'T-20' }],
        undefined,
      ),
    ).toThrow(/at most 20/);
  });
  it('requires at least one entry and caps the batch size', () => {
    expect(() => applyTaskReferences({}, undefined, undefined)).toThrow(/at least one/);
    expect(() =>
      applyTaskReferences(
        {},
        Array.from({ length: 21 }, () => ({ kind: 'ticket', id: 'x' })),
        undefined,
      ),
    ).toThrow(/at most 20/);
  });
});

describe('store-level manager edit', () => {
  it('advances the revision, records a manager-attributed audit row and leaves other facts alone', () => {
    const f = fixture();
    const before = f.store.task(f.task.taskId)!;
    const updated = f.set([{ kind: 'ticket', id: 'WKS-9', url: 'https://jira.test/browse/WKS-9' }]);
    expect(updated.revision).toBe((before.revision ?? 0) + 1);
    expect(updated.links!.tickets).toEqual([
      { id: 'WKS-9', url: 'https://jira.test/browse/WKS-9' },
    ]);
    expect(updated.audit!.at(-1)).toMatchObject({ actor: 'manager', action: 'links' });
    expect(updated.workflow!.hash).toBe(before.workflow!.hash);
    expect(updated.workflow!.steps.map((s) => s.state)).toEqual(
      before.workflow!.steps.map((s) => s.state),
    );
    expect(updated.ownerSessionId).toBe(before.ownerSessionId);
    expect(updated.title).toBe(before.title);
  });
  it('refuses a stale revision and leaves the stored references untouched', () => {
    const f = fixture();
    f.set([{ kind: 'ticket', id: 'FIRST' }]);
    const stale = f.rev() - 1;
    expect(() => f.set([{ kind: 'ticket', id: 'SECOND' }], undefined, stale)).toThrow();
    expect(f.store.task(f.task.taskId)!.links!.tickets).toEqual([{ id: 'FIRST' }]);
  });
  it('is idempotent: a repeated identical edit adds no audit row and does not move the revision', () => {
    const f = fixture();
    f.set([{ kind: 'ticket', id: 'WKS-9' }]);
    const after = f.store.task(f.task.taskId)!;
    const again = f.set([{ kind: 'ticket', id: 'WKS-9' }]);
    expect(again.revision).toBe(after.revision);
    expect(again.audit!.filter((a) => a.action === 'links')).toHaveLength(1);
  });
  it('cannot waive a step: a host waiver survives a later manager reference edit untouched', () => {
    const f = fixture();
    const waived = f.store.editByHostUser(
      {
        taskId: f.task.taskId,
        expectedTaskRevision: f.rev(),
        action: 'waive',
        stepId: 'review',
      },
      (id) => ({ sessionId: id, status: 'ended' }),
      () => false,
    );
    expect(waived.ok).toBe(true);
    const audit = f.store.task(f.task.taskId)!.audit!.find((a) => a.action === 'waive')!;
    expect(audit.actor).toBe('host-user');
    f.set([{ kind: 'pullRequest', number: '9492' }]);
    const after = f.store.task(f.task.taskId)!;
    expect(after.workflow!.steps.find((s) => s.id === 'review')!.state).toBe('waived');
    expect(after.audit!.find((a) => a.action === 'waive')).toEqual(audit);
  });
  it('refuses an unknown task and a non-integer revision', () => {
    const f = fixture();
    expect(() =>
      f.store.updateReferencesByManager(
        'no-such-task',
        0,
        [{ kind: 'ticket', id: 'X' }],
        undefined,
        'r',
      ),
    ).toThrow(/no longer available/);
    expect(() => f.set([{ kind: 'ticket', id: 'X' }], undefined, -1 as number)).toThrow(
      /expectedTaskRevision/,
    );
  });
});
