/**
 * Event-driven supervisor wake. When an agent transitions *into* a blocked
 * state (pending approval / question), nudge every live supervisor session with
 * a short message so it runs a /supervise pass and surfaces the decision
 * immediately — instead of waiting up to a poll interval.
 *
 * Fully optional: if no session is marked a supervisor, this is a no-op. A
 * supervisor is never nudged about its own block, and nudges are coalesced per
 * supervisor over a short window so a burst of blocking agents produces one
 * wake, not a storm.
 */

import { claudemonSessionClient } from './claudemonSessionClient';
import type { ClaudeSessionState } from './claudeSessionStore';
import { buildFleetMessage, excerptReply, type FleetMessageEntry } from '../shared/fleetMessages';
import { readStructuredResult } from '../shared/structuredResult';
import { workerFailureReason } from '../shared/workerFailure';

/** How long to coalesce nudges to one supervisor before sending. */
const COALESCE_MS = 1500;

/**
 * How long a block must SURVIVE before it wakes a supervisor at all. Most
 * approval/question blocks clear on their own within seconds (an auto-approve
 * hook, a fast human), and each wake costs the supervisor a full turn of
 * context — waking it for a block that was about to clear anyway trains the
 * standing doctrine into "fire one blind approve and stay silent" instead of
 * actually reading the decision. Below this, onBlockCleared cancels the wake
 * outright; only a block still open when the timer fires ever reaches the
 * coalesce path.
 */
const BLOCK_DEBOUNCE_MS = 20_000;

/**
 * A worker's finish must be OLDER than this before the backstop treats a
 * still-idle manager as having MISSED the wake — long enough that the normal
 * onFinished path (coalesce + deliver + the manager's own response) has had
 * every chance to land. Below it, an idle manager is just mid-handoff.
 */
const MISSED_WAKE_GRACE_MS = 3 * 60_000;

interface PendingNudge {
  timer: NodeJS.Timeout;
  /** Entries accumulated during this window, deduped by worker session id.
   *  Structured (not preformatted text) so the wake goes out through the
   *  shared fleetMessages builder — the format the GUI's card parser pins. */
  entries: Map<string, FleetMessageEntry>;
}

/** What the finished path needs from a worker session at DELIVERY time. The
 *  store's session objects mutate in place, so holding the reference lets the
 *  coalesce-window flush re-read the live truth (state flips, the final
 *  assistant message landing late) instead of trusting the schedule-time
 *  snapshot. All re-checked fields are optional so a caller that can't provide
 *  them (or a session evicted mid-window) degrades to the scheduled entry. */
type FinishedWorker = Pick<ClaudeSessionState, 'sessionId'> &
  Partial<
    Pick<
      ClaudeSessionState,
      'cwd' | 'label' | 'ambientState' | 'status' | 'conversation' | 'resultSchema' | 'statusLine'
    >
  >;

interface PendingFinish {
  timer: NodeJS.Timeout;
  /** Scheduled entry + the live session it was built from, per worker id. */
  workers: Map<string, { entry: FleetMessageEntry; session: FinishedWorker }>;
}

/** The worker's newest assistant reply, from its live conversation. */
function lastAssistantReply(session: FinishedWorker): string {
  const conv = session.conversation ?? [];
  for (let i = conv.length - 1; i >= 0; i--) {
    if (conv[i].role === 'assistant' && conv[i].content) return conv[i].content;
  }
  return '';
}

/** Whether the session has received at least one real user/task turn. A
 *  session whose conversation holds NO user turn has not been given its task
 *  yet (the parent's kickoff message is still in flight) — its idle is a boot
 *  idle, not a finish, and waking the manager about it tricks it into reading
 *  an empty session. Unknown conversation (untracked session) fails OPEN. */
function hasReceivedTask(session: FinishedWorker): boolean {
  return !session.conversation || session.conversation.some((t) => t.role === 'user');
}

