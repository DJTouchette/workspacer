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
import { readFileSync } from 'fs';
import { join } from 'path';
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
  SLEEP_NOTE,
  sleepAffordance,
  nodesWorthShowing,
  describeSleepError,
  nodeMayStillBeRunning,
  type NodeState,
  type NodeTone,
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
  it('gives the five states five labels, and only the transitional ones read as progress', () => {
    const labels = Object.values(NODE_PRESENTATION).map((p) => p.label);
    expect(new Set(labels).size).toBe(5);
    // Four tones over five states, on purpose: `waking` and `stopping` SHARE
    // the busy token, because both are work in progress and neither is a fault.
    const tones = Object.values(NODE_PRESENTATION).map((p) => p.tone);
    expect(new Set(tones).size).toBe(4);

    for (const s of ['waking', 'stopping'] as const) {
      expect(NODE_PRESENTATION[s].progress).toBe(true);
      // A booting or draining machine paints with the WORKING token, not the
      // failure one — collapsing either into `unreachable` is what makes a
      // deliberate act read as a hang.
      expect(NODE_PRESENTATION[s].tone).toBe('busy');
    }
    for (const s of ['available', 'stopped', 'unreachable'] as const) {
      expect(NODE_PRESENTATION[s].progress).toBe(false);
    }
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
  // THIS SENTENCE CHANGED WITH THE SLEEP PATH, and the change is the feature.
  // It used to say a failed wake left the machine running and billing, because
  // it did — the hub had no stop verb. The hub now stops what its own wake
  // started, so this line reports the FAILURE and stops claiming a bill the
  // hub has closed. Whether that stop worked is the hub's own `detail`, which
  // is rendered above this line on the row.
  it('reports a failed wake without claiming a bill the hub has since closed', () => {
    expect(nodeWakeFailureNotice(den({ wakeFailures: 1 }))).toMatch(/1 wake failed/);
    expect(nodeWakeFailureNotice(den({ wakeFailures: 1 }))).not.toMatch(/running and billing/i);
    expect(nodeWakeFailureNotice(den({ wakeFailures: 3 }))).toMatch(/3 wakes failed/);
    expect(nodeWakeFailureNotice(den())).toBeNull();
  });

  it('prices the wake button itself, and now names the way back', () => {
    const a = wakeAffordance(den(), true);
    expect(a.enabled).toBe(true);
    expect(a.title).toMatch(/bills from boot/i);
    // The old copy ended "and nothing here can stop it again yet". There is now
    // something here that stops it, and the sentence has to say so or it is
    // scaring people off a button that is no longer one-way.
    expect(a.title).not.toMatch(/nothing here can stop it/i);
    expect(a.title).toMatch(/until you put it back to sleep/i);
  });

  // THE SLEEP COPY NAMES THE WORK, NOT THE SAVING. The money is why somebody
  // presses it; the work is what they need to be warned about.
  it('warns about the work a shutdown ends rather than the money it saves', () => {
    expect(SLEEP_NOTE).toMatch(/anything still running on it stops/i);
    const a = sleepAffordance(den({ state: 'available' }), true);
    expect(a.enabled).toBe(true);
    expect(a.title).toBe(SLEEP_NOTE);
  });
});

