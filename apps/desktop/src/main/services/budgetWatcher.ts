/**
 * Per-session cost budgets. The user sets a dollar threshold on a session in
 * the inspector (persisted in `config.claude.budgets[sessionId]`); when that
 * session's spend crosses the threshold we fire a single OS notification.
 *
 * Deliberately low-footprint: no in-app UI beyond the inspector input, one
 * notification per crossing, and the alert re-arms if the budget is later
 * raised back above the current spend.
 */
import { Notification } from 'electron';
import { configService } from './configService';
import type { ClaudeSessionState } from './claudeSessionStore';

/** Sessions we've already alerted for, so a growing cost doesn't re-notify. */
const alerted = new Set<string>();

/** The session's best-known cumulative cost (statusLine is Claude's own
 *  authoritative number in stream mode; else the transcript-derived usage). */
function sessionCost(session: ClaudeSessionState): number {
  return session.statusLine?.costUSD ?? session.usage?.costUSD ?? 0;
}

/**
 * Check a session against its budget after a cost update. Fires at most one
 * notification per crossing; clears the latch when spend drops back under the
 * budget (e.g. the user raised it), so a later crossing alerts again.
 */
export function checkBudget(session: ClaudeSessionState): void {
  const budgets = configService.getConfig().claude?.budgets;
  const budget = budgets?.[session.sessionId];
  if (!budget || budget <= 0) return;

  const cost = sessionCost(session);
  if (cost < budget) {
    alerted.delete(session.sessionId);
    return;
  }
  if (alerted.has(session.sessionId)) return;
  alerted.add(session.sessionId);

  const cfg =
    (configService.getConfig() as { notifications?: { enabled?: boolean; sound?: boolean } })
      .notifications ?? {};
  if (cfg.enabled === false || !Notification.isSupported()) return;

  const label = session.label || 'Agent';
  new Notification({
    title: 'Session over budget',
    body: `${label} has spent $${cost.toFixed(2)} (budget $${budget.toFixed(2)}).`,
    silent: cfg.sound !== true,
  }).show();
}

/** Drop a session's alert latch (call on session end so a resumed id re-arms). */
export function forgetBudget(sessionId: string): void {
  alerted.delete(sessionId);
}
