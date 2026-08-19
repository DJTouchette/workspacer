/**
 * Standalone Guide harness — the Workspacer Guide onboarding surfaces with a
 * fake electronAPI, no Electron and no daemons. Fully interactive: chips and
 * the composer "spawn" a fake guide (800ms), sends to a running guide resolve
 * immediately, and every call is shown in the action log strip at the bottom.
 *
 * Views (query param `view`):
 *   - (default)      the Guide pane, fresh state — scripted bubbles + chips
 *   - ?view=running  the Guide pane with a live guide agent to reuse
 *   - ?view=welcome  the first-run welcome card with the "Or just ask" section
 *
 * Open http://localhost:5173/guide-harness.html with the dev server running.
 */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import type { AgentWorkspace } from '../types/pane';

const log: string[] = [];
let pushLog: (line: string) => void = (line) => log.push(line);

(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    claudeMessage: async (sessionId: string, text: string) => {
      pushLog(`claudeMessage(${sessionId}): ${text.slice(0, 80)}…`);
      return { ok: true };
    },
    getSupervisorHome: async () => '/home/you/.workspacer',
  },
  {
    get(target: any, prop: string) {
      if (prop in target) return target[prop];
      if (typeof prop === 'string' && prop.startsWith('on')) {
        return () => () => {};
      }
      return () => Promise.resolve(undefined);
    },
  },
);

// Deferred imports so the stub is installed first.
const { default: GuidePane } = await import('../panes/GuidePane');
const { default: Onboarding } = await import('../components/Onboarding');
const { GUIDE_AGENT_NAME } = await import('../lib/guide');
const { resolveTheme, applyTheme } = await import('../themes');

const params = new URLSearchParams(location.search);
applyTheme(resolveTheme(params.get('theme') ?? 'dark'));
const view = params.get('view') ?? 'pane';

const runningGuide: AgentWorkspace = {
  id: 'agent-guide',
  name: GUIDE_AGENT_NAME,
  cwd: '/home/you/.workspacer',
  sessionId: 'sess-guide-1',
  tabs: [],
  activeTabId: '',
};

const Frame: React.FC = () => {
  const [lines, setLines] = useState<string[]>(log);
  pushLog = (line) => setLines((prev) => [...prev, line]);

  const spawnGuide = async (question: string): Promise<string> => {
    pushLog(`spawnGuide: ${question.slice(0, 80)}…`);
    await new Promise((r) => setTimeout(r, 800));
    return 'agent-guide-fresh';
  };

  return (
    <div
      style={{
        minHeight: '100vh',
        background: 'var(--wks-bg-base)',
        display: 'flex',
        flexDirection: 'column',
        fontFamily: 'var(--wks-font-sans)',
      }}
    >
      <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
        {view === 'welcome' ? (
          <Onboarding
            overlay
            firstRun
            onSpawn={() => pushLog('onSpawn (spawn dialog)')}
            onDismiss={() => pushLog('onDismiss')}
            onOpenKeybindings={() => pushLog('onOpenKeybindings')}
            shortcuts={{}}
            presetId="vscode"
            onChoosePreset={(id) => pushLog(`onChoosePreset: ${id}`)}
            onAskGuide={(q) => pushLog(`onAskGuide: ${q.slice(0, 80)}…`)}
          />
        ) : (
          <div style={{ height: '80vh' }}>
            <GuidePane
              agents={view === 'running' ? [runningGuide] : []}
              spawnGuide={spawnGuide}
              onJumpToAgent={(id) => pushLog(`onJumpToAgent: ${id}`)}
            />
          </div>
        )}
      </div>
      {/* Action log — what the surface would have done to the real app. */}
      <div
        data-testid="action-log"
        style={{
          borderTop: '1px solid var(--wks-border-subtle)',
          padding: '8px 12px',
          fontFamily: 'var(--wks-font-mono)',
          fontSize: '0.66rem',
          color: 'var(--wks-text-faint)',
          maxHeight: 120,
          overflowY: 'auto',
          flexShrink: 0,
        }}
      >
        {lines.length === 0 ? 'action log — nothing called yet' : null}
        {lines.map((l, i) => (
          <div key={i}>{l}</div>
        ))}
      </div>
    </div>
  );
};

createRoot(document.getElementById('root')!).render(<Frame />);
