/**
 * The remote-node reading logic — the half every surface shares.
 *
 * Contract: `.workspacer/reports/2026-08-24-fly-wake-contract.md`. These pin the
 * edges the hub already decided, so no client re-derives them: running-with-no-
 * provider is `unreachable`, stopped-after-a-failed-wake is `unreachable`, and a
 * node with no credential is never `stopped`. Nothing here infers a state — it
 * only reads the one the hub sent.
 */
import { describe, it, expect } from 'vitest';
import {
  NODE_PRESENTATION,
  applyNodeStateChange,
  describeWakeError,
  isHostAuthorityRefusal,
  isNodeRegistryAbsent,
  nodeCrashNotice,
  nodeDetailLine,
  nodeToneVar,
  nodeWakeFailureNotice,
  nodesNeedingAttention,
  nodesSummary,
  normalizeNode,
  normalizeNodes,
  wakeAffordance,
  type RemoteNodeView,
} from '../src/lib/remoteNodes';

const den = (over: Partial<RemoteNodeView> = {}): RemoteNodeView => ({
  id: 'den',
  label: 'Fly node (den)',
  state: 'stopped',
  wakeable: true,
  ...over,
});

describe('feature absence vs failure', () => {
  it('reads the router\'s "no provider" as FEATURE ABSENT, and nothing else', () => {
    expect(isNodeRegistryAbsent(new Error('no provider for nodes.list'))).toBe(true);
    expect(isNodeRegistryAbsent(new Error('no provider for nodes.wake'))).toBe(true);
    // The Electron IPC wrapper glues its own prefix on — still absent.
    expect(
      isNodeRegistryAbsent(
        new Error("Error invoking remote method 'nodes:list': Error: no provider for nodes.list"),
      ),
    ).toBe(true);
    // Real failures must NOT be swallowed into "this hub has no nodes", or a
    // broken hub renders as an ordinary install.
    expect(isNodeRegistryAbsent(new Error('hub not connected'))).toBe(false);
    expect(isNodeRegistryAbsent(new Error('hub call timeout: nodes.list'))).toBe(false);
    expect(isNodeRegistryAbsent(new Error('no provider for jobs.list'))).toBe(false);
    expect(isNodeRegistryAbsent(undefined)).toBe(false);
  });

  it('reads the tier refusal separately from the feature being absent', () => {
    const refusal = new Error(
      'nodes.wake requires host authority (starting a machine spends money)',
    );
    expect(isHostAuthorityRefusal(refusal)).toBe(true);
    expect(isNodeRegistryAbsent(refusal)).toBe(false);
  });
});

describe('normalizeNode', () => {
  it('keeps every field the contract sends, and drops what it did not', () => {
    const n = normalizeNode({
      id: 'den',
      label: 'Fly node (den)',
      state: 'waking',
      since: 1787631009502,
      lastSeen: 1787630000000,
      detail: 'the machine is up; waiting for its provider to register',
      wakeable: true,
      wakeFailures: 2,
      lastExit: { reason: 'claudemon-died', exitCode: 1, at: '2026-08-24T21:00:00Z' },
    })!;
    expect(n.state).toBe('waking');
    expect(n.since).toBe(1787631009502);
    expect(n.wakeFailures).toBe(2);
    expect(n.lastExit).toEqual({
      reason: 'claudemon-died',
      exitCode: 1,
      at: '2026-08-24T21:00:00Z',
    });
  });

  it('accepts the minimal payload the contract documents', () => {
    const n = normalizeNode({ id: 'den', label: 'den', state: 'available', wakeable: false })!;
    expect(n).toEqual({ id: 'den', label: 'den', state: 'available', wakeable: false });
    // Omitted is omitted — a missing lastExit means NOBODY KNOWS, and this
    // must never fabricate an empty record that reads as "ended cleanly".
    expect(n.lastExit).toBeUndefined();
    expect(nodeCrashNotice(n)).toBeNull();
  });

  it('falls the label back to the id, and an unknown state to unreachable', () => {
    expect(normalizeNode({ id: 'x', state: 'available', wakeable: true })!.label).toBe('x');
    // A state this client cannot presume to understand is, by definition, one
    // it does not know how to get a working node out of.
    expect(normalizeNode({ id: 'x', state: 'hibernating', wakeable: true })!.state).toBe(
      'unreachable',
    );
    expect(normalizeNode({ label: 'no id' })).toBeNull();
    expect(normalizeNode(null)).toBeNull();
    expect(normalizeNodes({ not: 'an array' })).toEqual([]);
    expect(normalizeNodes([den(), null, 7])).toHaveLength(1);
  });

  it('drops a wakeFailures of 0 rather than rendering "0 wakes failed"', () => {
    expect(
      normalizeNode({ id: 'x', state: 'stopped', wakeable: true, wakeFailures: 0 })!.wakeFailures,
    ).toBeUndefined();
  });
});

