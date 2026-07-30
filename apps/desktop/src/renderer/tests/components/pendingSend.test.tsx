import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';
import { ConversationMessage } from '../../src/components/claude/ConversationMessage';

/**
 * "The agent doesn't have this yet." A user message is shown optimistically the
 * instant you hit Enter, but it isn't acknowledged until the daemon echoes the
 * turn back in the transcript — and when it's sent mid-turn it waits for the
 * current turn to finish first. Until then the bubble is marked:
 *
 *   idle agent      → "Sending…"  (brief, just the settle round-trip)
 *   agent mid-turn  → "Queued"    (lasts as long as the turn does)
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

vi.mock('../../src/hooks/useClaudeSpawn', () => ({
  useClaudeSpawn: vi.fn().mockReturnValue({
    sessionId: 'sess-1',
    isReady: true,
    spawnError: null,
    write: vi.fn(),
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
const composer = () => screen.getByRole('textbox') as HTMLTextAreaElement;

function send(text: string) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.keyDown(composer(), { key: 'Enter' });
}

beforeEach(() => {
  mockSession = makeSnapshot();
  (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
});

describe('unacknowledged sends', () => {
  it('marks a message sent while the agent is working as Queued', async () => {
    mockSession = makeSnapshot({ ambientState: 'streaming' });
    render(pane());
    send('and also update the docs');

    expect(await screen.findByText('and also update the docs')).toBeInTheDocument();
    expect(screen.getByText('Queued')).toBeInTheDocument();
    expect(screen.queryByText('Sending…')).toBeNull();
  });

  it('marks a message sent to an idle agent as Sending', async () => {
    render(pane());
    send('start on the refactor');

    expect(await screen.findByText('start on the refactor')).toBeInTheDocument();
    expect(screen.getByText('Sending…')).toBeInTheDocument();
    expect(screen.queryByText('Queued')).toBeNull();
  });

  it('treats a turn parked on an approval as work in progress, not idle', async () => {
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: { toolName: 'Bash', toolInput: { command: 'ls' }, timestamp: Date.now() },
    });
    render(pane());
    send('actually run the tests first');
    expect(await screen.findByText('Queued')).toBeInTheDocument();
  });

  it('queues a second send behind the first even from an idle agent', async () => {
    render(pane());
    send('first');
    expect(await screen.findByText('Sending…')).toBeInTheDocument();
    send('second');
    // The first is still on its way, so the second lands behind it.
    expect(await screen.findByText('Queued')).toBeInTheDocument();
    expect(screen.getByText('Sending…')).toBeInTheDocument();
  });

  it('drops the mark once the daemon echoes the turn back', async () => {
    const { rerender } = render(pane());
    send('take a look at the diff');
    expect(await screen.findByText('Sending…')).toBeInTheDocument();

    // The authoritative transcript now carries the turn — that IS the ack.
    mockSession = makeSnapshot({
      ambientState: 'thinking',
      conversation: [{ role: 'user', content: 'take a look at the diff', timestamp: Date.now() }],
      lastActivity: Date.now(),
    } as Partial<ClaudeSessionSnapshot>);
    rerender(pane());

    await waitFor(() => expect(screen.queryByText('Sending…')).toBeNull());
    // The message itself stays — only its provisional marking goes away.
    expect(screen.getByText('take a look at the diff')).toBeInTheDocument();
  });

  it('a rejected send takes the whole bubble away, mark and all', async () => {
    (window.electronAPI.claudeMessage as any) = vi
      .fn()
      .mockResolvedValue({ ok: false, mode: 'stopped' });
    render(pane());
    send('are you there');
    await waitFor(() => expect(screen.queryByText('Sending…')).toBeNull());
    // (scoped to a div — the restored draft in the textarea has the same text)
    expect(screen.queryByText('are you there', { selector: 'div' })).toBeNull();
    // …and it's back in the composer to retry.
    expect(composer().value).toBe('are you there');
  });
});

describe('pending bubble treatment', () => {
  const bubble = (c: HTMLElement) => c.querySelector('div[style*="border"]') as HTMLElement;

  it('is dashed and dimmed while unacknowledged', () => {
    const { container } = render(
      <ConversationMessage turn={{ role: 'user', content: 'hi', timestamp: 1 }} pending="queued" />,
    );
    expect(bubble(container).style.border).toContain('dashed');
    expect(parseFloat(bubble(container).style.opacity)).toBeLessThan(1);
  });

  it('is solid and opaque once acknowledged', () => {
    const { container } = render(
      <ConversationMessage turn={{ role: 'user', content: 'hi', timestamp: 1 }} />,
    );
    expect(bubble(container).style.border).toContain('solid');
    expect(bubble(container).style.opacity).toBe('1');
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('never marks an assistant turn', () => {
    render(
      <ConversationMessage
        turn={{ role: 'assistant', content: 'on it' }}
        pending={'queued' as never}
      />,
    );
    expect(screen.queryByText('Queued')).toBeNull();
  });
});
