/**
 * `notify_when` — threshold alerts, so a manager never has to poll.
 *
 * Manager doctrine forbids polling, and it is right to: a manager looping on
 * list_agents is not monitoring, it is a hang that locks the user out. But the
 * doctrine left a hole — with no polling, a worker's cost and context are
 * invisible until it finishes. On 2026-08-21 a worker reached $22 and 309K
 * tokens and was noticed by chance.
 *
 * This closes the hole the same way the worker-finished wake closed the "how do
 * I know it's done" one: the manager ASKS to be told, once, and then stops.
 * `notify_when(session, {tokens|usd|idleSeconds|contextUsedPct})` arms a one-shot watch;
 * crossing it delivers a `[fleet]` wake through the same channel every other
 * wake uses, so a manager needs no new habit to receive it.
 *
 * Deliberate limits, all of them visible to the caller:
 *  - ONE-SHOT. A watch fires once and is gone. A re-arming watch would be a
 *    poll with extra steps — the wake exists so the manager can decide what to
 *    do, and deciding "watch it again" is a decision it should make out loud.
 *  - IN-MEMORY. Watches do not survive a workspacer restart. Persisting them
 *    would make them the jobs system, which already exists and is deliberately
 *    user-approved; a watch is a within-session intention.
 *  - EVALUATED ON A SWEEP, not on every snapshot push. A threshold on spend is
 *    not a real-time signal, and a sweep cannot be starved by a chatty session.
 *
 * The wake goes to the WATCHER (the manager that armed it), never to the
 * watched session, and the watcher must be a live session — a watch with no
 * recipient is refused at arm time rather than firing into nothing later.
 */

import { buildFleetMessage, type FleetMessageEntry } from '../shared/fleetMessages';

/** How often armed watches are evaluated. Fast enough that a runaway spend is
 *  caught within a turn or two, slow enough to be free. */
export const SWEEP_MS = 15_000;

/** Most watches one session may arm. A cap, not a policy: an agent in a loop
 *  arming watches would otherwise turn a wake channel into a firehose. */
export const MAX_WATCHES_PER_WATCHER = 20;

/** Runtime context evidence older than this is not actionable. The watch stays
 * armed and waits for a new sample; absence/staleness never becomes 0%. */
export const CONTEXT_HEALTH_MAX_AGE_MS = 2 * 60_000;
const CONTEXT_HEALTH_FUTURE_SKEW_MS = 5_000;
const CONTEXT_HEALTH_PROVIDERS = new Set(['claude', 'codex', 'copilot']);
const NO_CONTEXT_WINDOW_PROVIDERS = new Set(['opencode', 'pi']);

export interface ThresholdPredicate {
  /** Cache-inclusive cumulative throughput/cadence (input + output), not
   * active-context health. Compaction does not reset it. */
  tokens?: number;
  /** Fire when the session's cumulative cost in USD reaches this. */
  usd?: number;
  /** Fire when nothing has arrived from the session for this many seconds —
   *  whether it is sitting at a prompt or claiming to still be working (see
   *  `crossedBy`: a wedged session reports `streaming` forever). */
  idleSeconds?: number;
  /** Fire when runtime-confirmed ACTIVE context occupancy reaches this share
   * of the same runtime-confirmed effective window. Valid range: (0, 100]. */
  contextUsedPct?: number;
}

export interface ThresholdWatch extends ThresholdPredicate {
  id: string;
  /** The session being watched. */
  sessionId: string;
  /** The session to wake when it crosses — the manager that armed the watch. */
  watcherSessionId: string;
  armedAt: number;
  /** Health watches bind to the target's provider/telemetry life. */
  contextProvider?: string;
  /** Exact decimal wire value; nanosecond-seeded epochs exceed JS safe integer. */
  contextEpoch?: string;
  state?: 'armed' | 'alreadySatisfied' | 'waitingForTelemetry';
}

/** Return a detached API/test view; armed watches remain mutable during sweep. */
function snapshotWatch(watch: ThresholdWatch): ThresholdWatch {
  return { ...watch };
}

/** What the sweep needs of a session. A structural subset of
 *  ClaudeSessionState, so the store's rows satisfy it as-is. */
