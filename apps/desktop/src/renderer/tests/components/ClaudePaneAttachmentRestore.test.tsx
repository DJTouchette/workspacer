import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, act, waitFor } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * Regression: attached files (and their prompt prefix) must survive a REJECTED
 * send. handleSend clears both inputValue and attachedFiles up front, then
 * calls claudemon's /message. When the session has ended the daemon returns
 * { ok: false } — handleSend retracts the optimistic bubble and restores the
 * text, but historically it never restored attachedFiles, silently discarding
 * the user's attachment. A retry then re-sent only the bare text without the
 * file. Mock scaffolding mirrors ClaudePaneOptimisticLoading.test.tsx.
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
  // The session has ended → daemon rejects the send.
  (window.electronAPI.claudeMessage as any) = vi
    .fn()
    .mockResolvedValue({ ok: false, mode: 'ended' });
});

describe('ClaudePane rejected-send attachment restore', () => {
  it('keeps the attached file after the daemon rejects the send', async () => {
    render(pane());

    // Attach a file via the document-level drop handler.
    const dropEvent = new Event('drop', { bubbles: true, cancelable: true }) as any;
    dropEvent.dataTransfer = {
      files: { length: 1, 0: { path: '/tmp/foo.txt', name: 'foo.txt' } },
    };
    act(() => {
      document.dispatchEvent(dropEvent);
    });

    // The file chip is shown.
    expect(await screen.findByText('foo.txt')).toBeInTheDocument();

    // Type and send.
    const composer = screen.getByRole('textbox') as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: 'review this' } });
    fireEvent.keyDown(composer, { key: 'Enter' });

    // The full message (prefix + text) was submitted to the daemon...
    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith(
        'sess-1',
        '[File: /tmp/foo.txt] review this',
      ),
    );

    // ...and, because the send was rejected, the attachment must NOT be lost:
    // the chip is still present so a retry re-sends the file. (Fails today —
    // handleSend restores the text but drops attachedFiles.)
    expect(await screen.findByText('foo.txt')).toBeInTheDocument();
  });
});
