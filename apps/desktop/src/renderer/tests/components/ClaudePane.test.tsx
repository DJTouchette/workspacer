import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * Focused characterization of ClaudePane's send pipeline — the seam that turns
 * a composer submit / approval click into the daemon call the GUI relies on
 * (see the "GUI send pipeline" and session-lifecycle work). The pane itself is
 * heavily entangled (xterm, PTY spawn, virtualized transcript), so every heavy
 * dependency is mocked to a stable stub and we drive the real handlers:
 *
 *   - handleSend        → window.electronAPI.claudeMessage(sessionId, text)
 *   - handleApprovalRespond → window.electronAPI.claudeApprove(sessionId, 'yes'|'no')
 *   - handleAnswer      → PTY write("<n>\r") (questions bypass /answer by design)
 */

// xterm needs a real canvas; stub it.
vi.mock('@xterm/xterm', () => {
  class MockTerminal {
    cols = 80; rows = 24;
    options: Record<string, unknown> = {};
    open = vi.fn(); write = vi.fn(); dispose = vi.fn(); focus = vi.fn(); blur = vi.fn();
    refresh = vi.fn(); clearSelection = vi.fn(); getSelection = vi.fn().mockReturnValue('');
    onData = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onBinary = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onResize = vi.fn().mockReturnValue({ dispose: vi.fn() });
    loadAddon = vi.fn(); attachCustomKeyEventHandler = vi.fn();
    parser = { registerCsiHandler: vi.fn() };
  }
  return { Terminal: MockTerminal };
});
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = vi.fn(); activate = vi.fn(); dispose = vi.fn(); } }));
vi.mock('@xterm/addon-web-fonts', () => ({
  WebFontsAddon: class { activate = vi.fn(); dispose = vi.fn(); loadFonts = vi.fn().mockResolvedValue(undefined); },
}));

// PTY-facing write, exposed so tests can assert the question-answer path.
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

// The session snapshot the pane renders from — swapped per test.
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
      terminal: { fontSize: 14, fontFamily: 'monospace', cursorBlink: true, scrollback: 1000, cursorStyle: 'block', shell: '', shells: [] },
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

const composer = () => screen.getByRole('textbox') as HTMLTextAreaElement;

describe('ClaudePane send pipeline', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    mockWrite.mockClear();
    (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
    (window.electronAPI.claudeApprove as any) = vi.fn().mockResolvedValue(undefined);
  });

  it('submitting a message calls claudeMessage with the session id and the typed text', async () => {
    render(<ClaudePane paneId="p1" title="Claude" isActive cwd="/repo" />);
    fireEvent.change(composer(), { target: { value: 'fix the failing test' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith('sess-1', 'fix the failing test'),
    );
  });

  it('shows the submitted text optimistically and clears the composer', async () => {
    render(<ClaudePane paneId="p2" title="Claude" isActive cwd="/repo" />);
    fireEvent.change(composer(), { target: { value: 'hello there' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(composer().value).toBe('');
    expect(await screen.findByText('hello there')).toBeInTheDocument();
  });

  it('does not send an empty / whitespace-only composer', () => {
    render(<ClaudePane paneId="p3" title="Claude" isActive cwd="/repo" />);
    fireEvent.change(composer(), { target: { value: '   ' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(window.electronAPI.claudeMessage).not.toHaveBeenCalled();
  });

  it('a rejected send (session ended) restores the text to the composer', async () => {
    (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: false, mode: 'stopped' });
    render(<ClaudePane paneId="p4" title="Claude" isActive cwd="/repo" />);
    fireEvent.change(composer(), { target: { value: 'are you alive' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() => expect(composer().value).toBe('are you alive'));
  });

  it('an Approval card Allow routes to claudeApprove("yes")', async () => {
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: { toolName: 'Bash', toolInput: { command: 'npm test' }, timestamp: Date.now() },
    });
    render(<ClaudePane paneId="p5" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Allow'));
    await waitFor(() => expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'yes'));
  });

  it('an Approval card Deny routes to claudeApprove("no")', async () => {
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: { toolName: 'Bash', toolInput: { command: 'rm -rf x' }, timestamp: Date.now() },
    });
    render(<ClaudePane paneId="p6" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Deny'));
    await waitFor(() => expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'no'));
  });

  it('answering a question writes the option number to the PTY (not /answer)', async () => {
    mockSession = makeSnapshot({
      lastActivity: Date.now(),
      pendingQuestions: [{ question: 'Pick one', options: [{ label: 'First' }, { label: 'Second' }] }],
    });
    render(<ClaudePane paneId="p7" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Second'));
    expect(mockWrite).toHaveBeenCalledWith('2\r');
  });
});
