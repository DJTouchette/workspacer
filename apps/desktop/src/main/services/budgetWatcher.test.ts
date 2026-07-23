/**
 * budgetWatcher — one alert per crossing, delivered somewhere no matter what.
 *
 * The in-app notification center records every crossing (postInApp) even when
 * OS notifications are disabled, so the latch may set on first delivery: the
 * alert is never silently swallowed (the trap that used to require re-firing
 * the OS notification after a re-enable). The OS notification remains gated by
 * `notifications.enabled` and is click-wired to focus the offending agent.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const h = vi.hoisted(() => {
  const notificationShow = vi.fn();
  const notificationOn = vi.fn();
  // vitest v4 requires a real constructor for `new Notification(...)`, so use a
  // class (an arrow mockImplementation throws "is not a constructor").
  class NotificationMock {
    show = notificationShow;
    on = notificationOn;
    static isSupported = () => true;
  }
  const postInApp = vi.fn();
  const focusAgent = vi.fn();
  // Mutable config the service reads via configService.getConfig().
  const config: {
    claude: { budgets: Record<string, number> };
    notifications: { enabled?: boolean; sound?: boolean };
  } = { claude: { budgets: {} }, notifications: {} };
  return { notificationShow, notificationOn, NotificationMock, postInApp, focusAgent, config };
});

vi.mock('electron', () => ({ Notification: h.NotificationMock }));
vi.mock('./configService', () => ({
  configService: { getConfig: () => h.config },
}));
vi.mock('./agentNotifier', () => ({
  agentNotifier: { postInApp: h.postInApp, focusAgent: h.focusAgent },
}));

import { checkBudget, forgetBudget } from './budgetWatcher';
import type { ClaudeSessionState } from './claudeSessionStore';

const session = {
  sessionId: 's1',
  statusLine: { costUSD: 5 },
  label: 'A',
} as unknown as ClaudeSessionState;

describe('checkBudget', () => {
  beforeEach(() => {
    h.notificationShow.mockClear();
    h.notificationOn.mockClear();
    h.postInApp.mockClear();
    h.focusAgent.mockClear();
    h.config.claude.budgets = { s1: 1 };
    forgetBudget('s1'); // clear any latch from prior tests
  });

  it('records the crossing in the notification center even with OS notifications off', () => {
    h.config.notifications = { enabled: false };
    checkBudget(session);
    expect(h.notificationShow).not.toHaveBeenCalled();
    expect(h.postInApp).toHaveBeenCalledTimes(1);
    expect(h.postInApp.mock.calls[0][0]).toMatchObject({
      source: 'budget',
      sessionId: 's1',
      level: 'warn',
    });

    // The center delivery latched the crossing: re-enabling OS notifications
    // must not replay an alert the user already received.
    h.config.notifications = { enabled: true };
    checkBudget(session);
    expect(h.notificationShow).not.toHaveBeenCalled();
    expect(h.postInApp).toHaveBeenCalledTimes(1);
  });

  it('fires the OS notification once per crossing and wires click → focusAgent', () => {
    h.config.notifications = { enabled: true };
    checkBudget(session);
    checkBudget(session);
    expect(h.notificationShow).toHaveBeenCalledTimes(1);
    expect(h.postInApp).toHaveBeenCalledTimes(1);

    // The click handler focuses the over-budget agent.
    expect(h.notificationOn).toHaveBeenCalledWith('click', expect.any(Function));
    (h.notificationOn.mock.calls[0][1] as () => void)();
    expect(h.focusAgent).toHaveBeenCalledWith('s1');
  });

  it('re-arms after the budget is raised back above spend', () => {
    h.config.notifications = { enabled: true };
    checkBudget(session);
    expect(h.postInApp).toHaveBeenCalledTimes(1);

    // User raises the budget: spend is back under, latch clears...
    h.config.claude.budgets = { s1: 10 };
    checkBudget(session);
    expect(h.postInApp).toHaveBeenCalledTimes(1);

    // ...so a later crossing alerts again.
    h.config.claude.budgets = { s1: 2 };
    checkBudget(session);
    expect(h.postInApp).toHaveBeenCalledTimes(2);
    expect(h.notificationShow).toHaveBeenCalledTimes(2);
  });
});
