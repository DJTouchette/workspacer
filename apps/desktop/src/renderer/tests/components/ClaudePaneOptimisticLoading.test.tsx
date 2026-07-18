import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * Regression: the optimistic "sending" / working indicator must survive a send
 * from an IDLE prompt. handleSend sets optimisticLoading=true to bridge the gap
 * between the send and the daemon flipping the session to thinking/streaming
 * (~FLUSH_DELAY_MS=300ms + round-trips later). The optimistic-dequeue effect
 * used to clear optimisticLoading whenever ambientState is idle — but a chat
 * message is normally sent while ambientState is still 'idle', so the clear
 * fired the same tick optimisticLoading was set, suppressing the spinner for
 * the whole settle+submit window. Mock scaffolding mirrors
 * ClaudePaneQuestionsOptimistic.test.tsx.
 */

vi.mock('@xterm/xterm', () => {
  class MockTerminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    open = vi.fn();
    write = vi.fn();
    dispose = vi.fn();
    focus = vi.fn();
    blur = vi.fn();
    refresh = vi.fn();
    clearSelection = vi.fn();
    getSelection = vi.fn().mockReturnValue('');
    onData = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onBinary = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onResize = vi.fn().mockReturnValue({ dispose: vi.fn() });
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
    parser = { registerCsiHandler: vi.fn() };
  }
  return { Terminal: MockTerminal };
});
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn();
    activate = vi.fn();
    dispose = vi.fn();
  },
}));
vi.mock('@xterm/addon-web-fonts', () => ({
  WebFontsAddon: class {
    activate = vi.fn();
    dispose = vi.fn();
    loadFonts = vi.fn().mockResolvedValue(undefined);
  },
}));

const mockWrite = vi.fn();
vi.mock('../../src/hooks/useClaudeSpawn', () => ({
  useClaudeSpawn: vi.fn().mockReturnValue({
    sessionId: 'sess-1',
    isReady: true,
    spawnError: null,
    write: mockWrite,
    resize: vi.fn(),
    attachToTerminal: vi.fn(),
    startSession: vi.fn(),
    retry: vi.fn(),
    restartSession: vi.fn(),
  }),
}));

let mockSession: ClaudeSessionSnapshot | null = null;
vi.mock('../../src/hooks/useClaudeSession', () => ({
  useClaudeSession: vi.fn().mockImplementation(() => ({ session: mockSession, refresh: vi.fn() })),
}));

vi.mock('../../src/hooks/useTheme', () => ({
  useTheme: vi.fn().mockReturnValue({ theme: {}, terminalTheme: {} }),
}));

vi.mock('../../src/hooks/useConfig', () => ({
  useConfig: vi.fn().mockReturnValue({
    config: {
      claude: { defaultView: 'gui', workLog: 'cards' },
      terminal: {
        fontSize: 14,
        fontFamily: 'monospace',
        cursorBlink: true,
        scrollback: 1000,
        cursorStyle: 'block',
        shell: '',
        shells: [],
      },
      ui: { navBarHeight: 28, paneHeaderHeight: 22, guiFontScale: 1.15, showComposerSend: true },
      panes: { peek: 80, gap: 16, insertPosition: 'after' },
      keybindings: { prefix: 'ctrl+space', shortcuts: {} },
      apps: [],
    },
    reload: vi.fn(),
  }),
}));

const { default: ClaudePane } = await import('../../src/panes/ClaudePane');

function makeSnapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 'sess-1',
    cwd: '/repo',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    subagents: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

const pane = () => <ClaudePane paneId="p1" title="Claude" isActive cwd="/repo" />;

beforeEach(() => {
  mockSession = makeSnapshot({ ambientState: 'idle' });
  mockWrite.mockClear();
  (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
});

describe('ClaudePane optimistic sending indicator', () => {
  it('shows the working/cancel indicator right after sending from an idle session', async () => {
    render(pane());

    const composer = screen.getByRole('textbox') as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: 'do the thing' } });
    fireEvent.keyDown(composer, { key: 'Enter' });

    // The optimistic bubble is shown immediately.
    expect(await screen.findByText('do the thing')).toBeInTheDocument();

    // ...and so is the streaming/cancel indicator — optimisticLoading bridges
    // the gap until the daemon flips ambientState to thinking/streaming. It must
    // NOT be cleared by the still-idle snapshot the same tick it was set.
    expect(await screen.findByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });
});
