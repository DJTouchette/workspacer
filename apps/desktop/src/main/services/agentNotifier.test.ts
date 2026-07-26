/**
 * agentNotifier — context-rich OS notifications + in-app center mirroring.
 *
 * Pins the enrichment contract: an approval notification carries the tool name
 * and the decidable gist of its input, a question notification carries the
 * question text, a done notification carries the session cost. Every fired
 * transition also mirrors into the in-app center (NOTIFY_IN_APP push) — even
 * when OS notifications are disabled — and is toast-silent when the user is
 * already watching that agent.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const h = vi.hoisted(() => {
  const created: Array<{ title: string; body: string }> = [];
  const notificationOn = vi.fn();
  const notificationShow = vi.fn();
  class NotificationMock {
    title: string;
    body: string;
    on = notificationOn;
    show = notificationShow;
    constructor(opts: { title: string; body: string }) {
      this.title = opts.title;
      this.body = opts.body;
      created.push(opts);
    }
    static isSupported = () => true;
  }
  const config: { notifications: Record<string, boolean> } = { notifications: {} };
  return { created, notificationOn, notificationShow, NotificationMock, config };
});

vi.mock('electron', () => ({ Notification: h.NotificationMock, BrowserWindow: class {} }));
vi.mock('./configService', () => ({ configService: { getConfig: () => h.config } }));
vi.mock('../lib/appIcon', () => ({ appIconPath: () => null }));

import { agentNotifier } from './agentNotifier';
import type { ClaudeSessionState } from './claudeSessionStore';

/** Minimal BrowserWindow stub: enough for setMainWindow + postInApp + focus. */
function makeWindow(focused: boolean) {
  const sent: Array<[string, unknown]> = [];
  let loadHandler: (() => void) | null = null;
  const win = {
    removeAllListeners: vi.fn(),
    on: vi.fn(),
    isDestroyed: () => false,
    isFocused: () => focused,
    isMinimized: () => false,
    restore: vi.fn(),
    show: vi.fn(),
    focus: vi.fn(),
    flashFrame: vi.fn(),
    webContents: {
      on: (event: string, cb: () => void) => {
        if (event === 'did-finish-load') loadHandler = cb;
      },
      send: (channel: string, payload: unknown) => sent.push([channel, payload]),
    },
  };
  return {
    win,
    sent,
    finishLoad: () => loadHandler?.(),
  };
}

function session(over: Partial<ClaudeSessionState>): ClaudeSessionState {
  return {
    sessionId: 's1',
    cwd: '/home/me/projects/rocket',
    ambientState: 'idle',
    pendingApproval: null,
    pendingQuestions: null,
    ...over,
  } as ClaudeSessionState;
}

function inAppPayloads(sent: Array<[string, unknown]>) {
  return sent.filter(([ch]) => ch === 'notify:in-app').map(([, p]) => p as Record<string, unknown>);
}

