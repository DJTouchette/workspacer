/**
 * Stall detection — "this agent says it's working, but nothing is happening."
 *
 * The hard part is that a working agent legitimately goes quiet. A long
 * extended-thinking block produces no conversation items at all (the stream
 * adapter drops thinking deltas by design), so "no items lately" on its own
 * would flag every deep think as broken. And `lastActivity` can't carry this
 * alone either: it moves on hook events and conversation deltas, which is most
 * progress but not all of it.
 *
 * So progress is defined as a *fingerprint* over everything observable that
 * moves when work is really happening — the conversation, tool calls, token
 * spend, and the state of any spawned subagents and workflow agents. If not one
 * of those has changed in a long while and the agent still claims to be working,
 * that is worth a human's eyes. A workflow gets the same treatment against its
 * own agents' progress, because a run whose agents are all parked looks exactly
 * as busy as one that's flying.
 *
 * There is a second, independent signal: the status line. It keeps ticking while
 * the process is alive and deliberately does NOT bump `lastActivity` (see
 * `claudeSessionStore.applyStatusLine`), so it separates "alive but silent" —
 * probably thinking, possibly wedged on a long API call — from "no signal at
 * all", which is a process that has stopped talking to us. Both are reported;
 * the second is the more damning.
 *
 * Everything here is pure. The caller owns the elapsed-time memory (see
 * `trackProgress`) so this stays unit-testable without a clock.
 */

import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';

/**
 * How long an agent may show no observable progress before it's worth flagging.
 *
 * Generous on purpose: a single extended-thinking block can run for minutes with
 * nothing to show for it, and a false "stuck" badge trains people to ignore the
 * real ones. Erring long costs a few minutes of noticing; erring short costs the
 * signal's credibility.
 */
export const STALL_MS = 5 * 60_000;

/** A workflow's agents can be legitimately quiet longer — a single agent's turn
 *  is itself a whole session's worth of work — so the run gets more rope. */
export const WORKFLOW_STALL_MS = 8 * 60_000;

/** How long without a status-line tick before we call the process itself silent.
 *  The line ticks every few seconds while alive, so this is generous too. */
const SILENT_MS = 90_000;

/** States where the agent is claiming to be doing something. An idle or blocked
 *  agent isn't stalled — it's finished, or waiting on you, which the feed's
 *  other kinds already say better. */
const WORKING: SessionAmbientState[] = ['thinking', 'streaming', 'background'];

export function isWorkingState(state: SessionAmbientState | undefined): boolean {
  return !!state && WORKING.includes(state);
}

/**
 * Everything observable that moves when real work is happening, as one string.
 *
 * Counts and totals rather than contents: cheap to build on every snapshot, and
 * it only has to answer "did anything advance", not "what changed". Token spend
 * is in here because it advances during a turn that produces no items yet, which
 * is exactly the deep-thinking case a naive check gets wrong.
 */
export function progressFingerprint(snap: ClaudeSessionSnapshot): string {
  const parts: (string | number)[] = [
    snap.conversation?.length ?? 0,
    snap.activeToolCalls?.length ?? 0,
    snap.completedToolCalls?.length ?? 0,
    snap.fileChanges?.length ?? 0,
    snap.totalToolCalls ?? 0,
    // Spend moves mid-turn even when nothing else does.
    snap.statusLine?.totalOutputTokens ?? snap.usage?.totalOutputTokens ?? 0,
    snap.statusLine?.totalInputTokens ?? snap.usage?.totalInputTokens ?? 0,
    // Spawned work: a parent can be idle-looking while its children advance.
    (snap.subagents ?? [])
      .map((s) => `${s.id}${s.status}${s.toolCalls ?? 0}${s.tokens ?? 0}`)
      .join(','),
    workflowFingerprint(snap),
  ];
  return parts.join('|');
}

/** The workflow half of the fingerprint, on its own so a run can be judged
 *  against its own progress rather than the whole session's. */
