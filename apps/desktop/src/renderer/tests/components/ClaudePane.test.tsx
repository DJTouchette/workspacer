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

// PTY-facing write + restart, exposed so tests can assert the question-answer
// path and the restart-preserves-transport contract.
const mockWrite = vi.fn();
const mockRestartSession = vi.fn().mockResolvedValue(undefined);
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
    restartSession: mockRestartSession,
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
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith(
        'sess-1',
        'fix the failing test',
      ),
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
    (window.electronAPI.claudeMessage as any) = vi
      .fn()
      .mockResolvedValue({ ok: false, mode: 'stopped' });
    render(<ClaudePane paneId="p4" title="Claude" isActive cwd="/repo" />);
    fireEvent.change(composer(), { target: { value: 'are you alive' } });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() => expect(composer().value).toBe('are you alive'));
  });

  it('an Approval card Allow routes to claudeApprove("yes")', async () => {
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: {
        toolName: 'Bash',
        toolInput: { command: 'npm test' },
        timestamp: Date.now(),
      },
    });
    render(<ClaudePane paneId="p5" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Allow'));
    await waitFor(() =>
      expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'yes'),
    );
  });

  it('an Approval card Deny routes to claudeApprove("no")', async () => {
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: {
        toolName: 'Bash',
        toolInput: { command: 'rm -rf x' },
        timestamp: Date.now(),
      },
    });
    render(<ClaudePane paneId="p6" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Deny'));
    await waitFor(() =>
      expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'no'),
    );
  });

  it('answering a question writes the option number to the PTY (not /answer)', async () => {
    mockSession = makeSnapshot({
      lastActivity: Date.now(),
      pendingQuestions: [
        { question: 'Pick one', options: [{ label: 'First' }, { label: 'Second' }] },
      ],
    });
    render(<ClaudePane paneId="p7" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByText('Second'));
    expect(mockWrite).toHaveBeenCalledWith('2\r');
  });

  it('Reply on a fleet wake entry prefixes the composer, and the prefix ships with the send', async () => {
    const { buildFleetMessage } = await import('../../../main/shared/fleetMessages');
    mockSession = makeSnapshot({
      conversation: [
        {
          role: 'user',
          content: buildFleetMessage('worker-finished', [
            { label: 'alpha: fix tests', sessionId: 'w1', cwd: '/home/u/Work/alpha' },
            { label: 'beta: ship', sessionId: 'w2', cwd: '/home/u/Work/beta' },
          ]),
        },
      ],
    });
    render(<ClaudePane paneId="p8" title="Claude" isActive cwd="/repo" />);
    fireEvent.click(await screen.findByTitle(/Reply to beta: ship/));
    expect(composer().value).toBe('Re: session:w2 (beta: ship) — ');

    fireEvent.change(composer(), {
      target: { value: 'Re: session:w2 (beta: ship) — ship it' },
    });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith(
        'sess-1',
        'Re: session:w2 (beta: ship) — ship it',
      ),
    );
  });
});

/**
 * Non-claude hybrids (codex) HAVE a terminal, so the old `!hasTerminal`-only
 * checks would happily type into it — but their questions/approvals are the
 * daemon's parked AskUserQuestion / approval gateway, which the codex TUI knows
 * nothing about. Both handlers must stay structural (`!hasTerminal || !isClaude`),
 * mirroring the claude-side tests above.
 */
