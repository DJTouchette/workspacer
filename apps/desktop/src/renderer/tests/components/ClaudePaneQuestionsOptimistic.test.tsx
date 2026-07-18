import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot, ConversationTurn } from '../../src/types/claudeSession';

/**
 * Regression coverage for two ClaudePane behaviors (mock scaffolding mirrors
 * ClaudePane.test.tsx — the pane is entangled with xterm/PTY, so every heavy
 * dependency is stubbed and we drive the real state machines):
 *
 * 1. Question-signature dismissal. Answering dismisses the picker by question
 *    CONTENT signature, not by timestamp — lastActivity bumps on everything,
 *    so the old timestamp gate re-prompted answered questions every frame
 *    until PostToolUse cleared the snapshot. The dismissal must reset when the
 *    snapshot's questions clear, so a textually identical LATER question set
 *    still re-opens the picker; a DIFFERENT set re-opens it immediately.
 *
 * 2. Optimistic-message re-baseline. Optimistic user bubbles dequeue FIFO as
 *    session.conversation grows. If the conversation RESETS under the same
 *    session id (managed-provider restart → fresh provider-side thread), the
 *    consumed count must re-baseline and pending optimistic turns must drop —
 *    their real counterparts belong to the dead thread and will never arrive.
 */

// xterm needs a real canvas; stub it.
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

// The session snapshot the pane renders from — swapped per test (and between
// rerenders, to simulate daemon snapshot updates).
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

const userTurn = (content: string, timestamp = Date.now()): ConversationTurn => ({
  role: 'user',
  content,
  timestamp,
});

const questionSet = () => [
  { question: 'Pick a strategy', options: [{ label: 'First' }, { label: 'Second' }] },
];

// A fresh element per (re)render — reusing one element reference would let
// React bail out on identical props and never pick up the swapped snapshot.
const pane = () => <ClaudePane paneId="p1" title="Claude" isActive cwd="/repo" />;

beforeEach(() => {
  mockSession = makeSnapshot();
  mockWrite.mockClear();
  (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
});

describe('ClaudePane question-signature dismissal', () => {
  it('answering hides the picker and keeps it hidden across activity bumps of the same question set', () => {
    mockSession = makeSnapshot({ pendingQuestions: questionSet() });
    const { rerender } = render(pane());

    expect(screen.getByText('Pick a strategy')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Second'));
    expect(mockWrite).toHaveBeenCalledWith('2\r');

    // Picker dismissed optimistically while the snapshot still carries the
    // questions: no clickable options remain. (The question TEXT persists by
    // design — the AnsweredQuestionCard keeps a durable trace in the chat.)
    expect(screen.queryByRole('button', { name: /First/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Second/ })).not.toBeInTheDocument();

    // The daemon re-delivers the SAME questions with a bumped lastActivity
    // (hooks bump it on everything) — the old timestamp gate re-prompted here.
    mockSession = makeSnapshot({
      pendingQuestions: questionSet(),
      lastActivity: Date.now() + 60_000,
    });
    rerender(pane());
    expect(screen.queryByRole('button', { name: /First/ })).not.toBeInTheDocument();
  });

  it('resets the dismissal when questions clear, so an identical later set re-opens the picker', () => {
    mockSession = makeSnapshot({ pendingQuestions: questionSet() });
    const { rerender } = render(pane());
    fireEvent.click(screen.getByText('First'));
    expect(screen.queryByRole('button', { name: /Second/ })).not.toBeInTheDocument();

    // PostToolUse clears the snapshot's questions — the answered request is over.
    mockSession = makeSnapshot({ pendingQuestions: null });
    rerender(pane());

    // A textually identical LATER question set must prompt again — the picker's
    // option buttons are back (the answered card's text never left).
    mockSession = makeSnapshot({ pendingQuestions: questionSet() });
    rerender(pane());
    expect(screen.getByRole('button', { name: /Second/ })).toBeInTheDocument();
  });

  it('a different question set re-opens the picker without needing a clear in between', () => {
    mockSession = makeSnapshot({ pendingQuestions: questionSet() });
    const { rerender } = render(pane());
    fireEvent.click(screen.getByText('First'));
    expect(screen.queryByRole('button', { name: /Second/ })).not.toBeInTheDocument();

    mockSession = makeSnapshot({
      pendingQuestions: [{ question: 'Deploy now?', options: [{ label: 'Yes' }, { label: 'No' }] }],
    });
    rerender(pane());
    expect(screen.getByText('Deploy now?')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Yes/ })).toBeInTheDocument();
  });
});

describe('ClaudePane optimistic-message re-baseline', () => {
  const composer = () => screen.getByRole('textbox') as HTMLTextAreaElement;

  it('dequeues the optimistic bubble when its real counterpart lands (no duplicate)', async () => {
    const { rerender } = render(pane());
    fireEvent.change(composer(), { target: { value: 'hello again' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(await screen.findByText('hello again')).toBeInTheDocument();

    // JSONL catches up: the same user message arrives in the conversation.
    mockSession = makeSnapshot({ conversation: [userTurn('hello again')] });
    rerender(pane());
    expect(screen.getAllByText('hello again')).toHaveLength(1);
  });

  it('keeps a pending optimistic bubble when a nameless command-output card arrives', async () => {
    const { rerender } = render(pane());

    // User sends a real message → optimistic bubble, still pending (real user
    // turn has not landed in session.conversation yet).
    fireEvent.change(composer(), { target: { value: 'my pending message' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(await screen.findByText('my pending message')).toBeInTheDocument();

    // Before the real user turn lands, a slash command's orphaned output arrives
    // → conversationApplier pushes a nameless command card (role:'user',
    // command.name===''). This is NOT a user send, so it must not dequeue the
    // still-pending optimistic bubble.
    mockSession = makeSnapshot({
      conversation: [
        {
          role: 'user',
          content: 'orphaned slash output',
          timestamp: Date.now(),
          command: { name: '', output: 'orphaned slash output' },
        } as ConversationTurn,
      ],
    });
    rerender(pane());

    expect(screen.getByText('my pending message')).toBeInTheDocument();
  });

  it('drops optimistic turns and re-baselines when the conversation resets under the same session id', async () => {
    // Established thread: two user turns already consumed.
    mockSession = makeSnapshot({
      conversation: [
        userTurn('first ask', 1),
        { role: 'assistant', content: 'done', timestamp: 2 },
        userTurn('second ask', 3),
      ],
    });
    const { rerender } = render(pane());

    fireEvent.change(composer(), { target: { value: 'after the restart' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(await screen.findByText('after the restart')).toBeInTheDocument();

    // Managed-provider restart: fresh provider-side thread, empty conversation.
    // The optimistic turn's real counterpart belongs to the dead thread — it
    // must drop rather than linger as a phantom bubble.
    mockSession = makeSnapshot({ conversation: [] });
    rerender(pane());
    expect(screen.queryByText('after the restart')).not.toBeInTheDocument();

    // The consumed count re-baselined to the new thread: its first user turn
    // renders once, and the dropped optimistic turn stays gone.
    mockSession = makeSnapshot({ conversation: [userTurn('fresh thread start')] });
    rerender(pane());
    expect(screen.getAllByText('fresh thread start')).toHaveLength(1);
    expect(screen.queryByText('after the restart')).not.toBeInTheDocument();
  });
});
