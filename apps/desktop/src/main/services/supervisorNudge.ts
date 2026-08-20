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

/** How long to coalesce nudges to one supervisor before sending. */
const COALESCE_MS = 1500;

interface PendingNudge {
  timer: NodeJS.Timeout;
  /** Session ids that blocked during this window (deduped). */
  blocked: Set<string>;
}

class SupervisorNudge {
  private pending = new Map<string, PendingNudge>();
  private pendingFinished = new Map<string, PendingNudge>();

  /**
   * Call when a session has just transitioned into a needs-you state. `kind`
   * is what it's blocked on; `supervisorIds` is every live supervisor session.
   */
  onBlock(
    session: ClaudeSessionState,
    kind: 'approval' | 'question',
    supervisorIds: string[],
  ): void {
    const supervisors = supervisorIds.filter((id) => id !== session.sessionId);
    if (supervisors.length === 0) return; // no supervisor → optional, nothing to do

    const label = session.label || agentLabel(session.cwd);
    for (const supId of supervisors) {
      const entry = this.pending.get(supId);
      if (entry) {
        entry.blocked.add(`${label} (session:${session.sessionId}, ${kind})`);
        continue;
      }
      const blocked = new Set<string>([`${label} (session:${session.sessionId}, ${kind})`]);
      const timer = setTimeout(() => {
        this.pending.delete(supId);
        void this.send(supId, blocked);
      }, COALESCE_MS);
      timer.unref?.();
      this.pending.set(supId, { timer, blocked });
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
   */
  onFinished(session: ClaudeSessionState, parentId: string, lastReply: string): void {
    if (parentId === session.sessionId) return;
    const label = session.label || agentLabel(session.cwd);
    const tail = lastReply ? ` — last reply: ${truncateReply(lastReply)}` : '';
    const line = `${label} (session:${session.sessionId}, cwd ${session.cwd || '?'})${tail}`;
    const entry = this.pendingFinished.get(parentId);
    if (entry) {
      entry.blocked.add(line);
      return;
    }
    const finished = new Set<string>([line]);
    const timer = setTimeout(() => {
      this.pendingFinished.delete(parentId);
      void this.sendFinished(parentId, finished);
    }, COALESCE_MS);
    timer.unref?.();
    this.pendingFinished.set(parentId, { timer, blocked: finished });
  }

  private async sendFinished(parentId: string, finished: Set<string>): Promise<void> {
    const list = Array.from(finished).join('; ');
    const text =
      `[fleet] Worker finished: ${list}. ` +
      `Review the result (get_conversation with sinceSeq for detail), append one line to that ` +
      `project's .workspacer/brief.md "## Recently" (and adjust "## Now"), then report the ` +
      `outcome briefly with session:<id> references. If it was not one of your dispatches, ` +
      `a one-line acknowledgement is enough.`;
    try {
      await claudemonSessionClient.message(parentId, text);
    } catch {
      /* the parent may have just ended — best-effort */
    }
  }

  private async send(supervisorId: string, blocked: Set<string>): Promise<void> {
    const list = Array.from(blocked).join('; ');
    const text =
      `[supervisor] An agent is now blocked on a decision: ${list}. ` +
      `Run a /supervise pass: gather the context and notify me with a recommendation.`;
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

/** Keep the wake message readable: one flattened, capped excerpt. */
function truncateReply(reply: string): string {
  const flat = reply.replace(/\s+/g, ' ').trim();
  return flat.length > 400 ? `${flat.slice(0, 400)}…` : flat;
}

/** Basename of the working directory, as a fallback agent label. */
function agentLabel(cwd: string): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

export const supervisorNudge = new SupervisorNudge();