describe('the sleep affordance — never offer an off switch that does nothing', () => {
  it('offers it for a connected machine and not for one already off', () => {
    expect(sleepAffordance(den({ state: 'available' }), true).visible).toBe(true);
    expect(sleepAffordance(den({ state: 'stopped' }), true).visible).toBe(false);
    // `waking` is mid-transition; a stop pressed there would fight the button
    // beside it. The hub handles it, but that is not an interface.
    //
    // AND IT MUST BE EXCLUDED EXPLICITLY, not by accident of the running check:
    // a booting machine really IS mayBeRunning — the hub asked for it to be up —
    // so a version of this that only asked "is it running" put a Put-to-sleep
    // button on a node that said "Starting…". The e2e caught that; this pins it.
    expect(sleepAffordance(den({ state: 'waking' }), true).visible).toBe(false);
    expect(sleepAffordance(den({ state: 'waking', mayBeRunning: true }), true).visible).toBe(false);
    expect(sleepAffordance(den({ state: 'stopping', mayBeRunning: true }), true).enabled).toBe(
      false,
    );
  });

  it('offers it for an unreachable machine the hub says may STILL be running', () => {
    // The case with a meter attached, and the precise reason this button
    // exists. `unreachable` also covers a machine that is off and broken, and
    // an off switch for that one does nothing — so the state alone cannot
    // answer this and the hub sends the answer.
    const burning = den({ state: 'unreachable', mayBeRunning: true });
    expect(sleepAffordance(burning, true).visible).toBe(true);
    expect(sleepAffordance(burning, true).enabled).toBe(true);

    const brokenAndOff = den({ state: 'unreachable' });
    expect(sleepAffordance(brokenAndOff, true).visible).toBe(false);
  });

  it('shows the reason beside a disabled control, never only in a tooltip', () => {
    // There is no hover on a phone, and /app runs on one.
    const noAuthority = sleepAffordance(den({ state: 'available' }), false);
    expect(noAuthority.visible).toBe(true);
    expect(noAuthority.enabled).toBe(false);
    expect(noAuthority.reason).toMatch(/operator token/i);

    const noCredential = sleepAffordance(den({ state: 'available', wakeable: false }), true);
    expect(noCredential.enabled).toBe(false);
    expect(noCredential.reason).toMatch(/no credentials/i);
  });

  it('disables itself while a shutdown is already in flight', () => {
    expect(sleepAffordance(den({ state: 'stopping' }), true).enabled).toBe(false);
    expect(sleepAffordance(den({ state: 'available' }), true, true).enabled).toBe(false);
  });
});

describe('a connected machine is billing, so its off switch has to be reachable', () => {
  const connected = den({ state: 'available' });

  it('shows a connected node ONLY when this caller could actually stop it', () => {
    // The old rule — nothing to report renders nothing — still holds for every
    // caller that cannot act. A phone on the view tier, and any hub that merely
    // observes a node it cannot power, see exactly what they saw before.
    expect(nodesWorthShowing([connected], false)).toEqual([]);
    expect(nodesWorthShowing([den({ state: 'available', wakeable: false })], true)).toEqual([]);
    expect(nodesWorthShowing([connected], true)).toHaveLength(1);
  });

  it('still shows everything that needs attention, whoever is looking', () => {
    const asleep = den({ state: 'stopped' });
    const crashed = den({ state: 'available', lastExit: { reason: 'claudemon-died' } });
    expect(nodesWorthShowing([asleep, crashed], false)).toHaveLength(2);
    expect(nodesNeedingAttention([asleep, crashed])).toHaveLength(2);
  });
});