class SupervisorNudge {
  private pending = new Map<string, PendingNudge>();
  private pendingFinished = new Map<string, PendingFinish>();
  /** Debounce timers for a block that has not yet survived BLOCK_DEBOUNCE_MS,
   *  keyed by the BLOCKED session's id (not the supervisor — a worker can only
   *  be blocked once at a time, so one timer per worker is enough to cover
   *  every supervisor it would eventually wake). */
  private pendingBlocks = new Map<string, NodeJS.Timeout>();
  /**
   * The signature (reply + stopped + failed) of the last finished-wake actually
   * delivered for this worker, keyed by worker session id — PER_TURN_WAKE_FINDING.md.
   * A working→idle edge is a genuine finish ONLY the first time; a repeat edge
   * (a block that flaps, a re-derived Stop) with the identical reply/status is
   * noise, not a new report, and must not re-wake the parent. A follow-up that
   * produces an actually different reply — the wanted case, a manager sending a
   * worker a second instruction — always looks different from the last
   * signature and still fires. Cleared on SessionEnd via forgetWorker so it
   * doesn't retain an entry per session for the process lifetime.
   */
  private lastReportedReply = new Map<string, string>();

  /**
   * Call when a session has just transitioned into a needs-you state. `kind`
   * is what it's blocked on; `supervisorIds` is every live supervisor session.
   *
   * Debounced: the broadcast (and its own COALESCE_MS coalescing) only fires
   * if the block is still open BLOCK_DEBOUNCE_MS later — see onBlockCleared,
   * which the caller must invoke on the matching clear edge so a block that
   * resolves itself never wakes anyone. Re-blocking after a clear (or a second
   * onBlock before the debounce fires) replaces any existing timer for this
   * session rather than stacking one, so a flapping block can never leak a
   * timer or double-fire.
   */
  onBlock(
    session: ClaudeSessionState,
    kind: 'approval' | 'question',
    supervisorIds: string[],
  ): void {
    const supervisors = supervisorIds.filter((id) => id !== session.sessionId);
    if (supervisors.length === 0) return; // no supervisor → optional, nothing to do

    const existing = this.pendingBlocks.get(session.sessionId);
    if (existing) clearTimeout(existing);

    const timer = setTimeout(() => {
      this.pendingBlocks.delete(session.sessionId);
      this.broadcastBlock(session, kind, supervisors);
    }, BLOCK_DEBOUNCE_MS);
    timer.unref?.();
    this.pendingBlocks.set(session.sessionId, timer);
  }

  /**
   * Call when a session transitions OUT of a needs-you state (approval given,
   * question answered, the session ended). Cancels any debounce timer still
   * waiting on this session's block, so a block that clears before
   * BLOCK_DEBOUNCE_MS never reaches a supervisor at all. A no-op when nothing
   * is pending (the block already survived and broadcast, or there never was
   * one) — safe to call unconditionally on every un-block edge.
   */
  onBlockCleared(sessionId: string): void {
    const timer = this.pendingBlocks.get(sessionId);
    if (!timer) return;
    clearTimeout(timer);
    this.pendingBlocks.delete(sessionId);
  }

  /** The broadcast itself, run once a block has survived the debounce. Same
   *  per-supervisor coalescing as before: a burst of blocks arriving inside
   *  one supervisor's COALESCE_MS window still produces one wake. */
  private broadcastBlock(
    session: ClaudeSessionState,
    kind: 'approval' | 'question',
    supervisors: string[],
  ): void {
    const blockedEntry: FleetMessageEntry = {
      label: session.label || agentLabel(session.cwd),
      sessionId: session.sessionId,
      blockedOn: kind,
    };
    for (const supId of supervisors) {
      const entry = this.pending.get(supId);
      if (entry) {
        entry.entries.set(session.sessionId, blockedEntry);
        continue;
      }
      const entries = new Map([[session.sessionId, blockedEntry]]);
      const timer = setTimeout(() => {
        this.pending.delete(supId);
        void this.send(supId, entries);
      }, COALESCE_MS);
      timer.unref?.();
      this.pending.set(supId, { timer, entries });
    }
  }