describe('applyNodeStateChange', () => {
  it('patches by id in place, preserving registry order', () => {
    const list = [den({ id: 'a' }), den({ id: 'b' }), den({ id: 'c' })];
    const next = applyNodeStateChange(list, den({ id: 'b', state: 'waking' }));
    expect(next.map((n) => n.id)).toEqual(['a', 'b', 'c']);
    expect(next[1].state).toBe('waking');
    expect(list[1].state).toBe('stopped'); // no mutation
  });

  it('appends a node the seed did not hold — the registry is hand-edited', () => {
    expect(applyNodeStateChange([den({ id: 'a' })], den({ id: 'z' })).map((n) => n.id)).toEqual([
      'a',
      'z',
    ]);
  });
});

describe('waking is not unreachable', () => {
  it('gives the four states four presentations, and only waking reads as progress', () => {
    const labels = Object.values(NODE_PRESENTATION).map((p) => p.label);
    expect(new Set(labels).size).toBe(4);
    const tones = Object.values(NODE_PRESENTATION).map((p) => p.tone);
    expect(new Set(tones).size).toBe(4);

    expect(NODE_PRESENTATION.waking.progress).toBe(true);
    for (const s of ['available', 'stopped', 'unreachable'] as const) {
      expect(NODE_PRESENTATION[s].progress).toBe(false);
    }
    // A booting machine paints with the WORKING token, not the failure one.
    expect(NODE_PRESENTATION.waking.tone).toBe('busy');
    expect(NODE_PRESENTATION.unreachable.tone).toBe('warning');
    expect(nodeToneVar('busy')).not.toBe(nodeToneVar('warning'));
    // …and `stopped` is calm, not an error: it is a machine off ON PURPOSE.
    expect(NODE_PRESENTATION.stopped.tone).toBe('muted');
  });

  it("prefers the hub's own sentence — it is written to be read by a person", () => {
    expect(nodeDetailLine(den({ state: 'waking', detail: 'the machine is up; waiting' }))).toBe(
      'the machine is up; waiting',
    );
    expect(nodeDetailLine(den({ state: 'waking' }))).toBe(NODE_PRESENTATION.waking.fallbackDetail);
  });
});

describe('lastExit — the only crash notice anyone gets', () => {
  it('reports a crash and stays quiet about a deliberate stop', () => {
    expect(nodeCrashNotice(den({ lastExit: { reason: 'claudemon-died', exitCode: 1 } }))).toMatch(
      /did not end cleanly: claudemon-died \(exit 1\)/,
    );
    expect(nodeCrashNotice(den({ lastExit: { reason: 'brain-died' } }))).toMatch(/brain-died/);
    expect(nodeCrashNotice(den({ lastExit: { reason: 'signal-TERM' } }))).toBeNull();
    expect(nodeCrashNotice(den({ lastExit: { reason: 'signal-INT' } }))).toBeNull();
    // Missing means nobody knows, NOT that it ended cleanly.
    expect(nodeCrashNotice(den())).toBeNull();
  });

  it('puts a crashed-but-available node in front of someone anyway', () => {
    const healthy = den({ id: 'ok', state: 'available' });
    const revived = den({
      id: 'revived',
      state: 'available',
      lastExit: { reason: 'claudemon-died' },
    });
    expect(nodesNeedingAttention([healthy, revived]).map((n) => n.id)).toEqual(['revived']);
    expect(nodesNeedingAttention([healthy])).toEqual([]);
  });
});

