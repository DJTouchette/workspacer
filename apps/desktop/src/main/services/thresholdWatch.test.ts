/**
 * notify_when: the manager's alternative to polling. Doctrine forbids the loop,
 * so a runaway worker's cost was invisible until it finished ($22 / 309K
 * tokens, spotted by chance, 2026-08-21). These tests pin the properties that
 * make an armed watch trustworthy enough to STOP watching: it fires once, it
 * fires on the right threshold, and it never fires into nothing.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  ThresholdWatcher,
  crossedBy,
  parsePredicate,
  sessionTokens,
  type WatchableSession,
} from './thresholdWatch';
import { parseFleetMessage } from '../shared/fleetMessages';

const session = (over: Partial<WatchableSession> = {}): WatchableSession => ({
  sessionId: 'w1',
  cwd: '/home/u/Work/alpha',
  label: 'alpha: ship',
  status: 'active',
  ambientState: 'streaming',
  lastActivity: 1_000,
  usage: { totalInputTokens: 0, totalOutputTokens: 0, costUSD: 0 },
  ...over,
});

const mgr = (): WatchableSession => ({
  sessionId: 'mgr',
  label: 'Fleet Manager',
  status: 'active',
});

function rig(sessions: WatchableSession[]) {
  const deliver = vi.fn().mockResolvedValue(undefined);
  const watcher = new ThresholdWatcher(deliver, () => sessions);
  return { deliver, watcher };
}

function withHealth(
  now: number,
  usedTokens: number,
  windowTokens = 200_000,
  over: Record<string, unknown> = {},
): WatchableSession {
  return session({
    provider: 'codex',
    usage: null,
    statusLine: {
      totalInputTokens: 9_000_000,
      totalOutputTokens: 500_000,
      contextHealth: {
        usedTokens,
        windowTokens,
        usedPct: (usedTokens / windowTokens) * 100,
        windowSource: 'runtime',
        observedAt: new Date(now).toISOString(),
        epoch: 7,
        provider: 'codex',
        ...over,
      },
    },
  });
}

describe('parsePredicate', () => {
  it('refuses an EMPTY predicate — an armed watch that can never fire is a lie', () => {
    expect(() => parsePredicate({})).toThrow(/at least one threshold/);
  });

  it('refuses a non-positive or non-numeric threshold', () => {
    expect(() => parsePredicate({ usd: 0 })).toThrow(/positive number/);
    expect(() => parsePredicate({ tokens: -1 })).toThrow(/positive number/);
    expect(() => parsePredicate({ idleSeconds: NaN })).toThrow(/positive number/);
  });

  it('keeps every threshold given', () => {
    expect(parsePredicate({ tokens: 1000, usd: 5, idleSeconds: 60 })).toEqual({
      tokens: 1000,
      usd: 5,
      idleSeconds: 60,
    });
  });

  it('accepts only finite context percentages in (0, 100] and keeps health single-purpose', () => {
    expect(parsePredicate({ contextUsedPct: 80 })).toEqual({ contextUsedPct: 80 });
    expect(parsePredicate({ contextUsedPct: 100 })).toEqual({ contextUsedPct: 100 });
    for (const bad of [0, -1, 100.01, Infinity, NaN]) {
      expect(() => parsePredicate({ contextUsedPct: bad })).toThrow(/finite number in \(0, 100\]/);
    }
    expect(() => parsePredicate({ contextUsedPct: 80, tokens: 1 })).toThrow(/single-purpose/);
  });
});

describe('contextUsedPct health semantics', () => {
  const now = Date.parse('2026-08-31T20:10:00Z');

  it('arms alreadySatisfied and fires at the exact boundary with explainable provenance', () => {
    const sessions = [mgr(), withHealth(now, 160_000)];
    const { watcher, deliver } = rig(sessions);
    const armed = watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now,
    });
    expect(armed.state).toBe('alreadySatisfied');
    watcher.sweep(now);
    const text = deliver.mock.calls[0][1] as string;
    expect(text).toContain('contextUsedPct active context 80% ≥ 80%');
    expect(text).toContain('160,000 / 200,000 tokens');
    expect(text).toContain('runtime-confirmed by codex');
    expect(text).toContain('epoch 7');
    expect(watcher.list()).toHaveLength(0);
  });

  it('waits through missing, provisional, stale, and internally inconsistent telemetry', () => {
    const target = withHealth(now, 180_000);
    const sessions = [mgr(), target];
    const { watcher, deliver } = rig(sessions);
    target.statusLine!.contextHealth = undefined;
    const armed = watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now,
    });
    expect(armed.state).toBe('waitingForTelemetry');

    target.statusLine!.contextHealth = withHealth(now, 180_000).statusLine!.contextHealth;
    target.statusLine!.contextHealth!.windowSource = 'requested';
    watcher.sweep(now);
    target.statusLine!.contextHealth!.windowSource = 'runtime';
    target.statusLine!.contextHealth!.observedAt = new Date(now - 120_001).toISOString();
    watcher.sweep(now);
    target.statusLine!.contextHealth!.observedAt = new Date(now).toISOString();
    target.statusLine!.contextHealth!.usedPct = 10;
    watcher.sweep(now);
    expect(deliver).not.toHaveBeenCalled();
    expect(watcher.list()).toHaveLength(1);

    target.statusLine!.contextHealth!.usedPct = 90;
    watcher.sweep(now);
    expect(deliver).toHaveBeenCalledTimes(1);
  });

  it('handles compaction as a newer decrease, then fires on a later genuine rise', () => {
    const target = withHealth(now, 120_000);
    const sessions = [mgr(), target];
    const { watcher, deliver } = rig(sessions);
    watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now,
    });
    target.statusLine!.contextHealth = withHealth(now + 1_000, 40_000).statusLine!.contextHealth;
    watcher.sweep(now + 1_000);
    expect(deliver).not.toHaveBeenCalled();
    target.statusLine!.contextHealth = withHealth(now + 2_000, 170_000).statusLine!.contextHealth;
    watcher.sweep(now + 2_000);
    expect(deliver).toHaveBeenCalledTimes(1);
  });

  it('invalidates one-shot on provider or telemetry-epoch changes and asks for re-arm', () => {
    const target = withHealth(now, 100_000);
    const sessions = [mgr(), target];
    const { watcher, deliver } = rig(sessions);
    watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now,
    });
    target.statusLine!.contextHealth = withHealth(now + 1_000, 170_000, 200_000, {
      epoch: 8,
    }).statusLine!.contextHealth;
    watcher.sweep(now + 1_000);
    expect(deliver.mock.calls[0][1]).toContain('telemetry epoch 7 → 8');
    expect(deliver.mock.calls[0][1]).toContain('re-arm');
    expect(watcher.list()).toHaveLength(0);

    const second = rig(sessions);
    target.statusLine!.contextHealth = withHealth(now + 2_000, 100_000, 200_000, {
      epoch: 8,
    }).statusLine!.contextHealth;
    second.watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now: now + 2_000,
    });
    target.provider = 'claude';
    second.watcher.sweep(now + 3_000);
    expect(second.deliver.mock.calls[0][1]).toContain('provider/session boundary');
  });

  it('never substitutes cumulative token throughput for active occupancy', () => {
    const target = withHealth(now, 20_000); // 10% active, despite 9.5M cumulative.
    const sessions = [mgr(), target];
    const { watcher, deliver } = rig(sessions);
    watcher.arm({
      sessionId: 'w1',
      watcherSessionId: 'mgr',
      predicate: { contextUsedPct: 80 },
      now,
    });
    watcher.sweep(now);
    expect(sessionTokens(target)).toBe(9_500_000);
    expect(deliver).not.toHaveBeenCalled();
    expect(watcher.list()).toHaveLength(1);
  });
});

describe('crossedBy', () => {
  const w = (over: object) => ({
    id: 'x',
    sessionId: 'w1',
    watcherSessionId: 'mgr',
    armedAt: 0,
    ...over,
  });

  it('counts CUMULATIVE tokens, input plus output', () => {
    const s = session({
      usage: { totalInputTokens: 200_000, totalOutputTokens: 109_412, costUSD: 0 },
    });
    // The live case: 309,412 tokens against a 250,000 watch.
    expect(crossedBy(w({ tokens: 250_000 }) as never, s, 0)).toBe('tokens 309,412 ≥ 250,000');
    expect(crossedBy(w({ tokens: 400_000 }) as never, s, 0)).toBeNull();
  });

  it('reports cost with the currency and two decimals', () => {
    const s = session({ usage: { costUSD: 22.4 } });
    expect(crossedBy(w({ usd: 20 }) as never, s, 0)).toBe('cost $22.40 ≥ $20.00');
  });

  it('fires on a settled session and names it as idle', () => {
    const idle = session({ ambientState: 'idle', lastActivity: 0 });
    expect(crossedBy(w({ idleSeconds: 60 }) as never, idle, 600_000)).toBe('idle for 600s ≥ 60s');
  });

  // The wedge. This predicate exists to catch "a worker that stopped without
  // finishing", and the worst way a worker stops is the one that keeps
  // reporting `streaming` — an approval clobbered mid-turn leaves the session
  // claiming to work with nothing arriving, forever. Gating on
  // `ambientState === 'idle'` made every such watch unfireable, so the one
  // failure a manager most wants a wake for was the one it could never send.
  it('fires on a session that claims to be working but has produced nothing', () => {
    const wedged = session({ ambientState: 'streaming', lastActivity: 0 });
    expect(crossedBy(w({ idleSeconds: 60 }) as never, wedged, 600_000)).toBe(
      'no activity for 600s ≥ 60s (still reports streaming)',
    );
    // Blocked-on-the-user counts too: 10 minutes parked on an approval is
    // something the manager wants told, and the message says which it is.
    const parked = session({ ambientState: 'waiting_approval', lastActivity: 0 });
    expect(crossedBy(w({ idleSeconds: 60 }) as never, parked, 600_000)).toBe(
      'no activity for 600s ≥ 60s (still reports waiting_approval)',
    );
  });

  it('does not fire on a session that is producing, whatever its state', () => {
    const working = session({ ambientState: 'streaming', lastActivity: 590_000 });
    expect(crossedBy(w({ idleSeconds: 60 }) as never, working, 600_000)).toBeNull();
  });

  it('reports ONE crossing even when two thresholds are met', () => {
    const s = session({ usage: { totalInputTokens: 500_000, totalOutputTokens: 0, costUSD: 99 } });
    expect(crossedBy(w({ tokens: 1, usd: 1 }) as never, s, 0)).toMatch(/^tokens /);
  });
});

// Managed providers (codex/opencode/pi) never populate `usage` — the root
// cause is that claudemon's conversation-item mapper never emits a 'usage'
// item for them, so the accumulator that fills `session.usage` never runs.
// Without a statusLine fallback here, notify_when(tokens|usd) can never fire
// for a codex worker: sessionTokens() and the usd branch both read 0 forever,
// silently, with no error — it just never wakes the manager.
describe('codex-shaped sessions (usage: null) fall back to statusLine', () => {
  const codexSession = (over: Partial<WatchableSession> = {}): WatchableSession =>
    session({
      usage: null,
      statusLine: { totalInputTokens: 200_000, totalOutputTokens: 109_412, costUSD: 22.4 },
      ...over,
    });

  it('sessionTokens sums statusLine tokens when usage is null', () => {
    expect(sessionTokens(codexSession())).toBe(309_412);
  });

  it('crossedBy fires on tokens for a codex session', () => {
    const w = { id: 'x', sessionId: 'w1', watcherSessionId: 'mgr', armedAt: 0, tokens: 250_000 };
    expect(crossedBy(w as never, codexSession(), 0)).toBe('tokens 309,412 ≥ 250,000');
  });

  it('crossedBy fires on usd for a codex session', () => {
    const w = { id: 'x', sessionId: 'w1', watcherSessionId: 'mgr', armedAt: 0, usd: 20 };
    expect(crossedBy(w as never, codexSession(), 0)).toBe('cost $22.40 ≥ $20.00');
  });

  it('prefers usage over statusLine when both are present', () => {
    const s = codexSession({ usage: { totalInputTokens: 1, totalOutputTokens: 1, costUSD: 1 } });
    expect(sessionTokens(s)).toBe(2);
  });

  it('an armed watch actually SWEEPS and wakes the manager for a codex worker', () => {
    const sessions = [mgr(), codexSession()];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 20 } });
    watcher.sweep(0);
    expect(deliver).toHaveBeenCalledTimes(1);
    const [target, text] = deliver.mock.calls[0] as [string, string];
    expect(target).toBe('mgr');
    expect(text).toContain('cost $22.40 ≥ $20.00');
  });
});

describe('ThresholdWatcher.arm', () => {
  it('refuses a target that does not exist or has ended', () => {
    const { watcher } = rig([mgr(), session({ status: 'ended' })]);
    expect(() =>
      watcher.arm({ sessionId: 'nope', watcherSessionId: 'mgr', predicate: { usd: 1 } }),
    ).toThrow(/no such session/);
    expect(() =>
      watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 1 } }),
    ).toThrow(/already ended/);
  });

  it('refuses a recipient that is not live — a watch with no listener fires into nothing', () => {
    const { watcher } = rig([session()]);
    expect(() =>
      watcher.arm({ sessionId: 'w1', watcherSessionId: 'ghost', predicate: { usd: 1 } }),
    ).toThrow(/not a live session/);
  });

  it('caps how many watches one watcher may arm', () => {
    const { watcher } = rig([mgr(), session()]);
    for (let i = 0; i < 20; i++) {
      watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: i + 1 } });
    }
    expect(() =>
      watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 99 } }),
    ).toThrow(/max 20/);
  });
});

describe('ThresholdWatcher.sweep', () => {
  it('wakes the watcher with a parseable [fleet] message naming what crossed', () => {
    const sessions = [mgr(), session()];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 20 } });

    watcher.sweep(0);
    expect(deliver).not.toHaveBeenCalled(); // nothing crossed yet

    sessions[1].usage = { costUSD: 22.4 };
    watcher.sweep(0);
    expect(deliver).toHaveBeenCalledTimes(1);
    const [target, text] = deliver.mock.calls[0] as [string, string];
    expect(target).toBe('mgr');
    expect(text).toContain('[fleet] A threshold you asked to be told about');
    expect(text).toContain('cost $22.40 ≥ $20.00');
    expect(text).toContain('do not start polling');
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('threshold');
    expect(parsed?.entries[0]).toMatchObject({ sessionId: 'w1', crossed: 'cost $22.40 ≥ $20.00' });
  });

  it('is ONE-SHOT: a second sweep past the same threshold delivers nothing', () => {
    const sessions = [mgr(), session({ usage: { costUSD: 50 } })];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 20 } });
    watcher.sweep(0);
    watcher.sweep(0);
    watcher.sweep(0);
    expect(deliver).toHaveBeenCalledTimes(1);
    expect(watcher.list()).toHaveLength(0);
  });

  it('coalesces several crossings into ONE wake per watcher', () => {
    const sessions = [
      mgr(),
      session({ sessionId: 'a', label: 'alpha', usage: { costUSD: 50 } }),
      session({ sessionId: 'b', label: 'beta', usage: { costUSD: 50 } }),
    ];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'a', watcherSessionId: 'mgr', predicate: { usd: 1 } });
    watcher.arm({ sessionId: 'b', watcherSessionId: 'mgr', predicate: { usd: 1 } });
    watcher.sweep(0);
    expect(deliver).toHaveBeenCalledTimes(1);
    expect(parseFleetMessage(deliver.mock.calls[0][1] as string)?.entries).toHaveLength(2);
  });

  it('DROPS (never fires) a watch whose target ended — the finish wake already told them', () => {
    const sessions = [mgr(), session({ usage: { costUSD: 50 } })];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 1 } });
    sessions[1].status = 'ended';
    watcher.sweep(0);
    expect(deliver).not.toHaveBeenCalled();
    expect(watcher.list()).toHaveLength(0);
  });

  it('drops a watch whose WATCHER ended — nobody is listening', () => {
    const sessions = [mgr(), session({ usage: { costUSD: 50 } })];
    const { deliver, watcher } = rig(sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 1 } });
    sessions[0].status = 'ended';
    watcher.sweep(0);
    expect(deliver).not.toHaveBeenCalled();
    expect(watcher.list()).toHaveLength(0);
  });

  it('survives a delivery failure (the watcher just ended) without throwing', async () => {
    const sessions = [mgr(), session({ usage: { costUSD: 50 } })];
    const deliver = vi.fn().mockRejectedValue(new Error('gone'));
    const watcher = new ThresholdWatcher(deliver, () => sessions);
    watcher.arm({ sessionId: 'w1', watcherSessionId: 'mgr', predicate: { usd: 1 } });
    expect(() => watcher.sweep(0)).not.toThrow();
    await Promise.resolve();
  });
});