  /**
   * Call when a session transitions working→idle — its turn is DONE. Unlike
   * blocks (broadcast to every supervisor), a finish routes ONLY to the
   * session's PARENT: it is the parent's dispatch coming home, and waking
   * unrelated supervisors about it is noise. This wake is what lets a Fleet
   * Manager be a pure delegator — it never polls its workers, it gets told
   * (FLEET_MANAGER_SPIKE.md gap #2). Coalesced like blocks so a burst of
   * finishing workers produces one wake.
   *
   * Fires only on a GENUINE finish: a worker that has never received its task
   * turn is skipped here (boot idle), and every worker is re-verified against
   * its live session when the coalesce window closes (see sendFinished) so an
   * idle blip mid-stream never reports a half-done result as final.
   */
  onFinished(session: ClaudeSessionState, parentId: string, lastReply: string): void {
    if (parentId === session.sessionId) return;
    // Misfire guard: a freshly spawned worker idles once BEFORE the parent's
    // task message is delivered — that boot idle is not a finish.
    if (!hasReceivedTask(session)) return;
    const finishedEntry: FleetMessageEntry = {
      label: session.label || agentLabel(session.cwd),
      sessionId: session.sessionId,
      cwd: session.cwd || '?',
      ...(lastReply ? { lastReply: excerptReply(lastReply) } : {}),
    };
    const pending = this.pendingFinished.get(parentId);
    if (pending) {
      pending.workers.set(session.sessionId, { entry: finishedEntry, session });
      return;
    }
    const workers = new Map([[session.sessionId, { entry: finishedEntry, session }]]);
    const timer = setTimeout(() => {
      this.pendingFinished.delete(parentId);
      void this.sendFinished(parentId, workers);
    }, COALESCE_MS);
    timer.unref?.();
    this.pendingFinished.set(parentId, { timer, workers });
  }

  /**
   * Re-address a finished-wake that is still inside its coalesce window from a
   * retiring manager to its successor — the queued-wake half of
   * claudeSessionStore.reparentChildren.
   *
   * `pendingFinished` is keyed by PARENT id, so a worker that finished in the
   * seconds before a handoff has its report addressed to the manager on its way
   * out. Everything under `oldParentId` is by construction a child of the old
   * manager (onFinished keys off the child's own parentSessionId), so the whole
   * bucket moves. Merging into an existing window for the successor dedups by
   * worker id; otherwise the window restarts, costing at most COALESCE_MS.
   * No-op when nothing is queued.
   */
  reassignPendingFinish(oldParentId: string, newParentId: string): void {
    if (oldParentId === newParentId) return;
    const pending = this.pendingFinished.get(oldParentId);
    if (!pending) return;
    clearTimeout(pending.timer);
    this.pendingFinished.delete(oldParentId);
    const existing = this.pendingFinished.get(newParentId);
    if (existing) {
      for (const [workerId, worker] of pending.workers) existing.workers.set(workerId, worker);
      return;
    }
    const workers = pending.workers;
    const timer = setTimeout(() => {
      this.pendingFinished.delete(newParentId);
      void this.sendFinished(newParentId, workers);
    }, COALESCE_MS);
    timer.unref?.();
    this.pendingFinished.set(newParentId, { timer, workers });
  }