describe('ClaudePane managed provider (codex hybrid) — structural answer/approve', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    mockWrite.mockClear();
    (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
    (window.electronAPI.claudeApprove as any) = vi.fn().mockResolvedValue(undefined);
    (window.electronAPI.claudeAnswer as any) = vi.fn().mockResolvedValue(undefined);
  });

  it('answering a codex question calls claudeAnswer with {option}, never the PTY', async () => {
    mockSession = makeSnapshot({
      lastActivity: Date.now(),
      pendingQuestions: [
        { question: 'Pick one', options: [{ label: 'First' }, { label: 'Second' }] },
      ],
    });
    render(<ClaudePane paneId="c1" title="Codex" isActive cwd="/repo" provider="codex" />);
    fireEvent.click(await screen.findByText('Second'));
    await waitFor(() =>
      expect(window.electronAPI.claudeAnswer).toHaveBeenCalledWith('sess-1', { option: 2 }),
    );
    expect(window.electronAPI.claudeAnswer).toHaveBeenCalledTimes(1);
    expect(mockWrite).not.toHaveBeenCalled();
  });

  it('a failed codex /approve never falls back to PTY keystrokes', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    (window.electronAPI.claudeApprove as any) = vi.fn().mockRejectedValue(new Error('409'));
    mockSession = makeSnapshot({
      ambientState: 'waiting_approval',
      pendingApproval: {
        toolName: 'exec_command',
        toolInput: { command: 'npm test' },
        timestamp: Date.now(),
      },
    });
    render(<ClaudePane paneId="c2" title="Codex" isActive cwd="/repo" provider="codex" />);
    fireEvent.click(await screen.findByText('Allow'));
    await waitFor(() =>
      expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'yes'),
    );
    // Let the rejection's .catch() run — the claude-only sendApproval fallback
    // would land here as mockWrite keystrokes.
    await new Promise((r) => setTimeout(r, 0));
    expect(mockWrite).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});

/**
 * Headless codex (session.transport === 'stream') is GUI-only: the daemon owns
 * the thread, there is no PTY. The daemon-stamped session transport is the ONLY
 * client-side discriminator, and a restart must carry it forward — otherwise a
 * model/effort restart silently flips headless → hybrid.
 */
describe('ClaudePane headless codex — surface gating + restart transport', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    mockWrite.mockClear();
    mockRestartSession.mockClear();
    (window.electronAPI.claudeApprove as any) = vi.fn().mockResolvedValue(undefined);
  });

  it('a stream-transport codex session hides the GUI/Term view toggle (GUI-only)', () => {
    mockSession = makeSnapshot({ transport: 'stream' });
    render(<ClaudePane paneId="h1" title="Codex" isActive cwd="/repo" provider="codex" />);
    expect(screen.getByText('Term').parentElement).toHaveStyle({ display: 'none' });
  });

  it('hybrid codex (no stream transport) keeps both surfaces and the toggle', () => {
    mockSession = makeSnapshot();
    render(<ClaudePane paneId="h2" title="Codex" isActive cwd="/repo" provider="codex" />);
    expect(screen.getByText('Term').parentElement).toHaveStyle({ display: 'flex' });
  });

  it("restarting a headless codex session preserves transport:'stream'", async () => {
    mockSession = makeSnapshot({ transport: 'stream', settings: { effort: 'low' } });
    render(<ClaudePane paneId="h3" title="Codex" isActive cwd="/repo" provider="codex" />);
    // Drive a restart through the effort pill's "Default" row — concrete levels
    // now switch live, but clearing the override still needs a relaunch.
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('Default'));
    fireEvent.click(await screen.findByText(/Restart with Default effort/));
    await waitFor(() =>
      expect(mockRestartSession).toHaveBeenCalledWith(
        expect.objectContaining({ effort: '', provider: 'codex', transport: 'stream' }),
      ),
    );
  });

  it("restarting a hybrid codex session keeps transport:'pty', independent of config", async () => {
    mockSession = makeSnapshot({ settings: { effort: 'low' } });
    render(<ClaudePane paneId="h4" title="Codex" isActive cwd="/repo" provider="codex" />);
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('Default'));
    fireEvent.click(await screen.findByText(/Restart with Default effort/));
    await waitFor(() =>
      expect(mockRestartSession).toHaveBeenCalledWith(
        expect.objectContaining({ effort: '', provider: 'codex', transport: 'pty' }),
      ),
    );
  });
});