export interface WatchableSession {
  sessionId: string;
  cwd?: string;
  label?: string;
  status?: string;
  provider?: string;
  ambientState?: string;
  lastActivity?: number;
  usage?: {
    totalInputTokens?: number;
    totalOutputTokens?: number;
    costUSD?: number;
  } | null;
  /** Managed providers (codex/opencode/pi) never populate `usage` — their
   *  numbers live only here. Same fallback analyticsWriter.ts uses. */
  statusLine?: {
    totalInputTokens?: number;
    totalOutputTokens?: number;
    costUSD?: number;
    contextHealth?: ContextHealthSample;
  };
}

export interface ContextHealthSample {
  usedTokens: number;
  windowTokens: number;
  usedPct: number;
  windowSource: 'runtime' | string;
  observedAt: string;
  epoch: string;
  provider: string;
}

interface ContextHealthReading extends ContextHealthSample {
  observedAtMs: number;
}

/** Return only fresh, internally-consistent runtime evidence. */
export function contextHealthReading(
  s: WatchableSession,
  now: number,
): ContextHealthReading | null {
  const h = s.statusLine?.contextHealth;
  if (!h || h.windowSource !== 'runtime') return null;
  const healthProvider = normalizedProvider(h.provider);
  const sessionProvider = normalizedProvider(s.provider);
  const observedAtMs = Date.parse(h.observedAt);
  if (
    !Number.isFinite(h.usedTokens) ||
    !Number.isFinite(h.windowTokens) ||
    !Number.isFinite(h.usedPct) ||
    typeof h.epoch !== 'string' ||
    !/^[1-9]\d{0,19}$/.test(h.epoch) ||
    (h.epoch.length === 20 && h.epoch > '18446744073709551615') ||
    !Number.isFinite(observedAtMs) ||
    h.usedTokens < 0 ||
    h.windowTokens <= 0 ||
    h.usedTokens > h.windowTokens ||
    h.usedPct < 0 ||
    h.usedPct > 100 ||
    !healthProvider ||
    (sessionProvider !== undefined && healthProvider !== sessionProvider) ||
    observedAtMs > now + CONTEXT_HEALTH_FUTURE_SKEW_MS ||
    now - observedAtMs > CONTEXT_HEALTH_MAX_AGE_MS
  ) {
    return null;
  }
  const computed = (h.usedTokens / h.windowTokens) * 100;
  if (Math.abs(computed - h.usedPct) > 0.01) return null;
  return { ...h, provider: healthProvider, observedAtMs };
}

/** Total tokens a session has burned — the number the manager means by "how big
 *  has this got". Cumulative, not the point-in-time context window: a worker
 *  that compacts twice has spent the tokens either way. */
export function sessionTokens(s: WatchableSession): number {
  const input = s.usage?.totalInputTokens ?? s.statusLine?.totalInputTokens ?? 0;
  const output = s.usage?.totalOutputTokens ?? s.statusLine?.totalOutputTokens ?? 0;
  return input + output;
}

/**
 * The predicate that has been crossed, rendered for the wake — or null.
 *
 * Checked in the order tokens → usd → idle so a session crossing two reports
 * the one the caller is most likely to have meant (spend before staleness), and
 * one line, not three: the watch is one-shot, so one crossing ends it.
 */
