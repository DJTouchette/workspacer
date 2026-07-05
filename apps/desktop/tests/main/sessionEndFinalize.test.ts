/**
 * Regression test: a session's analytics row must be finalized to 'ended'
 * even when a Stop event already fired its delayed 'active' write first.
 *
 * Lifecycle in the wild: Stop (end of a turn) schedules a +1500ms 'active'
 * snapshot; SessionEnd arrives later. If SessionEnd refuses to write once the
 * Stop timer has run, the row is stuck on status='active' with no ended_at.
 */
import { describe, it, expect, vi } from 'vitest';

const { recordMock } = vi.hoisted(() => ({ recordMock: vi.fn() }));

vi.mock('electron', () => ({
  BrowserWindow: vi.fn(),
  Notification: { isSupported: vi.fn(() => false) },
}));

vi.mock('../../src/main/services/configService', () => ({
  configService: {
    getConfig: vi.fn(() => ({ notifications: { enabled: false }, claude: { seenModels: [] } })),
    saveConfig: vi.fn(),
  },
}));

vi.mock('../../src/main/services/workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), poke: vi.fn(), detach: vi.fn() },
}));

vi.mock('../../src/main/services/hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));

vi.mock('../../src/main/services/sessionHistory', () => ({
  sessionHistory: { record: recordMock },
}));

vi.mock('../../src/main/services/hubClient', () => ({
  publishToHub: vi.fn(),
  isHubConnected: vi.fn(() => false),
  startHubClient: vi.fn(),
  stopHubClient: vi.fn(),
  setHubMainWindow: vi.fn(),
  registerCapability: vi.fn(),
}));

vi.mock('../../src/main/services/hubDaemon', () => ({
  HUB_BUS_URL: 'ws://localhost:3457/bus',
  getHubToken: vi.fn(() => null),
}));

const { claudeSessionStore: store } = await import('../../src/main/services/claudeSessionStore');

function mkEvent(hookName: string, sessionId: string, extra: Record<string, unknown> = {}) {
  return { hook_event_name: hookName, session_id: sessionId, cwd: '/test/project', ...extra };
}

describe('SessionEnd analytics finalization', () => {
  it("writes status 'ended' even after the Stop timer's 'active' snapshot", () => {
    vi.useFakeTimers();
    try {
      const id = 'finalize-after-stop';
      recordMock.mockClear();

      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('Stop', id));
      // The Stop event schedules its delayed 'active' write at +1500ms.
      vi.advanceTimersByTime(1500);
      // SessionEnd arrives afterwards — it must finalize the row to 'ended'.
      store.handleHookEvent(mkEvent('SessionEnd', id));

      const statuses = recordMock.mock.calls.map((c) => (c[0] as any).status);
      expect(statuses).toContain('ended');

      // The finalizing write must carry a real ended_at timestamp.
      const endedCall = recordMock.mock.calls
        .map((c) => c[0] as any)
        .find((r) => r.status === 'ended');
      expect(endedCall.endedAt).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("writes 'ended' when SessionEnd arrives before the Stop timer fires", () => {
    vi.useFakeTimers();
    try {
      const id = 'finalize-before-stop-timer';
      recordMock.mockClear();

      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('Stop', id));
      // SessionEnd lands first; the pending Stop timer must not revert to 'active'.
      store.handleHookEvent(mkEvent('SessionEnd', id));
      vi.advanceTimersByTime(2000);

      const statuses = recordMock.mock.calls.map((c) => (c[0] as any).status);
      expect(statuses).toContain('ended');
      expect(statuses).not.toContain('active');
    } finally {
      vi.useRealTimers();
    }
  });
});