describe('ClaudePane restore loader vs fresh spawn', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
  });

  it('a fresh spawn (attach viewer, no history expected) shows the hero, not the loader', () => {
    // Every spawned pane attaches as a viewer — attachSessionId alone must NOT
    // arm the "Fetching session…" state (the regression: fresh agents showed
    // it for 15s instead of the new-agent hero and prompt suggestions).
    render(<ClaudePane paneId="f1" title="Claude" isActive cwd="/repo" attachSessionId="sess-1" />);
    expect(screen.queryByText(/Fetching session/)).toBeNull();
  });

  it('a restore (expectHistory) shows the fetching state while the replay is pending', () => {
    render(
      <ClaudePane
        paneId="f2"
        title="Claude"
        isActive
        cwd="/repo"
        attachSessionId="sess-1"
        expectHistory
      />,
    );
    expect(screen.getByText(/Fetching session/)).toBeTruthy();
  });

  it('the fetching state yields to the conversation once history lands', () => {
    mockSession = makeSnapshot({
      conversation: [{ role: 'assistant', content: 'restored turn', timestamp: 1 }] as any,
    });
    render(
      <ClaudePane
        paneId="f3"
        title="Claude"
        isActive
        cwd="/repo"
        attachSessionId="sess-1"
        expectHistory
      />,
    );
    expect(screen.queryByText(/Fetching session/)).toBeNull();
    expect(screen.getByText('restored turn')).toBeTruthy();
  });
});

/**
 * Cancel routing — which interrupt a Stop/Escape actually delivers.
 *
 * Escape is Claude's interrupt key, so only claude PTY panes may cancel by
 * writing '\x1b' at the terminal. Every other provider must go out as SIGINT so
 * the daemon can route it to that provider's own structural abort (codex
 * `turn/interrupt`, opencode `POST /session/:id/abort`, pi `abort`) — all of
 * which are registered on the hybrid PTY path, not just the headless one.
 * Typing Escape into a hybrid codex/opencode/pi TUI is not its interrupt.
 */
