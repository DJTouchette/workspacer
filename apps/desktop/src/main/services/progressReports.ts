/**
 * `report_progress` — the one thing a dispatched worker may say to the manager
 * that dispatched it, and nothing else.
 *
 * WHY IT EXISTS. Before this, the only way a worker could reach its manager
 * mid-task was to be spawned at `toolScope: "triage"` or `"operator"` — tiers
 * that also hand it approve / interrupt / reply over OTHER sessions. A Rust
 * worker should not need the power to approve another agent just to say "I'm at
 * 70% context and haven't started implementing yet". The motivating failure is
 * concrete: on 2026-08-22 a worker spent its entire 200K window reading code,
 * wrote nothing, and died at 198,926 tokens with everything uncommitted. Its
 * last words were "Now I have the full picture. Let me start implementing." One
 * self-report would have saved it.
 *
 * WHAT IT IS NOT. It is not a second `send_message`. The recipient is NEVER a
 * parameter: the caller's own session id is stamped host-side (the facade reads
 * it from the per-request token record's `session:<id>` label; the hub router
 * stamps it for a scoped bus connection), and the recipient is derived from that
 * — the caller's PARENT, or a refusal. A worker holding this capability cannot
 * name a session at all, so there is no session it can reach but the one that
 * dispatched it.
 *
 * It is also not `notify_when`. Thresholds (tokens / usd / idle) are covered
 * host-side and are strictly better there, because the worker cannot forget to
 * send them. What only the worker knows is SEMANTIC: "finished phase 1", "the
 * approach you gave me is wrong", "I'm reading more than I expected".
 *
 * NOISE. These arrive UNSOLICITED, at a manager whose doctrine is never to
 * poll, and interrupt whatever it was doing. So every bound below refuses out
 * loud rather than truncating or silently dropping: a worker that believes it
 * reported and did not is exactly the failure this tool exists to prevent.
 */

import { buildFleetMessage, type FleetMessageEntry } from '../shared/fleetMessages';

/**
 * Longest progress line accepted, after whitespace flattening. A progress
 * update is a LINE, not a report — the manager is being interrupted, so the
 * update has to be worth the interruption at a glance. Deliberately just above
 * REPLY_EXCERPT_MAX (400): a worker with more than this to say has finished a
 * phase, and the place for that is its final report, which the finish wake
 * already carries in full.
 */
export const NOTE_MAX = 500;

/**
 * Minimum gap between one worker's reports. Stops a burst — a worker narrating
 * every tool call would turn the manager's context into a log. A minute is long
 * enough that no reasonable phase boundary is lost and short enough that a
 * worker realizing mid-turn that its dispatch is wrong is not gagged.
 */
export const MIN_INTERVAL_MS = 60_000;

/**
 * Hard cap on reports from one worker, for its whole life. The interval alone
 * bounds the RATE but not the TOTAL: at one a minute a three-hour worker could
 * deliver 180 wakes, which is a firehose no manager reads. Twenty is generous
 * for phase-by-phase reporting on a long dispatch and small enough to stay
 * readable if a worker spends every one of them.
 */
export const MAX_REPORTS = 20;

/** What a report needs of a session. A structural subset of ClaudeSessionState,
 *  so the store's rows satisfy it as-is. */
export interface ReportableSession {
  sessionId: string;
  cwd?: string;
  label?: string;
  status?: string;
  parentSessionId?: string;
}

export interface ProgressReportInput {
  /** The CALLER's session — stamped host-side from its credential, never taken
   *  from the caller's own params. See the module comment. */
  callerSessionId?: string;
  note?: string;
  needsDecision?: boolean;
}

interface Budget {
  count: number;
  lastAt: number;
  lastNote: string;
}

/** Basename of the working directory, as a fallback worker label. Mirrors
 *  supervisorNudge's agentLabel — the wake bullets have to look the same
 *  whether the host or the worker itself produced them. */
function agentLabel(cwd: string | undefined): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

/** One line, always: the bullet grammar is line-based, so a multi-line note
 *  would break the card parser (and read as a wall of text besides). */