describe('sleep failures, in words a person can act on', () => {
  it('translates the refusals a client can actually cause', () => {
    expect(describeSleepError(new Error('nodes.sleep requires host authority (…)'))).toMatch(
      /operator token/i,
    );
    expect(describeSleepError(new Error('unknown node: "den"'))).toMatch(
      /no longer in the registry/i,
    );
    expect(
      describeSleepError(
        new Error(
          'this node has no cloud coordinates or credential on this hub, so it cannot be put to sleep from here: den',
        ),
      ),
    ).toMatch(/no cloud credentials/i);
    // The hub renders cloud failures BY CATEGORY rather than by quoting the
    // API, so its own sentence is safe to pass straight through.
    expect(describeSleepError(new Error('the cloud API is rate-limiting this machine'))).toBe(
      'the cloud API is rate-limiting this machine',
    );
    expect(describeSleepError(undefined)).toMatch(/couldn't stop the machine/i);
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

// ---------------------------------------------------------------------------
// THE CROSS-LANGUAGE CORPUS.
//
// Everything above pins this copy against itself, which is exactly how five
// readers of one payload drifted: each one was individually well-tested and
// nothing held them to each other. contracts/node-view-cases.json is that
// thing. The other loaders are services/hub/internal/nodes/view_test.go (the
// side that WRITES the payload, plus a substring sweep over the inline /m
// client) and a #[test] in apps/tui/src/nodes.rs.
// ---------------------------------------------------------------------------

const NODE_FIXTURE = JSON.parse(
  readFileSync(
    join(__dirname, '..', '..', '..', '..', '..', 'contracts', 'node-view-cases.json'),
    'utf8',
  ),
) as {
  states: { state: NodeState; transitional: boolean; wakeOffered: boolean; why: string }[];
  unknownStates: string[];
  lastExit: {
    cleanPrefix: string;
    cases: {
      name: string;
      reason: string;
      exitCode?: number;
      at?: string;
      clean: boolean;
      notice: boolean;
      recordAbsent?: boolean;
    }[];
  };
  presentation: {
    cases: {
      state: NodeState;
      tone: NodeTone;
      progress: boolean;
      desktopLabel: string;
      mobileLabel: string;
      fallbackDetail: string;
      tuiLabel: string | null;
      why: string;
    }[];
  };
};

describe('the state vocabulary (contracts/node-view-cases.json)', () => {
  it('knows every state the contract declares, and no others', () => {
    // BOTH directions. "every contract state is known here" alone would pass a
    // renderer that had grown a sixth state the hub never sends and no other
    // client renders — the same drift, walked the other way.
    const fromContract = NODE_FIXTURE.states.map((s) => s.state).sort();
    expect(Object.keys(NODE_PRESENTATION).sort()).toEqual(fromContract);
    for (const c of NODE_FIXTURE.states) {
      expect(normalizeNode({ id: 'ord', state: c.state, wakeable: true })?.state).toBe(c.state);
    }
  });

  it('coerces a state it does not recognise to `unreachable`, never renders it raw', () => {
    for (const s of NODE_FIXTURE.unknownStates) {
      expect(normalizeNode({ id: 'ord', state: s, wakeable: true })?.state).toBe('unreachable');
    }
  });

  it('offers an enabled wake in exactly the states the contract says', () => {
    // The money column. A `stopping` node used to fall through to the final
    // branch here and get an ENABLED Connect, rendered beside the disabled
    // "Shutting down…" the sleep affordance was already showing — two buttons
    // fighting over one row, and the enabled one races a stop the user asked
    // for.
    for (const c of NODE_FIXTURE.states) {
      const a = wakeAffordance(den({ state: c.state, wakeable: true }), true);
      expect(`${c.state}:${a.enabled}`).toBe(`${c.state}:${c.wakeOffered}`);
    }
  });

  it('animates exactly the transitional states', () => {
    for (const c of NODE_FIXTURE.states) {
      expect(`${c.state}:${NODE_PRESENTATION[c.state].progress}`).toBe(
        `${c.state}:${c.transitional}`,
      );
    }
  });
});

describe('lastExit verdicts (contracts/node-view-cases.json)', () => {
  it('reaches the contract verdict for every ending', () => {
    for (const c of NODE_FIXTURE.lastExit.cases) {
      const node = den({
        state: 'available',
        lastExit: c.recordAbsent
          ? undefined
          : { reason: c.reason || undefined, exitCode: c.exitCode, at: c.at },
      });
      const notice = nodeCrashNotice(node);
      expect(`${c.name}: ${notice !== null}`).toBe(`${c.name}: ${c.notice}`);
      if (c.notice) {
        // The reason is the whole content of the line — a notice that does not
        // name the ending tells a person to look without saying at what.
        expect(notice).toContain(c.reason);
      }
      if (c.clean) {
        expect(notice).toBeNull();
      }
    }
  });

  it('exercises both arms, so a corpus that drifted to one would fail here', () => {
    const clean = NODE_FIXTURE.lastExit.cases.filter((c) => c.clean).length;
    const crash = NODE_FIXTURE.lastExit.cases.filter((c) => c.notice).length;
    expect(clean).toBeGreaterThan(0);
    expect(crash).toBeGreaterThan(0);
    expect(NODE_FIXTURE.lastExit.cleanPrefix).toBe('signal-');
  });
});

describe('presentation (contracts/node-view-cases.json)', () => {
  it('renders each state with the words the contract carries', () => {
    for (const c of NODE_FIXTURE.presentation.cases) {
      const p = NODE_PRESENTATION[c.state];
      expect(p.label).toBe(c.desktopLabel);
      expect(p.tone).toBe(c.tone);
      expect(p.progress).toBe(c.progress);
      // The one string that must be byte-identical across copies: /m ships it
      // verbatim too, and the Go loader checks that side.
      expect(p.fallbackDetail).toBe(c.fallbackDetail);
      expect(nodeDetailLine(den({ state: c.state }))).toBe(c.fallbackDetail);
    }
  });

  it('covers every state in the vocabulary', () => {
    expect(NODE_FIXTURE.presentation.cases.map((c) => c.state).sort()).toEqual(
      NODE_FIXTURE.states.map((s) => s.state).sort(),
    );
  });
});