describe('ClaudePane cancel routing', () => {
  beforeEach(() => {
    mockWrite.mockClear();
    (window.electronAPI.claudeSignal as any) = vi.fn().mockResolvedValue(undefined);
    // A live turn — cancel is a no-op unless the pane thinks it is streaming.
    mockSession = makeSnapshot({ ambientState: 'streaming', lastActivity: Date.now() });
  });

  it('a claude PTY session cancels with the Escape keystroke, not a signal', () => {
    render(<ClaudePane paneId="c1" title="Claude" isActive cwd="/repo" />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(mockWrite).toHaveBeenCalledWith('\x1b');
    expect(window.electronAPI.claudeSignal).not.toHaveBeenCalled();
  });

  it.each(['codex', 'opencode', 'pi'] as const)(
    'a hybrid %s session cancels with SIGINT so the daemon uses that provider’s abort',
    (provider) => {
      render(
        <ClaudePane
          paneId={`c-${provider}`}
          title={provider}
          isActive
          cwd="/repo"
          provider={provider}
        />,
      );
      fireEvent.keyDown(window, { key: 'Escape' });
      expect(window.electronAPI.claudeSignal).toHaveBeenCalledWith('sess-1', 'SIGINT');
      expect(mockWrite).not.toHaveBeenCalledWith('\x1b');
    },
  );
});

/**
 * Composer paste routing. A paste is one of four things and the pane has to
 * tell them apart: files with a host path (attach), an image with none (spill
 * to a temp PNG, then attach), a large text dump (collapse to a marker so it
 * can't bury the draft), or ordinary text (leave alone). The collapse is only
 * safe if the marker expands back to the exact bytes on send — that round trip
 * is what these pin.
 */
function clipboard(opts: {
  text?: string;
  files?: File[];
  items?: { kind: string; type: string }[];
}): any {
  return {
    files: opts.files ?? [],
    items: opts.items ?? [],
    getData: (type: string) => (type === 'text/plain' ? (opts.text ?? '') : ''),
  };
}

const bigPaste = Array.from({ length: 40 }, (_, i) => `log line ${i}`).join('\n');

describe('ClaudePane composer paste', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    mockWrite.mockClear();
    (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
    (window.electronAPI.saveClipboardImage as any) = vi.fn().mockResolvedValue(null);
    (window.electronAPI.getPathForFile as any) = vi.fn().mockReturnValue('');
  });

  it('collapses a large paste to a marker instead of filling the composer', () => {
    render(<ClaudePane paneId="pp1" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    expect(composer().value).toBe('[Pasted text #1 +40 lines]');
    expect(composer().value).not.toContain('log line 0');
  });

  it('sends the full pasted text, not the marker', async () => {
    render(<ClaudePane paneId="pp2" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    fireEvent.change(composer(), {
      target: { value: `what broke here?\n${composer().value}` },
    });
    fireEvent.keyDown(composer(), { key: 'Enter' });

    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith(
        'sess-1',
        `what broke here?\n${bigPaste}`,
      ),
    );
  });

  it('a rejected send restores the marker, and the retry still expands it', async () => {
    (window.electronAPI.claudeMessage as any) = vi
      .fn()
      .mockResolvedValueOnce({ ok: false, mode: 'stopped' })
      .mockResolvedValue({ ok: true });
    render(<ClaudePane paneId="pp3" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    fireEvent.keyDown(composer(), { key: 'Enter' });

    // The draft comes back readable (the marker), not the 40-line expansion.
    await waitFor(() => expect(composer().value).toBe('[Pasted text #1 +40 lines]'));

    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenLastCalledWith('sess-1', bigPaste),
    );
  });

  it('leaves a short paste to the browser (no marker, no interception)', () => {
    render(<ClaudePane paneId="pp4" title="Claude" isActive cwd="/repo" />);
    const ev = fireEvent.paste(composer(), { clipboardData: clipboard({ text: 'one liner' }) });
    expect(ev).toBe(true); // not preventDefault-ed
    expect(composer().value).toBe('');
  });

  it('spills a pasted screenshot to a file and attaches it', async () => {
    (window.electronAPI.saveClipboardImage as any) = vi
      .fn()
      .mockResolvedValue({ path: '/tmp/workspacer-pasted/pasted-1.png', width: 10, height: 10 });
    render(<ClaudePane paneId="pp5" title="Claude" isActive cwd="/repo" />);

    fireEvent.paste(composer(), {
      clipboardData: clipboard({ items: [{ kind: 'file', type: 'image/png' }] }),
    });

    await waitFor(() => expect(window.electronAPI.saveClipboardImage).toHaveBeenCalled());
    expect(await screen.findByText('pasted-1.png')).toBeInTheDocument();
  });

  it('does not spill anything for a plain-text paste', () => {
    render(<ClaudePane paneId="pp6" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: 'hi' }) });
    expect(window.electronAPI.saveClipboardImage).not.toHaveBeenCalled();
  });
});

/**
 * File drop. Handled on the pane's own element, not the document: a file
 * dropped on a browser pane or the editor belongs to them, and used to be
 * hijacked by whichever chat pane was active.
 */
describe('ClaudePane file drop', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    (window.electronAPI.getPathForFile as any) = vi.fn().mockReturnValue('/repo/shot.png');
  });

  const dropData = () => ({
    files: [new File(['x'], 'shot.png')],
    items: [],
    types: ['Files'],
    getData: () => '',
  });

  it('attaches a file dropped on the pane', async () => {
    const { container } = render(<ClaudePane paneId="d1" title="Claude" isActive cwd="/repo" />);
    fireEvent.drop(container.firstChild as Element, { dataTransfer: dropData() });
    expect(await screen.findByText('shot.png')).toBeInTheDocument();
  });

  it('ignores a drop outside the pane (another pane owns it)', () => {
    render(<ClaudePane paneId="d2" title="Claude" isActive cwd="/repo" />);
    fireEvent.drop(document.body, { dataTransfer: dropData() });
    expect(screen.queryByText('shot.png')).not.toBeInTheDocument();
  });
});

/**
 * Regressions found by the v0.147.0 bug hunt, both reproduced before the fix:
 *  - a transient edit that broke a marker's shape destroyed the held paste, and
 *    the repaired marker then went out to the agent as a literal string;
 *  - a clipboard carrying BOTH text and a bitmap (spreadsheet/word-processor
 *    copy) was treated as a screenshot, silently swallowing the text.
 */
