/**
 * Standalone ClaudePane harness for the TAIL PIN — "your newest message rides
 * the top of the viewport while the reply streams in below it".
 *
 * That behaviour is layout, and layout is exactly what the existing unit tests
 * cannot see: jsdom returns 0 for getBoundingClientRect and scrollHeight, so
 * ClaudePaneTailPin.test.tsx can assert the tail spacer was *computed* but
 * never that the message actually lands at the top. This runs the real pane in
 * a real engine so a Playwright test can measure where the message ended up.
 *
 * Served by the normal Vite dev server: /chat-pin-harness.html
 * Not part of the app build — nothing imports this except the harness page.
 */
/* eslint-disable @typescript-eslint/no-explicit-any */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';

const SESSION_ID = 'pin-harness-session';

/** A long transcript, so the pane is scrolled well past its first turn. */
const conversation = Array.from({ length: 40 }, (_, i) => ({
  role: i % 2 === 0 ? 'user' : 'assistant',
  content:
    i % 2 === 0
      ? `Turn ${i}: a user message that is long enough to occupy real vertical space in the transcript.`
      : `Turn ${i}: an assistant reply. ${'It carries several lines of prose so the conversation is genuinely taller than the viewport. '.repeat(3)}`,
  timestamp: 1_700_000_000_000 + i * 60_000,
  toolCalls: [],
}));

const snapshot: any = {
  sessionId: SESSION_ID,
  cwd: '/work/harness',
  status: 'running',
  ambientState: 'idle',
  provider: 'claude',
  model: 'opus',
  conversation,
  conversationOffset: 0,
  conversationUserOffset: 0,
  activeToolCalls: [],
  completedToolCalls: [],
  fileChanges: [],
  pendingApproval: null,
  pendingQuestions: null,
};

// Installed BEFORE importing anything that touches it at module scope. The pin
// arms inside the send handler and the optimistic turn renders locally, so the
// backend only has to accept the send without throwing.
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    // The pane reads nested config (theme, terminal link handling) and throws on
    // a bare `{}`. Imported lazily so the stub is still installed before any
    // module that touches electronAPI at import time.
    getConfig: async () => (await import('../hooks/useConfig')).DEFAULT_CONFIG,
    reloadConfig: async () => (await import('../hooks/useConfig')).DEFAULT_CONFIG,
    saveConfig: async () => (await import('../hooks/useConfig')).DEFAULT_CONFIG,
    getHubStatus: async () => ({ connected: true }),
    // useClaudeSpawn adopts whatever this resolves to as the pane's sessionId;
    // undefined leaves the pane stuck on "Connecting to Claude…".
    attachClaude: async (_paneId: string, sid: string) => sid ?? SESSION_ID,
    detachClaude: async () => {},
    getClaudeSession: async () => snapshot,
    getAllClaudeSessions: async () => [snapshot],
    listLiveClaudeSessionIds: async () => [SESSION_ID],
    // The send itself: report success and let the optimistic turn stand in for
    // the echo. What is under test is where the view lands, not the transport.
    claudeSendMessage: async () => ({ ok: true }),
    sendClaudeMessage: async () => ({ ok: true }),
    claudeWrite: () => {},
  },
  {
    get(target: any, prop: string) {
      if (prop in target) return target[prop];
      if (typeof prop === 'string' && prop.startsWith('on')) return () => () => {};
      return () => Promise.resolve(undefined);
    },
  },
);

// Deferred so the stub is installed first.
const { default: ClaudePane } = await import('../panes/ClaudePane');
const { AttentionProvider } = await import('../contexts/AttentionContext');
const { NotificationsProvider } = await import('../contexts/NotificationsContext');
const { ConfigProvider } = await import('../contexts/ConfigContext');
const { resolveTheme, applyTheme } = await import('../themes');

applyTheme(resolveTheme('everforest'));
document.documentElement.style.setProperty('--wks-font-mono', 'ui-monospace, monospace');

const noop = () => {};

function Harness(): React.ReactElement {
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={[]}
          activeAgentId=""
          snapshotBySession={{ [SESSION_ID]: snapshot }}
          inboxOpen={false}
          openInbox={noop}
          closeInbox={noop}
          viewLevel="piloting"
          setViewLevel={noop}
          onOpenAgent={noop}
          attention={{ items: [], counts: {} } as any}
        >
          {/* The pane sizes itself with flex:1, so every ancestor needs a
              definite height and min-height:0 or the scroll container collapses
              to zero and there is nothing to measure. */}
          <div
            className="app-root"
            style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
          >
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <ClaudePane
                paneId="pin-harness-pane"
                title="Harness"
                isActive
                cwd="/work/harness"
                attachSessionId={SESSION_ID}
                transport="stream"
              />
            </div>
          </div>
        </AttentionProvider>
      </NotificationsProvider>
    </ConfigProvider>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />);
