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
 * `notify_when(session, {tokens|usd|idleSeconds})` arms a one-shot watch;
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

export interface ThresholdPredicate {
  /** Fire when the session's cumulative tokens (input + output) reach this. */
  tokens?: number;
  /** Fire when the session's cumulative cost in USD reaches this. */
  usd?: number;
  /** Fire when nothing has arrived from the session for this many seconds —
   *  whether it is sitting at a prompt or claiming to still be working (see
   *  `crossedBy`: a wedged session reports `streaming` forever). */
  idleSeconds?: number;
}

export interface ThresholdWatch extends ThresholdPredicate {
  id: string;
  /** The session being watched. */
  sessionId: string;
  /** The session to wake when it crosses — the manager that armed the watch. */
  watcherSessionId: string;
  armedAt: number;
}

/** What the sweep needs of a session. A structural subset of
 *  ClaudeSessionState, so the store's rows satisfy it as-is. */
export interface WatchableSession {
  sessionId: string;
  cwd?: string;
  label?: string;
  status?: string;
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
  };
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
  if (Object.keys(out).length === 0) {
    throw new Error(
      'agents.notifyWhen requires at least one threshold: tokens, usd, or idleSeconds',
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
    const watch: ThresholdWatch = {
      id: `w${++this.seq}`,
      sessionId: args.sessionId,
      watcherSessionId: args.watcherSessionId,
      armedAt: args.now ?? Date.now(),
      ...predicate,
    };
    this.watches.set(watch.id, watch);
    this.start();
    return watch;
  }

  /** Armed watches, for tests and for the arm() response's own accounting. */
  list(): ThresholdWatch[] {
    return [...this.watches.values()];
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
