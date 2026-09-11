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

it('keeps byte custody independent from a live manager or unknown wake delivery', () => {
  const { registry, file, update } = fixture();
  registry.setHandoff(update.dispatchId, {
    binding: 'fixture-repository-binding',
    state: 'running',
  });
  registry.beginDelivery(update.dispatchId, update.seq);
  const restored = new RemoteDispatchRegistry();
  restored.start(file, () => null);
  expect(restored.accept('paired-credential-a', update).ok).toBe(false);
  expect(restored.acceptHandoffUpdate('paired-credential-a', update)?.record.dispatchId).toBe(
    update.dispatchId,
  );
  expect(restored.acceptHandoffUpdate('paired-credential-b', update)).toBeUndefined();
  expect(
    restored.acceptHandoffUpdate('paired-credential-a', {
      ...update,
      sessionId: 'other',
      entry: { ...update.entry, sessionId: 'other' },
    }),
  ).toBeUndefined();
  expect(restored.list()[0].ackedSeq).toBe(0);
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

it('reconciles adoption only after the new origin owner is durable', () => {
  const { registry, file } = fixture();
  let replayOwner = '';
  registry.onReparent = () => {
    replayOwner = JSON.parse(fs.readFileSync(file, 'utf8'))[0].ownerSessionId;
  };
  registry.reparent('manager', 'successor');
  expect(replayOwner).toBe('successor');
});

it('records verified unknown once without closing the dispatch or rewriting terminal state', () => {
  const { registry, file, update } = fixture();
  registry.markLost(update.dispatchId, 'Verified unknown; reconcile before retrying');
  const first = structuredClone(registry.list()[0]);
  const inode = fs.statSync(file).ino;
  registry.markLost(update.dispatchId, first.note!);
  expect(registry.list()[0]).toEqual(first);
  expect(fs.statSync(file).ino).toBe(inode);
  expect(first.state).toBe('open');
  expect(first.ownerSessionId).toBe('manager');
  registry.acknowledge(update.dispatchId, update);
  const terminal = structuredClone(registry.list()[0]);
  registry.markLost(update.dispatchId, 'Verified unknown; reconcile before retrying');
  expect(registry.list()[0]).toEqual(terminal);
});

it('retries a verified unknown note after a failed journal write', () => {
  const { registry, file, update } = fixture();
  const original = fs.readFileSync(file, 'utf8');
  fs.rmSync(file);
  fs.mkdirSync(file);
  expect(() => registry.markLost(update.dispatchId, 'Verified unknown')).toThrow();
  expect(registry.list()[0].note).toBeUndefined();
  fs.rmdirSync(file);
  fs.writeFileSync(file, original);
  registry.markLost(update.dispatchId, 'Verified unknown');
  expect(JSON.parse(fs.readFileSync(file, 'utf8'))[0].note).toBe('Verified unknown');
});

it.each([
  { name: 'no outcome', evidence: 'none', delivering: false, acknowledged: false },
  { name: 'nonterminal update', evidence: 'progress', delivering: true, acknowledged: false },
  {
    name: 'terminal before delivery',
    evidence: 'terminal',
    delivering: false,
    acknowledged: false,
  },
  {
    name: 'terminal with uncertain delivery',
    evidence: 'terminal',
    delivering: true,
    acknowledged: false,
  },
  { name: 'acknowledged terminal', evidence: 'terminal', delivering: true, acknowledged: true },
] as const)(
  'applies replay-unknown precedence for $name',
  ({ evidence, delivering, acknowledged }) => {
    const { registry, file, update } = fixture();
    const input = 'Worker outcome is unknown; reconcile before retrying';
    // A newly retained terminal packet must outrank even an identical earlier
    // generic note, before the delivery guard has been created.
    if (evidence === 'terminal' && !delivering) registry.markLost(update.dispatchId, input);
    if (evidence !== 'none')
      registry.retainEvidence(update.dispatchId, {
        ...update,
        kind: evidence === 'terminal' ? 'worker-finished' : 'progress',
        final: evidence === 'terminal',
      });
    if (delivering) registry.beginDelivery(update.dispatchId, update.seq);
    if (acknowledged) registry.acknowledge(update.dispatchId, update);
    const before = structuredClone(registry.list()[0]);
    const retained = registry.list()[0].lastUpdate;
    registry.markLost(update.dispatchId, input);
    const after = structuredClone(registry.list()[0]);
    if (acknowledged) expect(after).toEqual(before);
    else {
      expect(after).toEqual({ ...before, note: after.note });
      if (evidence === 'terminal') {
        expect(after.note).toContain('terminal outcome is retained locally');
        expect(after.note).toContain('wake delivery is unconfirmed');
        expect(after.note).not.toContain('outcome is unknown');
      } else expect(after.note).toBe(input);
    }
    expect(registry.list()[0].lastUpdate).toBe(retained);
    const inode = fs.statSync(file).ino;
    registry.markLost(update.dispatchId, input);
    expect(registry.list()[0]).toEqual(after);
    expect(fs.statSync(file).ino).toBe(inode);
    expect(JSON.parse(fs.readFileSync(file, 'utf8'))[0]).toEqual(JSON.parse(JSON.stringify(after)));
  },
);

it('keeps terminal evidence and the delivery guard precise across a failed note write', () => {
  const { registry, file, update } = fixture();
  registry.retainEvidence(update.dispatchId, update);
  registry.beginDelivery(update.dispatchId, update.seq);
  const before = structuredClone(registry.list()[0]);
  const original = fs.readFileSync(file, 'utf8');
  fs.rmSync(file);
  fs.mkdirSync(file);
  expect(() => registry.markLost(update.dispatchId, 'Worker outcome is unknown')).toThrow();
  expect(registry.list()[0]).toEqual(before);
  fs.rmdirSync(file);
  fs.writeFileSync(file, original);
  registry.markLost(update.dispatchId, 'Worker outcome is unknown');
  const after = registry.list()[0];
  expect(after.note).toContain('terminal outcome is retained locally');
  expect(after.note).toContain('wake delivery is unconfirmed');
  expect(after).toEqual({ ...before, note: after.note });
  expect(registry.accept('paired-credential-a', update)).toEqual({
    ok: false,
    reason: 'delivery-unknown',
  });
  expect(JSON.parse(fs.readFileSync(file, 'utf8'))[0].lastUpdate).toEqual(update);
});
