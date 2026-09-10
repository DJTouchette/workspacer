import { afterEach, expect, it } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { RemoteDispatchRegistry, DISPATCH_PROTOCOL } from './remoteDispatchRegistry';

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-registry-'));
  dirs.push(dir);
  const file = path.join(dir, 'dispatches.json');
  const registry = new RemoteDispatchRegistry();
  registry.start(file, (id) => (id === 'manager' || id === 'successor' ? id : null));
  registry.open({
    dispatchId: '0123456789abcdef',
    peer: 'paired-credential-a',
    ownerSessionId: 'manager',
    localSessionId: 'local-worker',
  });
  registry.attachSession('0123456789abcdef', 'remote-worker');
  const update = {
    protocol: DISPATCH_PROTOCOL,
    dispatchId: '0123456789abcdef',
    sessionId: 'remote-worker',
    seq: 1,
    ts: 1,
    final: true,
    kind: 'worker-finished',
    entry: { sessionId: 'remote-worker', label: 'Worker', fullReply: 'done' },
  } as const;
  return { registry, file, update };
}
it('binds results to the recorded destination and actual worker, never a supplied parent', () => {
  const { registry, update } = fixture();
  expect(registry.accept(undefined, update)).toEqual({ ok: false, reason: 'not-federated' });
  expect(registry.accept('paired-credential-b', update)).toEqual({
    ok: false,
    reason: 'wrong-peer',
  });
  expect(registry.accept('paired-credential-a', { ...update, sessionId: 'other' })).toEqual({
    ok: false,
    reason: 'malformed',
  });
  expect(
    registry.accept('paired-credential-a', {
      ...update,
      sessionId: 'other',
      entry: { ...update.entry, sessionId: 'other' },
    }),
  ).toEqual({ ok: false, reason: 'session-mismatch' });
  expect(
    registry.accept('paired-credential-a', { ...update, parentSessionId: 'victim' }),
  ).toMatchObject({ ok: true, parentSessionId: 'manager' });
});
it('persists acknowledgement and deduplicates reconnect replay', () => {
  const { registry, file, update } = fixture();
  expect(registry.accept('paired-credential-a', update).ok).toBe(true);
  registry.acknowledge(update.dispatchId, update);
  const restored = new RemoteDispatchRegistry();
  restored.start(file, () => 'manager');
  expect(restored.accept('paired-credential-a', update)).toEqual({ ok: false, reason: 'closed' });
});
it('keeps uncertain admission open and transfers origin ownership on adoption', () => {
  const { registry, update } = fixture();
  registry.fail(update.dispatchId, 'timeout');
  registry.markLost(update.dispatchId, 'peer does not know yet');
  expect(registry.openForPeer('paired-credential-a')).toHaveLength(1);
  registry.reparent('manager', 'successor');
  expect(registry.accept('paired-credential-a', update)).toMatchObject({
    ok: true,
    parentSessionId: 'successor',
  });
});
it('rejects fractional sequence numbers and inconsistent terminal markers', () => {
  const { registry, update } = fixture();
  for (const invalid of [
    { ...update, seq: 1.5 },
    { ...update, final: false },
    { ...update, kind: 'progress' },
  ])
    expect(registry.accept('paired-credential-a', invalid)).toEqual({
      ok: false,
      reason: 'malformed',
    });
});

it('an interrupted wake stays unknown across restart rather than sending twice', () => {
  const { registry, file, update } = fixture();
  registry.beginDelivery(update.dispatchId, update.seq);
  const restored = new RemoteDispatchRegistry();
  restored.start(file, () => 'manager');
  expect(restored.accept('paired-credential-a', update)).toEqual({
    ok: false,
    reason: 'delivery-unknown',
  });
  expect(restored.list()[0].state).toBe('open');
});
it('corrupt durable state refuses new admission without overwriting it', () => {
  const { file } = fixture();
  fs.writeFileSync(file, 'not-json');
  expect(() => new RemoteDispatchRegistry().start(file, () => 'manager')).toThrow(/invalid/);
  expect(fs.readFileSync(file, 'utf8')).toBe('not-json');
});