export function crossedBy(watch: ThresholdWatch, s: WatchableSession, now: number): string | null {
  if (watch.contextUsedPct !== undefined) {
    return contextWatchCrossing(watch, s, now);
  }
  if (watch.tokens !== undefined) {
    const tokens = sessionTokens(s);
    if (tokens >= watch.tokens) {
      return `tokens ${tokens.toLocaleString('en-US')} ≥ ${watch.tokens.toLocaleString('en-US')}`;
    }
  }
  if (watch.usd !== undefined) {
    const usd = s.usage?.costUSD ?? s.statusLine?.costUSD ?? 0;
    if (usd >= watch.usd) return `cost $${usd.toFixed(2)} ≥ $${watch.usd.toFixed(2)}`;
  }
  if (watch.idleSeconds !== undefined) {
    // "A worker that stopped without finishing" is this predicate's whole
    // stated purpose (mcp help.go) — and the most damaging way a worker stops
    // is precisely the one that never reports `idle`. A wedged session (an
    // approval clobbered mid-turn, a driver blocked on a request nobody can
    // see) reports `streaming` forever, so gating on `ambientState === 'idle'`
    // made this watch structurally blind to the exact failure it was armed
    // for: every such watch was silently unfireable.
    //
    // `lastActivity` is what makes broadening it safe. It moves only on real
    // conversation deltas and ambient transitions and deliberately NOT on
    // statusLine ticks, so "nothing has arrived in N seconds" is a fact about
    // output whatever the session claims to be doing.
    //
    // The two cases still read differently, because they ARE different: a
    // session sitting at a prompt is done; one still claiming to work has
    // stalled. Naming the claimed state keeps the wake honest — a genuinely
    // long single tool call (a slow build) can trip this too, and the manager
    // can see that from the message rather than being told a lie about idling.
    const since = now - (s.lastActivity ?? now);
    if (since >= watch.idleSeconds * 1000) {
      const secs = Math.round(since / 1000);
      const settled = s.ambientState === undefined || s.ambientState === 'idle';
      return settled
        ? `idle for ${secs}s ≥ ${watch.idleSeconds}s`
        : `no activity for ${secs}s ≥ ${watch.idleSeconds}s (still reports ${s.ambientState})`;
    }
  }
  return null;
}

/** Canonical desktop/Hub wake percentage: bounded and exactly one decimal. */
export function formatContextPct(value: number): string {
  const bounded = Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
  // Spell out decimal half-up rounding so V8/Intl and Go do not disagree on
  // binary half-ties such as 80.05 and 12.35.
  return (Math.round(bounded * 10) / 10).toFixed(1);
}

function contextCrossing(threshold: number, h: ContextHealthReading): string {
  return (
    `contextUsedPct active context ${formatContextPct(h.usedPct)}% ≥ ${formatContextPct(threshold)}% ` +
    `(${h.usedTokens.toLocaleString('en-US')} / ${h.windowTokens.toLocaleString('en-US')} tokens; ` +
    `runtime-confirmed by ${h.provider}; observed ${h.observedAt}; epoch ${h.epoch})`
  );
}

function normalizedProvider(provider?: string): string | undefined {
  return provider?.trim().toLowerCase() || undefined;
}

function providerName(provider?: string): string {
  return normalizedProvider(provider) ?? 'unknown';
}

function assertContextWatchCanArm(s: WatchableSession, now: number): void {
  const provider = normalizedProvider(s.provider);
  // Hook-adopted Claude rows can exist before their provider enrichment lands.
  // Absence is not an unknown provider: keep the watch waiting and bind it only
  // when a trustworthy correlated sample names its owner.
  if (!provider) return;
  if (NO_CONTEXT_WINDOW_PROVIDERS.has(provider)) {
    throw new Error(
      `agents.notifyWhen: contextUsedPct is unavailable for provider ${provider}: it cannot emit a runtime context window`,
    );
  }
  if (!CONTEXT_HEALTH_PROVIDERS.has(provider) && !contextHealthReading(s, now)) {
    // Future providers remain extensible: a real correlated sample proves the
    // capability. Without one, waiting forever would consume a permanent slot.
    throw new Error(
      `agents.notifyWhen: contextUsedPct cannot wait for unknown provider ${provider} without a fresh runtime context sample`,
    );
  }
}

/** The single authoritative context predicate used by both production sweeps
 * and direct predicate tests. It includes ownership invalidation, not merely
 * the numeric comparison. */
