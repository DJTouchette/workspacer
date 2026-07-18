/**
 * budgetWatcher — the alert latch must not swallow the notification when it is
 * set BEFORE the notifications-enabled check. If a budget crossing happens while
 * notifications are disabled, re-enabling them must still deliver the alert
 * (cumulative cost never drops back under budget to clear the latch).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const h = vi.hoisted(() => {
  const notificationShow = vi.fn();
  // vitest v4 requires a real constructor for `new Notification(...)`, so use a
  // class (an arrow mockImplementation throws "is not a constructor").
  class NotificationMock {
    show = notificationShow;
    static isSupported = () => true;
  }
  // Mutable config the service reads via configService.getConfig().
  const config: {
    claude: { budgets: Record<string, number> };
    notifications: { enabled?: boolean; sound?: boolean };
  } = { claude: { budgets: {} }, notifications: {} };
  return { notificationShow, NotificationMock, config };
});

vi.mock('electron', () => ({ Notification: h.NotificationMock }));
vi.mock('./configService', () => ({
  configService: { getConfig: () => h.config },
}));

import { checkBudget, forgetBudget } from './budgetWatcher';
import type { ClaudeSessionState } from './claudeSessionStore';

const session = {
  sessionId: 's1',
  statusLine: { costUSD: 5 },
  label: 'A',
} as unknown as ClaudeSessionState;

describe('checkBudget latch vs notifications toggle', () => {
  beforeEach(() => {
    h.notificationShow.mockClear();
    h.config.claude.budgets = { s1: 1 };
    forgetBudget('s1'); // clear any latch from prior tests
  });

  it('re-fires the over-budget alert after notifications are re-enabled', () => {
    // Notifications OFF: crossing the budget must not notify...
    h.config.notifications = { enabled: false };
    checkBudget(session);
    expect(h.notificationShow).not.toHaveBeenCalled();

    // ...and must NOT have latched the session as "already alerted".
    // Cost stays >= budget (cumulative spend never decreases).
    h.config.notifications = { enabled: true };
    checkBudget(session);
    expect(h.notificationShow).toHaveBeenCalledTimes(1);
  });

  it('still fires only once per crossing when notifications are on throughout', () => {
    h.config.notifications = { enabled: true };
    checkBudget(session);
    checkBudget(session);
    expect(h.notificationShow).toHaveBeenCalledTimes(1);
  });
});
