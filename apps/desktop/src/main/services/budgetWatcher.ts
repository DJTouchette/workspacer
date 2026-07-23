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
import { agentNotifier } from './agentNotifier';
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

  const cfg =
    (configService.getConfig() as { notifications?: { enabled?: boolean; sound?: boolean } })
      .notifications ?? {};

  // Latch once the alert is actually delivered somewhere (the in-app center
  // always records it). Latching before any delivery would permanently
  // suppress the alert: cumulative cost never drops back under budget, so the
  // latch (cleared only at `cost < budget`) would never re-arm.
  alerted.add(session.sessionId);

  const label = session.label || 'Agent';
  const title = 'Session over budget';
  const body = `${label} has spent $${cost.toFixed(2)} (budget $${budget.toFixed(2)}).`;

  agentNotifier.postInApp({
    level: 'warn',
    title,
    body,
    source: 'budget',
    sessionId: session.sessionId,
    key: `budget:${session.sessionId}`,
  });

  if (cfg.enabled === false || !Notification.isSupported()) return;
  const notification = new Notification({ title, body, silent: cfg.sound !== true });
  notification.on('click', () => agentNotifier.focusAgent(session.sessionId));
  notification.on('failed', (_e, err) =>
    console.warn(`[budget] OS notification failed (in-app center still has it): ${err}`),
  );
  notification.show();
}

/** Drop a session's alert latch (call on session end so a resumed id re-arms). */
export function forgetBudget(sessionId: string): void {
  alerted.delete(sessionId);
}
