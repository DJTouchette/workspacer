import { describe, expect, it } from 'vitest';
import { importTaskHandoffResult, prepareTaskHandoff, type HandoffCall, type HandoffRecord } from './taskHandoff';

const manifest = { version: 1, task: 'a'.repeat(32), origin: 'b'.repeat(32), producer: 'a'.repeat(32), commit: 'c'.repeat(40), objectFormat: 'sha1', entries: [{ name: 'brief.md', kind: 'report' as const, size: 5, sha256: 'd'.repeat(64) }] };
const record: HandoffRecord = { plan: { version: 1, binding: 'binding-fixture-1', revision: '1', provider: 'claude', input: manifest, outputs: [] }, digest: 'e'.repeat(64), state: 'prepared', allocation: '/generated/worktree' };

describe('workspace handoff host orchestration', () => {
  it('verifies preparation after all bytes and never substitutes remote HEAD', async () => {
    const calls: string[] = [];
    const endpoint = (side: string): HandoffCall => async <T>(_method: string, raw: unknown) => {
      const p = raw as { operation: string };
      calls.push(`${side}:${p.operation}`);
      return (p.operation === 'read' ? { data: Buffer.from('hello').toString('base64') } : record) as T;
    };
    await prepareTaskHandoff(manifest.task, { binding: record.plan.binding, artifacts: [], outputs: [] }, 'claude', '/source', endpoint('origin'), endpoint('target'));
    expect(calls).toEqual(['origin:freeze', 'target:reserve', 'origin:publish', 'origin:read', 'target:write', 'target:prepare']);
  });

  it('does not acknowledge custody for a dirty result', async () => {
    const local: HandoffCall = async () => { throw new Error('must not import dirty output'); };
    const peer: HandoffCall = async <T>() => ({ ...record, state: 'needs-checkpoint' }) as T;
    expect((await importTaskHandoffResult(manifest.task, record.plan.binding, local, peer)).state).toBe('needs-checkpoint');
  });

  it('does not prepare after an interrupted artifact write', async () => {
    const calls: string[] = [];
    const local: HandoffCall = async <T>(_method: string, raw: unknown) => ((raw as { operation: string }).operation === 'read' ? { data: Buffer.from('hello').toString('base64') } : record) as T;
    const peer: HandoffCall = async <T>(_method: string, raw: unknown) => {
      const p = raw as { operation: string }; calls.push(p.operation);
      if (p.operation === 'write') throw new Error('link interrupted');
      return record as T;
    };
    await expect(prepareTaskHandoff(manifest.task, { binding: record.plan.binding, artifacts: [], outputs: [] }, 'claude', '/source', local, peer)).rejects.toThrow('link interrupted');
    expect(calls).toEqual(['reserve', 'write']);
  });
});