export function workflowFingerprint(snap: ClaudeSessionSnapshot): string {
  return (snap.workflows ?? [])
    .map(
      (w) =>
        `${w.runId}${w.status}${w.totalToolCalls ?? 0}${w.totalTokens ?? 0}:` +
        (w.agents ?? [])
          .map(
            (a) => `${a.id}${a.status}${a.toolCalls ?? 0}${a.tokens ?? 0}${a.lastToolName ?? ''}`,
          )
          .join(','),
    )
    .join(';');
}

/** When a fingerprint was last seen to change. */
export interface ProgressMark {
  fingerprint: string;
  /** Timestamp the fingerprint last differed from the one before it. */
  since: number;
}

export type ProgressMarks = Map<string, ProgressMark>;

/**
 * Fold a round of snapshots into the progress marks, returning a new map.
 *
 * A session whose fingerprint changed gets `since = now`; one that didn't keeps
 * the mark it had, which is what makes the elapsed time meaningful. Sessions
 * that have gone away are dropped so the map can't grow forever.
 *
 * Returns the *same* map instance when nothing changed at all, so callers can
 * use it as a render dependency without churning.
 */
export function trackProgress(
  prev: ProgressMarks,
  snapshots: Record<string, ClaudeSessionSnapshot>,
  now: number,
  fingerprintOf: (snap: ClaudeSessionSnapshot) => string = progressFingerprint,
): ProgressMarks {
  let mutated = false;
  const next: ProgressMarks = new Map();
  for (const [sid, snap] of Object.entries(snapshots)) {
    const fingerprint = fingerprintOf(snap);
    const before = prev.get(sid);
    if (before && before.fingerprint === fingerprint) {
      next.set(sid, before);
      continue;
    }
    next.set(sid, { fingerprint, since: now });
    mutated = true;
  }
  if (!mutated && next.size === prev.size) return prev;
  return next;
}

export interface StallVerdict {
  /** How long nothing has moved. */
  stalledForMs: number;
  /**
   * Whether the process is still talking to us at all (status line ticking).
   * `false` is the more serious reading: not just quiet, but silent.
   */
  alive: boolean;
}

/**
 * Whether an agent has stalled, and how badly.
 *
 * `null` for an agent that isn't claiming to work, hasn't been tracked yet, or
 * is still inside the grace period. The caller supplies the threshold so a
 * workflow can be judged on a longer fuse than a turn.
 */
export function stallOf(
  snap: ClaudeSessionSnapshot,
  mark: ProgressMark | undefined,
  now: number,
  thresholdMs: number = STALL_MS,
): StallVerdict | null {
  if (!mark || !isWorkingState(snap.ambientState)) return null;
  const stalledForMs = now - mark.since;
  if (stalledForMs < thresholdMs) return null;
  return { stalledForMs, alive: statusLineAlive(snap, now) };
}

/** True when the status line has ticked recently enough to call the process
 *  alive. Absent status line → unknown, and we don't claim silence we can't
 *  observe (managed providers other than claude publish none). */
function statusLineAlive(snap: ClaudeSessionSnapshot, now: number): boolean {
  const at = snap.statusLine?.receivedAt;
  if (!at) return true;
  const ts = Date.parse(at);
  if (Number.isNaN(ts)) return true;
  return now - ts < SILENT_MS;
}

/** Running workflow runs on this session, for the per-run stall check. */
export function runningWorkflows(snap: ClaudeSessionSnapshot) {
  return (snap.workflows ?? []).filter((w) => w.status === 'running');
}

/** Human phrasing for an elapsed stall — "6m", "1h 4m". Deliberately plain:
 *  the card states an observation and lets the reader draw the conclusion. */
export function stalledFor(ms: number): string {
  const mins = Math.floor(ms / 60_000);
  if (mins < 60) return `${Math.max(1, mins)}m`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}