  /**
   * BACKSTOP for a dropped wake (the "dark manager" failure): the onFinished
   * path is best-effort — if its message never lands, or a working→idle edge
   * was never observed, a manager can sit idle forever while a dispatched
   * worker has finished. Run periodically over all sessions; for each LIVE,
   * IDLE manager, find children that have finished (idle/ended) AFTER the
   * manager last acted and long enough ago that a normal wake would have
   * landed, and re-nudge.
   *
   * The dedup is implicit and exact: the moment the manager acts on the wake it
   * reports and its lastActivity advances PAST the child's finish, so the
   * condition clears — no acknowledgement bookkeeping to drift. A catch-up that
   * itself fails simply re-fires next sweep. Pure over its inputs (`now` passed
   * in) so it is trivially testable.
   */
  sweepMissedFinishes(
    sessions: Array<
      Pick<
        ClaudeSessionState,
        'sessionId' | 'cwd' | 'label' | 'ambientState' | 'lastActivity' | 'parentSessionId'
      > & { isSupervisor?: boolean; status?: ClaudeSessionState['status'] } & Partial<
          Pick<ClaudeSessionState, 'conversation' | 'statusLine'>
        >
    >,
    now: number,
  ): void {
    const managers = sessions.filter(
      (s) => s.isSupervisor && s.status !== 'ended' && s.ambientState === 'idle',
    );
    if (managers.length === 0) return;
    for (const manager of managers) {
      const missed = sessions.filter(
        (c) =>
          c.parentSessionId === manager.sessionId &&
          c.sessionId !== manager.sessionId &&
          (c.ambientState === 'idle' || c.status === 'ended') &&
          // Same no-task gate as onFinished: a child idling with no user turn
          // was never given its task — nothing finished, nothing to catch up.
          hasReceivedTask(c) &&
          // The manager has not acted since this child finished…
          c.lastActivity > manager.lastActivity &&
          // …and the finish is old enough that a normal wake would have landed.
          now - c.lastActivity > MISSED_WAKE_GRACE_MS,
      );
      if (missed.length === 0) continue;
      const entries = missed.map((c): FleetMessageEntry => {
        // The catch-up path must tell finished from died too — a manager that
        // missed the live wake is exactly the one most likely to book a crash
        // as an outcome.
        const failure = workerFailureReason(c, lastAssistantReply(c));
        return {
          label: c.label || agentLabel(c.cwd),
          sessionId: c.sessionId,
          cwd: c.cwd || '?',
          ...(c.status === 'ended' ? { stopped: true } : {}),
          ...(failure ? { failed: failure } : {}),
        };
      });
      void this.sendCatchUp(manager.sessionId, entries);
    }
  }

  private async sendCatchUp(parentId: string, entries: FleetMessageEntry[]): Promise<void> {
    try {
      await claudemonSessionClient.message(parentId, buildFleetMessage('catch-up', entries));
    } catch {
      /* still unreachable — the next sweep retries */
    }
  }