describe('ClaudePane paste regressions', () => {
  beforeEach(() => {
    mockSession = makeSnapshot();
    (window.electronAPI.claudeMessage as any) = vi.fn().mockResolvedValue({ ok: true });
    (window.electronAPI.saveClipboardImage as any) = vi.fn().mockResolvedValue(null);
    (window.electronAPI.getPathForFile as any) = vi.fn().mockReturnValue('');
  });

  it('survives the marker being broken and repaired mid-edit', async () => {
    render(<ClaudePane paneId="pr1" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    const marker = composer().value;

    // Backspace the closing bracket, then type it back.
    fireEvent.change(composer(), { target: { value: marker.slice(0, -1) } });
    fireEvent.change(composer(), { target: { value: marker } });
    fireEvent.keyDown(composer(), { key: 'Enter' });

    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith('sess-1', bigPaste),
    );
  });

  it('survives the marker being cut away and pasted back elsewhere', async () => {
    render(<ClaudePane paneId="pr2" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    const marker = composer().value;

    fireEvent.change(composer(), { target: { value: '' } }); // cut
    fireEvent.change(composer(), { target: { value: 'why: ' } }); // type context
    // Re-paste: short text, so the browser's own insertion applies.
    fireEvent.change(composer(), { target: { value: `why: ${marker}` } });
    fireEvent.keyDown(composer(), { key: 'Enter' });

    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith('sess-1', `why: ${bigPaste}`),
    );
  });

  it('still expands after a rejected send even if the composer was touched meanwhile', async () => {
    (window.electronAPI.claudeMessage as any) = vi
      .fn()
      .mockResolvedValueOnce({ ok: false, mode: 'stopped' })
      .mockResolvedValue({ ok: true });
    render(<ClaudePane paneId="pr3" title="Claude" isActive cwd="/repo" />);
    fireEvent.paste(composer(), { clipboardData: clipboard({ text: bigPaste }) });
    fireEvent.keyDown(composer(), { key: 'Enter' });
    // Typing during the in-flight send used to prune the block out from under
    // the restore.
    fireEvent.change(composer(), { target: { value: ' ' } });
    fireEvent.change(composer(), { target: { value: '' } });

    await waitFor(() => expect(composer().value).toMatch(/^\[Pasted text #1/));
    fireEvent.keyDown(composer(), { key: 'Enter' });
    await waitFor(() =>
      expect(window.electronAPI.claudeMessage).toHaveBeenLastCalledWith('sess-1', bigPaste),
    );
  });

  it('pastes the text when the clipboard carries text AND a bitmap (spreadsheet copy)', () => {
    render(<ClaudePane paneId="pr4" title="Claude" isActive cwd="/repo" />);
    const ev = fireEvent.paste(composer(), {
      clipboardData: clipboard({
        text: 'a\tb\tc\n1\t2\t3',
        items: [
          { kind: 'string', type: 'text/plain' },
          { kind: 'file', type: 'image/png' },
        ],
      }),
    });
    // Left to the browser to insert, and nothing spilled to disk.
    expect(ev).toBe(true);
    expect(window.electronAPI.saveClipboardImage).not.toHaveBeenCalled();
  });

  it('attaching the same file twice keeps one chip', async () => {
    (window.electronAPI.getPathForFile as any) = vi.fn().mockReturnValue('/repo/shot.png');
    const { container } = render(<ClaudePane paneId="pr5" title="Claude" isActive cwd="/repo" />);
    const data = () => ({
      files: [new File(['x'], 'shot.png')],
      items: [],
      types: ['Files'],
      getData: () => '',
    });
    fireEvent.drop(container.firstChild as Element, { dataTransfer: data() });
    fireEvent.drop(container.firstChild as Element, { dataTransfer: data() });

    expect(await screen.findAllByText('shot.png')).toHaveLength(1);
  });
});