function contextWatchCrossing(
  watch: ThresholdWatch,
  s: WatchableSession,
  now: number,
): string | null {
  const targetProvider = providerName(s.provider);
  const targetIdentity = normalizedProvider(s.provider);
  if (
    watch.contextProvider &&
    targetIdentity &&
    targetIdentity !== normalizedProvider(watch.contextProvider)
  ) {
    return `monitoring invalidated: contextUsedPct ${formatContextPct(watch.contextUsedPct!)}% watch crossed a provider/session boundary (${watch.contextProvider} → ${targetProvider}); re-arm after a confirmed sample`;
  }
  if (targetIdentity && NO_CONTEXT_WINDOW_PROVIDERS.has(targetIdentity)) {
    return `monitoring invalidated: contextUsedPct ${formatContextPct(watch.contextUsedPct!)}% is unavailable for provider ${targetProvider}; re-arm only on a provider with runtime context telemetry`;
  }
  const health = contextHealthReading(s, now);
  if (!health) return null;
  if (watch.contextEpoch !== undefined && watch.contextEpoch !== health.epoch) {
    return `monitoring invalidated: contextUsedPct ${formatContextPct(watch.contextUsedPct!)}% watch crossed telemetry epoch ${watch.contextEpoch} → ${health.epoch}; re-arm after the confirmed ${health.provider} sample`;
  }
  watch.contextProvider ??= health.provider;
  watch.contextEpoch ??= health.epoch;
  return health.usedPct >= watch.contextUsedPct!
    ? contextCrossing(watch.contextUsedPct!, health)
    : null;
}

/** Validate and normalize a caller's predicate. At least one threshold is
 *  required — an empty predicate is a watch that can never fire, which reads to
 *  the caller as "armed" and is worse than a refusal. */
export function parsePredicate(input: ThresholdPredicate): ThresholdPredicate {
  const out: ThresholdPredicate = {};
  for (const key of ['tokens', 'usd', 'idleSeconds'] as const) {
    const v = input[key];
    if (v === undefined || v === null) continue;
    const n = Number(v);
    if (!Number.isFinite(n) || n <= 0) {
      throw new Error(
        `agents.notifyWhen: ${key} must be a positive number, got ${JSON.stringify(v)}`,
      );
    }
    out[key] = n;
  }
  if (input.contextUsedPct !== undefined && input.contextUsedPct !== null) {
    const n = Number(input.contextUsedPct);
    if (!Number.isFinite(n) || n <= 0 || n > 100) {
      throw new Error(
        `agents.notifyWhen: contextUsedPct must be a finite number in (0, 100], got ${JSON.stringify(input.contextUsedPct)}`,
      );
    }
    if (Object.keys(out).length > 0) {
      throw new Error(
        'agents.notifyWhen: contextUsedPct is a single-purpose health predicate and cannot be combined with tokens, usd, or idleSeconds',
      );
    }
    out.contextUsedPct = n;
  }
  if (Object.keys(out).length === 0) {
    throw new Error(
      'agents.notifyWhen requires at least one threshold: tokens, usd, idleSeconds, or contextUsedPct',
    );
  }
  return out;
}

/** How a crossed watch is described back to the manager. */
function watchEntry(s: WatchableSession, crossed: string): FleetMessageEntry {
  return {
    label: s.label || basename(s.cwd) || 'Agent',
    sessionId: s.sessionId,
    cwd: s.cwd || '?',
    crossed,
  };
}