export function flattenNote(note: string): string {
  return note.replace(/\s+/g, ' ').trim();
}

export class ProgressReports {
  private budgets = new Map<string, Budget>();

  constructor(
    private deliver: (sessionId: string, text: string) => Promise<unknown>,
    private sessions: () => ReportableSession[],
  ) {}

  /**
   * Deliver one worker's progress line to its parent. Resolves with the
   * recipient on success; THROWS a caller-readable reason on every refusal —
   * the worker sees it as a tool error and can shorten, wait, or stop trying.
   */
  async report(input: ProgressReportInput, now = Date.now()): Promise<{ deliveredTo: string }> {
    const callerId = (input.callerSessionId || '').trim();
    if (!callerId) {
      // Only reachable from a credential with no session identity (the static
      // MCP token, the untokened loopback default, a plugin). Say so rather
      // than picking a recipient — guessing here is exactly the containment
      // hole this tool is defined not to have.
      throw new Error(
        'report_progress: the host could not identify your session from your credential, so it cannot tell who dispatched you. This tool is for agents workspacer spawned; use send_message if you are driving the fleet.',
      );
    }
    const note = flattenNote(String(input.note ?? ''));
    if (!note) throw new Error('report_progress requires a non-empty note');
    if (note.length > NOTE_MAX) {
      throw new Error(
        `report_progress: note is ${note.length} characters; the limit is ${NOTE_MAX}. This is a progress LINE, not a report — say what changed for your manager's decision (a phase finished, the approach is wrong, the budget is running out) and leave the detail for your final message, which the finish wake delivers in full.`,
      );
    }

    const all = this.sessions();
    const me = all.find((s) => s.sessionId === callerId);
    if (!me) throw new Error(`report_progress: session ${callerId} is not a tracked session`);
    const parentId = me.parentSessionId;
    if (!parentId || parentId === callerId) {
      throw new Error(
        'report_progress: you have no parent session — nothing dispatched you, so there is nobody to report to. Tell the user directly in your reply instead.',
      );
    }
    const parent = all.find((s) => s.sessionId === parentId);
    if (!parent || parent.status === 'ended') {
      throw new Error(
        `report_progress: your parent session (${parentId}) has ended — there is nobody to receive this. Carry on and put it in your final message.`,
      );
    }

    const budget = this.budgets.get(callerId) ?? { count: 0, lastAt: 0, lastNote: '' };
    if (budget.count >= MAX_REPORTS) {
      throw new Error(
        `report_progress: you have already sent ${MAX_REPORTS} progress updates, which is the limit for one session. Stop reporting and finish the task — your final message reaches your manager in full.`,
      );
    }
    if (note === budget.lastNote) {
      // A retry loop double-waking the manager with the same sentence is the
      // cheapest way to make this channel unreadable.
      throw new Error(
        'report_progress: that is the same note you just sent; it was NOT delivered again.',
      );
    }
    const since = now - budget.lastAt;
    if (budget.count > 0 && since < MIN_INTERVAL_MS) {
      throw new Error(
        `report_progress: you reported ${Math.round(since / 1000)}s ago; updates are limited to one per ${MIN_INTERVAL_MS / 1000}s. This one was NOT delivered — carry on working and fold it into your next update.`,
      );
    }

    const entry: FleetMessageEntry = {
      label: me.label || agentLabel(me.cwd),
      sessionId: callerId,
      cwd: me.cwd || '?',
      note,
      ...(input.needsDecision ? { needsDecision: true } : {}),
    };
    // Charge the budget BEFORE delivery: a failed send still consumed the
    // manager's attention budget as far as the worker is concerned, and a
    // worker that retries a failing send in a loop must still hit the cap.
    this.budgets.set(callerId, { count: budget.count + 1, lastAt: now, lastNote: note });
    await this.deliver(parentId, buildFleetMessage('progress', [entry]));
    return { deliveredTo: parentId };
  }

  /** Drop a session's budget — used by tests; live budgets are per-session and
   *  die with the process, like threshold watches. */
  reset(): void {
    this.budgets.clear();
  }
}