describe('cost honesty', () => {
  it('says a failed wake left a machine running and billing', () => {
    expect(nodeWakeFailureNotice(den({ wakeFailures: 1 }))).toMatch(
      /1 wake failed.*running and billing/,
    );
    expect(nodeWakeFailureNotice(den({ wakeFailures: 3 }))).toMatch(/3 wakes failed/);
    expect(nodeWakeFailureNotice(den())).toBeNull();
  });

  it('prices the button itself, since this hub cannot stop a machine', () => {
    const a = wakeAffordance(den(), true);
    expect(a.enabled).toBe(true);
    expect(a.title).toMatch(/bills from boot/i);
    expect(a.title).toMatch(/nothing here can stop it/i);
  });
});

describe('never offer a button that will be refused', () => {
  it('offers nothing at all for a connected node', () => {
    expect(wakeAffordance(den({ state: 'available' }), true).visible).toBe(false);
  });

  it('refuses to arm a second wake on a machine already starting', () => {
    const a = wakeAffordance(den({ state: 'waking' }), true);
    expect(a.visible).toBe(true);
    expect(a.enabled).toBe(false);
    expect(a.label).toMatch(/starting/i);
    // …and the same while our own wake is in flight and unanswered.
    expect(wakeAffordance(den(), true, true).enabled).toBe(false);
  });

  it('shows the state and disables the button on a view/triage tier', () => {
    const a = wakeAffordance(den(), false);
    expect(a.visible).toBe(true);
    expect(a.enabled).toBe(false);
    // The reason survives without a hover — there is no hover on a phone.
    expect(a.reason).toMatch(/operator token/i);
    expect(a.title).toMatch(/spends money/i);
  });

  it('disables the button for a node the hub holds no credentials for', () => {
    // wakeable:false is the hub saying the wake would fail EVERY time — the
    // no-credential case, which is why such a node is never reported `stopped`.
    const a = wakeAffordance(den({ state: 'unreachable', wakeable: false }), true);
    expect(a.enabled).toBe(false);
    expect(a.reason).toMatch(/no credentials/i);
  });
});

describe('describeWakeError', () => {
  it('turns each documented refusal into something a person can act on', () => {
    expect(describeWakeError(new Error('nodes.wake requires host authority (…)'))).toMatch(
      /operator token/i,
    );
    expect(describeWakeError(new Error('unknown node "zzz"'))).toMatch(/no longer in the registry/);
    expect(
      describeWakeError(new Error('den has no cloud coordinates or credential on this hub')),
    ).toMatch(/no cloud credentials/);
    expect(describeWakeError(new Error('no provider for nodes.wake'))).toMatch(/no longer has/);
  });

  it("passes the hub's own category wording through, minus the IPC wrapper", () => {
    // The hub renders cloud failures BY CATEGORY and never quotes the API's
    // body, so its sentence is safe to show verbatim.
    expect(
      describeWakeError(
        new Error(
          "Error invoking remote method 'nodes:wake': Error: the cloud API is rate-limiting this machine",
        ),
      ),
    ).toBe('the cloud API is rate-limiting this machine');
    expect(describeWakeError(new Error(''))).toMatch(/Couldn't start the machine/);
  });
});

describe('nodesSummary', () => {
  it('leads with what is happening, not with what is fine', () => {
    expect(
      nodesSummary([
        den({ id: 'a', state: 'available' }),
        den({ id: 'b', state: 'stopped' }),
        den({ id: 'c', state: 'waking' }),
      ]),
    ).toBe('1 starting · 1 asleep · 1 connected');
    expect(nodesSummary([])).toBe('');
  });
});
