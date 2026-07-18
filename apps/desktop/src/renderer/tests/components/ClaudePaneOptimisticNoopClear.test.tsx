import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * Regression: the optimistic "sending"/cancel indicator must eventually CLEAR
 * even when a send never drives the session into thinking/streaming — e.g. an
 * instant no-op slash command like `/model sonnet` that the daemon processes
 * without starting a turn (ambientState stays 'idle'), or a transient thinking
 * snapshot coalesced away by store batching.
 *
 * The daemon still lands the real user turn in session.conversation. That turn
 * arriving is proof the send was engaged, so the optimistic spinner must be
 * retired once the session is idle again. Before the fix, optimisticLoading was
 * only cleared after observing thinking/streaming (sawServerActivitySinceSendRef),
 * so a no-op send left the spinner + "Cancel" affordance stuck on forever.
 *
 * Mock scaffolding mirrors ClaudePaneOptimisticLoading.test.tsx.
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

describe('ClaudePane optimistic spinner clears on a no-op (never-thinking) send', () => {
  it('retires the cancel/streaming indicator once the send lands as a turn while idle', async () => {
    const { rerender } = render(pane());

    const composer = screen.getByRole('textbox') as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: '/model sonnet' } });
    fireEvent.keyDown(composer, { key: 'Enter' });

    // Optimistic bridge is up: spinner + Cancel affordance shown.
    expect(await screen.findByRole('button', { name: 'Cancel' })).toBeInTheDocument();

    // Daemon processes the no-op command: the real user turn lands in the
    // authoritative conversation but ambientState never leaves 'idle' (no turn
    // started). This dequeues the optimistic bubble.
    mockSession = makeSnapshot({
      ambientState: 'idle',
      conversation: [{ role: 'user', content: '/model sonnet', timestamp: Date.now() }] as any,
      lastActivity: Date.now(),
    });
    rerender(pane());

    // The optimistic spinner must clear — the send was acknowledged.
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
    });
  });
});
