import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot, ConversationTurn } from '../../src/types/claudeSession';

/**
 * The "Working for 1m 04s" label must measure THIS turn, on every transport.
 *
 * Reproduced live against `workspacer serve` (a headless brain fleet — the same
 * shape a Fly node serves to `/app`): turn 1 read "Working for 2s", turn 2
 * opened at "Working for 34s" and turn 3 at "Working for 1m 20s". The label was
 * counting from the FIRST turn of the session, idle gaps included.
 *
 * Two things made that happen, and this file pins both:
 *
 *   1. The bus backend folds the transcript into a buffer it mutates in place
 *      and hands out as-is, so `session.conversation` kept ONE array identity
 *      for the session's whole life. ClaudePane memoizes the anchor on exactly
 *      that identity, so it froze a turn behind. (The seam half is fixed in
 *      busConversation.merge; see its test.)
 *   2. Even with a fresh array, the transcript reaches a bus client through a
 *      SEPARATE `sessions.conversation` fetch than the state flip does, so a
 *      turn routinely starts before its own user message has landed. Anchoring
 *      to whatever user turn is newest then means anchoring to the previous
 *      turn's message — and counting the wait between turns as work.
 *
 * The rule the tests below encode: a user turn stamped before the last idle we
 * WATCHED cannot be this run's start. An attach mid-turn (no idle observed)
 * still trusts the transcript, because there the old timestamp is the truth.
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

const T0 = 1_800_000_000_000;

/** The bus seam's transcript: ONE array the fold mutates in place. Snapshots
 *  copy it (busConversation.merge) exactly as Electron IPC would. */
let turns: ConversationTurn[];

/** Advance the clock and let the label's own 1s interval catch up. */
function tick(ms: number) {
  act(() => void vi.advanceTimersByTime(ms));
}

/** The daemon flipping this session's state, with the transcript it has now. */
function push(ambientState: ClaudeSessionSnapshot['ambientState']) {
  mockSession = makeSnapshot({
    ambientState,
    conversation: turns.slice() as any,
    lastActivity: Date.now(),
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(T0);
  turns = [];
  mockSession = makeSnapshot({ ambientState: 'idle' });
  mockWrite.mockClear();
  (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
});

afterEach(() => {
  vi.useRealTimers();
});

const label = () => screen.queryByText(/Working for/)?.textContent ?? null;

describe('the working label re-anchors on every turn boundary', () => {
  it('starts each turn at zero, not from the first turn of the session', () => {
    const { rerender } = render(pane());

    // ── turn 1: the user message lands with the state flip ──
    turns.push({ role: 'user', content: 'first ask', timestamp: Date.now() });
    push('streaming');
    rerender(pane());
    expect(label()).toBe('Working for 0s');
    tick(30_000);
    expect(label()).toBe('Working for 30s');

    // ── the turn ends, and the user reads the answer for a while ──
    push('idle');
    rerender(pane());
    expect(label()).toBeNull();
    tick(60_000);

    // ── turn 2: the state flip arrives BEFORE the fetched transcript grows,
    // so the newest user turn is still turn 1's message. Anchoring to it is
    // what produced "Working for 1m 30s" on a turn one second old. ──
    push('streaming');
    rerender(pane());
    expect(label(), 'a new turn starts its own clock').toBe('Working for 0s');
    tick(5_000);
    expect(label()).toBe('Working for 5s');

    // The transcript catching up mid-turn must not move the anchor either.
    turns.push({ role: 'user', content: 'second ask', timestamp: Date.now() - 5_000 });
    push('streaming');
    rerender(pane());
    expect(label()).toBe('Working for 5s');
  });

  it('keeps the wall clock since you asked when the message arrives with the turn', () => {
    const { rerender } = render(pane());

    turns.push({ role: 'user', content: 'first ask', timestamp: Date.now() });
    push('streaming');
    rerender(pane());
    push('idle');
    rerender(pane());
    tick(20_000);

    // Turn 2's message IS in the transcript when the turn starts (the desktop's
    // normal ordering — the store folds the user turn as it flips the state).
    // Its timestamp is 3s old: the daemon settled the send before the turn
    // opened, and that wait is part of what the user is waiting through.
    const asked = Date.now() - 3_000;
    turns.push({ role: 'user', content: 'second ask', timestamp: asked });
    push('streaming');
    rerender(pane());
    expect(label(), "the turn's own message wins over the local clock").toBe('Working for 3s');
  });

  it('reports the real wait when attaching to a session already mid-turn', () => {
    // No idle was ever observed here — the pane opened onto a running turn, so
    // the transcript's timestamp is the only evidence of when it started, and
    // restarting the clock at zero would understate the wait.
    turns.push({ role: 'user', content: 'asked before we attached', timestamp: T0 - 95_000 });
    push('streaming');
    render(pane());
    expect(label()).toBe('Working for 1m 35s');
  });

  it('does not restart the clock when a turn parks on an approval', () => {
    const { rerender } = render(pane());

    turns.push({ role: 'user', content: 'first ask', timestamp: Date.now() });
    push('streaming');
    rerender(pane());
    tick(10_000);

    // waiting_approval is NOT idle: the turn is still open, and the clock the
    // user is watching must keep running across the approval.
    push('waiting_approval');
    rerender(pane());
    tick(5_000);
    push('streaming');
    rerender(pane());
    expect(label()).toBe('Working for 15s');
  });
});