  /**
   * Deliver a coalesced finished-wake, re-verifying each worker against its
   * LIVE session first. The working→idle edge that scheduled the wake can lie
   * twice: an idle blip mid-stream (the worker was back to streaming before
   * the coalesce window closed — reporting that as finished would present a
   * half-done result as final), and a final assistant message that lands on
   * the conversation stream AFTER the Stop edge (claudemon keeps tailing
   * briefly). So at delivery: drop any worker that is working again (its real
   * finish re-fires the edge later), re-read the reply from the live
   * conversation, mark ended sessions stopped/killed, and carry the COMPLETE
   * final message (capped, truncation announced) so the manager never fetches
   * a whole conversation just to read a report.
   */
  private async sendFinished(
    parentId: string,
    workers: Map<string, { entry: FleetMessageEntry; session: FinishedWorker }>,
  ): Promise<void> {
    const entries: FleetMessageEntry[] = [];
    /** Signatures to book as reported — applied only after the send lands. */
    const delivered: Array<[string, string]> = [];
    for (const { entry, session } of workers.values()) {
      const genuinelyIdle =
        session.status === 'ended' ||
        session.ambientState === undefined ||
        session.ambientState === 'idle';
      if (!genuinelyIdle) continue; // resumed working — not a finish after all
      if (!hasReceivedTask(session)) continue; // still no task turn — boot idle
      if (session.status === 'ended') entry.stopped = true;
      // Fall back to the schedule-time excerpt when the live conversation has
      // no assistant turn to re-read (untracked or already-evicted session).
      const reply = lastAssistantReply(session) || entry.lastReply || '';
      if (reply) {
        entry.lastReply = excerptReply(reply);
        // Carry the complete message only when the excerpt is lossy —
        // otherwise the bullet already IS the whole reply.
        if (entry.lastReply !== reply.trim()) entry.fullReply = reply;
        else delete entry.fullReply;
      }
      // Did it FINISH or did it DIE? A provider error (out of credits, an
      // overload) idles a worker exactly like a completed task and leaves the
      // error text as its last reply — so without this the wake said "Worker
      // finished" and handed the manager a crash as a summary. Detected from
      // the same reply, on a separate axis from `stopped` (session ENDED),
      // because an error can arrive with the session still alive.
      const failure = workerFailureReason(session, reply);
      if (failure) entry.failed = failure;
      // A dispatch that asked for a machine-readable result gets it validated
      // HERE, against the schema recorded at spawn, from the same final message
      // the prose comes from. Strictly additive: success adds the object,
      // failure adds a one-line reason, and neither touches lastReply/fullReply
      // — the manager always still receives what the worker actually wrote.
      if (session.resultSchema) {
        const outcome = readStructuredResult(reply, session.resultSchema);
        if (outcome.json) entry.result = outcome.json;
        else if (outcome.error) entry.resultError = outcome.error;
      }
      // Nothing new to report: this edge produced the exact reply/status
      // already delivered for this worker — a flapping block or a re-derived
      // Stop with no fresh output (PER_TURN_WAKE_FINDING.md 1b). A genuinely
      // different reply, a fresh stop, or a newly-surfaced failure always
      // changes the signature and still wakes the parent (1a).
      const signature = `${reply} ${entry.stopped ? 1 : 0} ${entry.failed ?? ''}`;
      if (this.lastReportedReply.get(session.sessionId) === signature) continue;
      delivered.push([session.sessionId, signature]);
      entries.push(entry);
    }
    if (entries.length === 0) return;
    const text = buildFleetMessage('worker-finished', entries);
    try {
      await claudemonSessionClient.message(parentId, text);
    } catch {
      /* the parent may have just ended — best-effort */
      return; // NOT delivered: leave the signatures unrecorded (see below)
    }
    // Record the signatures only once the wake is actually on the wire. The
    // suppression means "the parent has already been told this", so booking it
    // on a send that THREW would turn a lost wake into a permanently silenced
    // one — the next identical edge would dedup against a report nobody ever
    // received. The opposite symptom (wakes going missing) is as real as the
    // duplicate this dedup exists to kill — PER_TURN_WAKE_FINDING.md — so the
    // failure mode has to fall on the side of re-reporting.
    for (const [sessionId, signature] of delivered)
      this.lastReportedReply.set(sessionId, signature);
  }

  /** Drop the dedup signature for a worker whose life has ended, so
   *  `lastReportedReply` doesn't retain one entry per session for the whole
   *  process lifetime (same concern as claudeSessionStore's per-session
   *  Maps — see its evictNow). A respawn onto a reused id starts fresh. */
  forgetWorker(sessionId: string): void {
    this.lastReportedReply.delete(sessionId);
  }

  private async send(supervisorId: string, entries: Map<string, FleetMessageEntry>): Promise<void> {
    const text = buildFleetMessage('blocked', Array.from(entries.values()));
    try {
      // claudemon's /message queues while the supervisor is busy (or a dialog
      // is up) and delivers once its prompt settles — no raw-PTY fallback
      // needed (typing into an open dialog could answer it by accident). A
      // rejection means the supervisor session has ended; nothing to do.
      await claudemonSessionClient.message(supervisorId, text);
    } catch {
      /* the supervisor may have just ended — best-effort */
    }
  }
}

/** Basename of the working directory, as a fallback agent label. */
function agentLabel(cwd: string): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

export const supervisorNudge = new SupervisorNudge();