describe('notifyOnTransition', () => {
  beforeEach(() => {
    h.created.length = 0;
    h.notificationOn.mockClear();
    h.notificationShow.mockClear();
    h.config.notifications = {};
    agentNotifier.setActiveSession(null);
  });

  it('approval notifications carry the tool name and input gist', () => {
    const { win, sent, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();

    agentNotifier.notifyOnTransition(
      session({
        ambientState: 'waiting_approval',
        pendingApproval: {
          toolName: 'Bash',
          toolInput: { command: 'rm -rf build' },
          timestamp: 1,
        },
      }),
      'streaming',
    );

    expect(h.created).toHaveLength(1);
    expect(h.created[0].title).toBe('rocket needs approval');
    expect(h.created[0].body).toBe('Allow Bash — $ rm -rf build?');

    const inApp = inAppPayloads(sent);
    expect(inApp).toHaveLength(1);
    expect(inApp[0]).toMatchObject({
      level: 'warn',
      source: 'agent',
      sessionId: 's1',
      key: 'agent:s1:needs-you',
      silent: false,
    });
  });

  it('question notifications carry the question text', () => {
    const { win, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();

    agentNotifier.notifyOnTransition(
      session({
        ambientState: 'waiting_input',
        pendingQuestions: [
          { question: 'Deploy to prod?', options: [] },
          { question: 'Second question', options: [] },
        ],
      }),
      'thinking',
    );

    expect(h.created[0].title).toBe('rocket is waiting for input');
    expect(h.created[0].body).toBe('Deploy to prod? (+1 more)');
  });

  it('done notifications carry the session cost and prefer the explicit label', () => {
    const { win, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();

    agentNotifier.notifyOnTransition(
      session({ ambientState: 'idle', label: 'Refactor', statusLine: { costUSD: 1.5 } as never }),
      'streaming',
    );

    expect(h.created[0].title).toBe('Refactor finished');
    expect(h.created[0].body).toBe('Ready for your next step. Spent $1.50 this session.');
  });

  it('mirrors to the in-app center even when OS notifications are disabled', () => {
    const { win, sent, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();
    h.config.notifications = { enabled: false };

    agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_input' }), 'streaming');

    expect(h.created).toHaveLength(0);
    expect(inAppPayloads(sent)).toHaveLength(1);
  });

  it('is toast-silent (and OS-suppressed) for the agent being watched', () => {
    const { win, sent, finishLoad } = makeWindow(true);
    agentNotifier.setMainWindow(win as never);
    finishLoad();
    agentNotifier.setActiveSession('s1');

    agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_approval' }), 'streaming');

    expect(h.created).toHaveLength(0); // onlyWhenUnwatched default suppresses OS
    const inApp = inAppPayloads(sent);
    expect(inApp).toHaveLength(1);
    expect(inApp[0].silent).toBe(true);
  });

  it('escalation raises a clickable OS notification that routes back to activate', () => {
    const { win, sent, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();

    const n = {
      id: 'x1',
      level: 'info' as const,
      title: 'CI failed',
      body: 'main is red',
      source: 'plugin:ci',
      sessionId: 's9',
      createdAt: 1,
    };
    agentNotifier.escalateFromRenderer(n);

    expect(h.created).toHaveLength(1);
    expect(h.created[0]).toMatchObject({ title: 'CI failed', body: 'main is red' });

    // Click hands the notification back to the renderer's activate path.
    expect(h.notificationOn).toHaveBeenCalledWith('click', expect.any(Function));
    (h.notificationOn.mock.calls[0][1] as () => void)();
    expect(sent.filter(([ch]) => ch === 'notify:activate').map(([, p]) => p)).toEqual([n]);
  });

  it('escalation respects the OS master switch and the silent flag', () => {
    const { win, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);
    finishLoad();

    const n = {
      id: 'x2',
      level: 'info' as const,
      title: 't',
      source: 'plugin:ci',
      createdAt: 1,
    };
    h.config.notifications = { enabled: false };
    agentNotifier.escalateFromRenderer(n);
    expect(h.created).toHaveLength(0);

    h.config.notifications = {};
    agentNotifier.escalateFromRenderer({ ...n, silent: true });
    expect(h.created).toHaveLength(0);

    // Garbage from a compromised renderer is dropped, not thrown.
    agentNotifier.escalateFromRenderer(null as never);
    agentNotifier.escalateFromRenderer({} as never);
    expect(h.created).toHaveLength(0);
  });

  /**
   * One event, one surface. `onlyWhenUnwatched` only ever covered the agent you
   * are looking *at*; with the window focused on a different agent it left the
   * OS notification firing right beside the in-app toast, so the same event
   * arrived twice at once. The OS notification now stands down whenever a toast
   * is actually coming — and only then.
   */
  describe('an OS notification never doubles up with an in-app toast', () => {
    it('stands down when the window is focused on a DIFFERENT agent', () => {
      const { win, sent, finishLoad } = makeWindow(true);
      agentNotifier.setMainWindow(win as never);
      finishLoad();
      agentNotifier.setActiveSession('other-agent');

      agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_approval' }), 'streaming');

      expect(h.created).toHaveLength(0);
      // The toast is the surface here, so it must not be silenced too.
      const inApp = inAppPayloads(sent);
      expect(inApp).toHaveLength(1);
      expect(inApp[0].silent).toBe(false);
    });

    it('still fires when the window is unfocused', () => {
      const { win, sent, finishLoad } = makeWindow(false);
      agentNotifier.setMainWindow(win as never);
      finishLoad();
      agentNotifier.setActiveSession('other-agent');

      agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_approval' }), 'streaming');

      expect(h.created).toHaveLength(1);
      expect(inAppPayloads(sent)).toHaveLength(1);
    });

    it('still fires when focused but in-app toasts are switched off', () => {
      const { win, finishLoad } = makeWindow(true);
      agentNotifier.setMainWindow(win as never);
      finishLoad();
      agentNotifier.setActiveSession('other-agent');
      h.config.notifications = { inAppToasts: false };

      agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_approval' }), 'streaming');

      // Nothing else would have surfaced it, so standing down would lose it.
      expect(h.created).toHaveLength(1);
    });

    it('still fires for the watched agent when the user opted into that', () => {
      const { win, sent, finishLoad } = makeWindow(true);
      agentNotifier.setMainWindow(win as never);
      finishLoad();
      agentNotifier.setActiveSession('s1');
      h.config.notifications = { onlyWhenUnwatched: false };

      agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_approval' }), 'streaming');

      // The in-app mirror is toast-silent while watching, so the OS one is the
      // only surface left and the focus guard must not eat it.
      expect(inAppPayloads(sent)[0].silent).toBe(true);
      expect(h.created).toHaveLength(1);
    });
  });

  it('buffers in-app notifications raised before the renderer loads', () => {
    const { win, sent, finishLoad } = makeWindow(false);
    agentNotifier.setMainWindow(win as never);

    agentNotifier.notifyOnTransition(session({ ambientState: 'waiting_input' }), 'streaming');
    expect(inAppPayloads(sent)).toHaveLength(0);

    finishLoad();
    expect(inAppPayloads(sent)).toHaveLength(1);
  });
});