function basename(cwd?: string): string {
  if (!cwd) return '';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

type Deliver = (sessionId: string, text: string) => Promise<unknown>;

export class ThresholdWatcher {
  private watches = new Map<string, ThresholdWatch>();
  private timer?: NodeJS.Timeout;
  private seq = 0;

  constructor(
    private deliver: Deliver,
    private listSessions: () => WatchableSession[],
  ) {}

  /**
   * Arm a one-shot watch. Throws (rather than quietly no-opping) when the
   * target or the watcher is not a live session, or when the predicate is
   * empty: every failure mode here produces a caller that believes it is being
   * watched and is not.
   */
  arm(args: {
    sessionId: string;
    watcherSessionId: string;
    predicate: ThresholdPredicate;
    now?: number;
  }): ThresholdWatch {
    const predicate = parsePredicate(args.predicate);
    const sessions = this.listSessions();
    const target = sessions.find((s) => s.sessionId === args.sessionId);
    if (!target) throw new Error(`agents.notifyWhen: no such session ${args.sessionId}`);
    if (target.status === 'ended') {
      throw new Error(
        `agents.notifyWhen: session ${args.sessionId} has already ended — nothing left to watch`,
      );
    }
    const armedAt = args.now ?? Date.now();
    if (predicate.contextUsedPct !== undefined) assertContextWatchCanArm(target, armedAt);
    const watcher = sessions.find((s) => s.sessionId === args.watcherSessionId);
    if (!watcher || watcher.status === 'ended') {
      throw new Error(
        `agents.notifyWhen: notifySessionId ${args.watcherSessionId} is not a live session — a watch with no recipient would fire into nothing`,
      );
    }
    const mine = [...this.watches.values()].filter(
      (w) => w.watcherSessionId === args.watcherSessionId,
    );
    if (mine.length >= MAX_WATCHES_PER_WATCHER) {
      throw new Error(
        `agents.notifyWhen: ${args.watcherSessionId} already has ${mine.length} armed watches (max ${MAX_WATCHES_PER_WATCHER}) — let some fire, or stop arming in a loop`,
      );
    }
    const health =
      predicate.contextUsedPct !== undefined ? contextHealthReading(target, armedAt) : null;
    const watch: ThresholdWatch = {
      id: `w${++this.seq}`,
      sessionId: args.sessionId,
      watcherSessionId: args.watcherSessionId,
      armedAt,
      ...predicate,
    };
    if (predicate.contextUsedPct !== undefined) {
      watch.contextProvider = normalizedProvider(target.provider) ?? health?.provider;
      watch.contextEpoch = health?.epoch;
      watch.state = health
        ? health.usedPct >= predicate.contextUsedPct
          ? 'alreadySatisfied'
          : 'armed'
        : 'waitingForTelemetry';
    }
    this.watches.set(watch.id, watch);
    this.start();
    // Keep arm() semantically aligned with the Hub: this is an immutable
    // response snapshot, never the mutable value retained in watches.
    return snapshotWatch(watch);
  }

  /** Armed watches, for tests and for the arm() response's own accounting. */
  list(): ThresholdWatch[] {
    return [...this.watches.values()].map(snapshotWatch);
  }

  /**
   * Evaluate every armed watch. Pure over its inputs (`now` is passed in) so it
   * is trivially testable; the only side effect is delivery.
   *
   * A watch whose TARGET has ended is dropped, not fired: the finish wake
   * already told the manager, and a second "and by the way it cost $9" after
   * the fact is noise. A watch whose WATCHER has ended is dropped too — nobody
   * is listening.
   */
  sweep(now: number): void {
    if (this.watches.size === 0) return;
    const sessions = this.listSessions();
    const byId = new Map(sessions.map((s) => [s.sessionId, s]));
    const fired: Array<{ watch: ThresholdWatch; entry: FleetMessageEntry }> = [];

    for (const watch of [...this.watches.values()]) {
      const target = byId.get(watch.sessionId);
      const watcher = byId.get(watch.watcherSessionId);
      if (!target || target.status === 'ended' || !watcher || watcher.status === 'ended') {
        this.watches.delete(watch.id);
        continue;
      }
      const crossed = crossedBy(watch, target, now);
      if (!crossed) continue;
      this.watches.delete(watch.id); // one-shot: gone before delivery, never twice
      fired.push({ watch, entry: watchEntry(target, crossed) });
    }
    if (this.watches.size === 0) this.stop();
    if (fired.length === 0) return;

    // One wake per WATCHER, carrying every threshold that crossed this sweep —
    // the same coalescing every other fleet wake does.
    const byWatcher = new Map<string, FleetMessageEntry[]>();
    for (const f of fired) {
      const list = byWatcher.get(f.watch.watcherSessionId) ?? [];
      list.push(f.entry);
      byWatcher.set(f.watch.watcherSessionId, list);
    }
    for (const [watcherId, entries] of byWatcher) {
      void this.deliver(watcherId, buildFleetMessage('threshold', entries)).catch(() => {
        /* the watcher may have just ended — best-effort, exactly like the other wakes */
      });
    }
  }

  /** Start the sweep timer (idempotent). Unref'd so it never holds the process. */
  private start(): void {
    if (this.timer) return;
    this.timer = setInterval(() => {
      try {
        this.sweep(Date.now());
      } catch {
        /* a sweep failure must not kill the timer */
      }
    }, SWEEP_MS);
    this.timer.unref?.();
  }

  /** Stop sweeping when nothing is armed — no watches, no wakeups. */
  private stop(): void {
    if (!this.timer) return;
    clearInterval(this.timer);
    this.timer = undefined;
  }
}
