import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * "Your message rides the top of the viewport": sending pins the newest user
 * message to the top of the transcript by padding the tail with dead space, so
 * the reply streams into the room below it instead of both being crushed
 * against the composer. The pad math itself is unit-tested in
 * src/lib/chatScroll.test.ts — what's tested here is the wiring:
 *
 *   - the pin is armed by a SEND, not by opening a session (a restored
 *     transcript still opens at its natural bottom, no blank tail),
 *   - the anchor sits immediately above the newest user message.
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

beforeEach(() => {
  mockSession = makeSnapshot({ ambientState: 'idle' });
  (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
});

describe('ClaudePane pin-to-top tail', () => {
  it('adds no tail space to a restored transcript that was never sent to', () => {
    mockSession = makeSnapshot({
      conversation: [
        { role: 'user', content: 'earlier question', timestamp: 1 },
        { role: 'assistant', content: 'earlier answer', timestamp: 2 },
      ],
    } as Partial<ClaudeSessionSnapshot>);
    const { container } = render(pane());
    expect(container.querySelector('[data-tail-pad]')).toBeNull();
  });

  it('pads the tail once you send, so your message can sit at the top', async () => {
    const { container } = render(pane());
    expect(container.querySelector('[data-tail-pad]')).toBeNull();

    fireEvent.change(composer(), { target: { value: 'take it from here' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(await screen.findByText('take it from here')).toBeInTheDocument();

    const pad = container.querySelector('[data-tail-pad]') as HTMLElement | null;
    expect(pad).not.toBeNull();
    // Enough dead space to scroll the message up to the top of the (stubbed
    // 600px) viewport — see tailPadForAnchor.
    expect(parseInt(pad!.style.height, 10)).toBeGreaterThan(0);
  });

  it('anchors on the newest user message, not the first one', async () => {
    mockSession = makeSnapshot({
      conversation: [
        { role: 'user', content: 'first question', timestamp: 1 },
        { role: 'assistant', content: 'first answer', timestamp: 2 },
        { role: 'user', content: 'second question', timestamp: 3 },
      ],
    } as Partial<ClaudeSessionSnapshot>);
    const { container } = render(pane());

    const anchors = container.querySelectorAll('[data-pin-anchor]');
    expect(anchors).toHaveLength(1);
    expect(anchors[0].nextElementSibling?.textContent).toContain('second question');
  });
});
