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
    const { container } = render(pane());

    // Attach a file by dropping it on the pane (drops are pane-scoped — see the
    // drag & drop effect). This file carries the legacy `path` property, so it
    // also covers the fallback for when the webUtils bridge yields nothing.
    const dropEvent = new Event('drop', { bubbles: true, cancelable: true }) as any;
    dropEvent.dataTransfer = {
      files: { length: 1, 0: { path: '/tmp/foo.txt', name: 'foo.txt' } },
    };
    act(() => {
      (container.querySelector('[data-session-chat-view]') as Element).dispatchEvent(dropEvent);
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

it('captures before manager send, preserves a rejected draft request ID, and never falls back on unknown acknowledgement', async () => {
  mockSession = makeSnapshot({ isWakeTarget: true });
  const prepare = vi
    .fn()
    .mockResolvedValue({ available: true, requestId: 'host-request', delivery: 'pending' });
  window.electronAPI.managerRequestPrepare = prepare;
  const send = vi
    .fn()
    .mockResolvedValueOnce({ ok: false, requestId: 'host-request', delivery: 'rejected' })
    .mockResolvedValueOnce({ ok: false, requestId: 'host-request', delivery: 'unknown' });
  window.electronAPI.claudeMessage = send;
  render(pane());
  const composer = screen.getByRole('textbox');
  fireEvent.change(composer, { target: { value: 'Publish after fixes' } });
  fireEvent.keyDown(composer, { key: 'Enter' });
  await waitFor(() =>
    expect(send).toHaveBeenCalledWith('sess-1', 'Publish after fixes', 'host-request'),
  );
  expect(prepare.mock.invocationCallOrder[0]).toBeLessThan(send.mock.invocationCallOrder[0]);
  await waitFor(() => expect(composer).toHaveValue('Publish after fixes'));
  fireEvent.keyDown(composer, { key: 'Enter' });
  await screen.findByText(/Saved in request inbox. Chat delivery is unknown/);
  expect(prepare).toHaveBeenCalledOnce();
  expect(send).toHaveBeenCalledTimes(2);
  expect(mockWrite).not.toHaveBeenCalled();
  delete window.electronAPI.managerRequestPrepare;
});

it.each(['missing', 'unavailable', 'prepare-error', 'malformed', 'rejected', 'transport-error'])(
  'restores an uncaptured manager draft and attachments without replay (%s)',
  async (mode) => {
    mockSession = makeSnapshot({ isWakeTarget: true });
    delete window.electronAPI.managerRequestPrepare;
    if (mode === 'unavailable')
      window.electronAPI.managerRequestPrepare = vi
        .fn()
        .mockResolvedValue({ available: false, reason: 'Capture unavailable' });
    if (mode === 'prepare-error')
      window.electronAPI.managerRequestPrepare = vi
        .fn()
        .mockRejectedValue(new Error('Prepare unavailable'));
    if (mode === 'malformed')
      window.electronAPI.managerRequestPrepare = vi.fn().mockResolvedValue(undefined);
    const send = vi.fn();
    if (mode === 'rejected') send.mockResolvedValue({ ok: false, mode: 'stopped' });
    else send.mockRejectedValue(new Error('Fixture message transport unavailable'));
    window.electronAPI.claudeMessage = send;
    const { container } = render(pane());
    const drop = new Event('drop', { bubbles: true, cancelable: true }) as any;
    drop.dataTransfer = {
      files: { length: 1, 0: { path: '/tmp/retained.txt', name: 'retained.txt' } },
    };
    act(() => {
      container.querySelector('[data-session-chat-view]')!.dispatchEvent(drop);
    });
    await screen.findByText('retained.txt');
    const composer = screen.getByRole('textbox');
    fireEvent.change(composer, { target: { value: 'Uncaptured matrix request' } });
    fireEvent.keyDown(composer, { key: 'Enter' });
    await waitFor(() => expect(composer).toHaveValue('Uncaptured matrix request'));
    expect(screen.getByText('retained.txt')).toBeVisible();
    expect(screen.queryAllByText(/Uncaptured matrix request/, { selector: 'div' })).toHaveLength(0);
    expect(mockWrite).not.toHaveBeenCalled();
    expect(send).toHaveBeenCalledTimes(['prepare-error', 'malformed'].includes(mode) ? 0 : 1);
    expect(screen.queryByText(/Inspect the manager inbox/)).not.toBeInTheDocument();
    delete window.electronAPI.managerRequestPrepare;
  },
);

it.each(['throw', 'unknown', 'wrong-identity'])(
  'keeps an actually captured request durable without restoring a replayable draft (%s)',
  async (mode) => {
    mockSession = makeSnapshot({ isWakeTarget: true });
    window.electronAPI.managerRequestPrepare = vi
      .fn()
      .mockResolvedValue({ available: true, requestId: 'durable-id', delivery: 'pending' });
    const send = vi.fn();
    if (mode === 'throw') send.mockRejectedValue(new Error('Lost acknowledgement'));
    else
      send.mockResolvedValue({
        ok: mode === 'wrong-identity',
        requestId: mode === 'wrong-identity' ? 'foreign-id' : 'durable-id',
        delivery: mode === 'wrong-identity' ? 'accepted' : 'unknown',
      });
    window.electronAPI.claudeMessage = send;
    const { container } = render(pane());
    const composer = screen.getByRole('textbox');
    fireEvent.change(composer, { target: { value: 'Durable matrix request' } });
    fireEvent.keyDown(composer, { key: 'Enter' });
    await waitFor(() =>
      expect(send).toHaveBeenCalledExactlyOnceWith(
        'sess-1',
        'Durable matrix request',
        'durable-id',
      ),
    );
    await screen.findByText(
      mode === 'unknown' ? /Saved in request inbox/ : /Request delivery could not be confirmed/,
    );
    expect(composer).toHaveValue('');
    expect(
      screen.getAllByText('Durable matrix request', { exact: true, selector: 'div' }),
    ).toHaveLength(1);
    expect(mockWrite).not.toHaveBeenCalled();
    delete window.electronAPI.managerRequestPrepare;
  },
);

it('does not borrow a late capture ID for a draft explicitly edited while preparation was pending', async () => {
  mockSession = makeSnapshot({ isWakeTarget: true });
  let finish!: (value: unknown) => void;
  const prepare = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValueOnce({ available: true, requestId: 'new-draft-id', delivery: 'pending' });
  window.electronAPI.managerRequestPrepare = prepare;
  const send = vi
    .fn()
    .mockResolvedValueOnce({ ok: false, requestId: 'old-draft-id', delivery: 'rejected' })
    .mockResolvedValueOnce({ ok: true, requestId: 'new-draft-id', delivery: 'accepted' });
  window.electronAPI.claudeMessage = send;
  render(pane());
  const composer = screen.getByRole('textbox');
  fireEvent.change(composer, { target: { value: 'same words' } });
  fireEvent.keyDown(composer, { key: 'Enter' });
  await waitFor(() => expect(prepare).toHaveBeenCalledOnce());
  fireEvent.change(composer, { target: { value: 'same words' } });
  await act(async () => {
    finish({ available: true, requestId: 'old-draft-id', delivery: 'pending' });
  });
  await waitFor(() => expect(send).toHaveBeenCalledOnce());
  await waitFor(() => expect(composer).toHaveValue('same words'));
  fireEvent.keyDown(composer, { key: 'Enter' });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
  expect(prepare).toHaveBeenCalledTimes(2);
  expect(send.mock.calls.map((args) => args[2])).toEqual(['old-draft-id', 'new-draft-id']);
  delete window.electronAPI.managerRequestPrepare;
});

it('retries one captured card request without duplicate bubbles, but new identical submissions get new IDs', async () => {
  const { useClaudePaneModel } = await import('../../src/panes/ClaudePane');
  const { SessionChatView } = await import('../../src/panes/SessionChatView');
  let model!: ReturnType<typeof useClaudePaneModel>;
  function CardChat() {
    model = useClaudePaneModel({
      paneId: 'card-retry',
      title: 'Manager',
      isActive: true,
      cwd: '/repo',
    });
    return <SessionChatView {...model} />;
  }
  mockSession = makeSnapshot({ isWakeTarget: true });
  const prepare = vi
    .fn()
    .mockResolvedValueOnce({ available: true, requestId: 'card-id', delivery: 'pending' })
    .mockResolvedValueOnce({ available: true, requestId: 'new-id', delivery: 'pending' });
  window.electronAPI.managerRequestPrepare = prepare;
  const send = vi.fn(async (_id, _text, requestId) => ({
    ok: false,
    requestId,
    delivery: 'unknown',
  }));
  window.electronAPI.claudeMessage = send;
  const { container } = render(<CardChat />);
  await act(async () => {
    await model.handleSend('Identical card request');
  });
  await act(async () => {
    await model.handleSend('Identical card request');
  });
  expect(prepare).toHaveBeenCalledOnce();
  expect(send.mock.calls.map((args) => args[2])).toEqual(['card-id', 'card-id']);
  expect(
    screen.getAllByText('Identical card request', { exact: true, selector: 'div' }),
  ).toHaveLength(1);
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Identical card request' } });
  await act(async () => {
    await model.handleSend();
  });
  expect(prepare).toHaveBeenCalledTimes(2);
  expect(send.mock.calls[2][2]).toBe('new-id');
  expect(
    screen.getAllByText('Identical card request', { exact: true, selector: 'div' }),
  ).toHaveLength(2);
  delete window.electronAPI.managerRequestPrepare;
});

it('posts one in-app receipt per request ID and clears durable error text on accepted retry', async () => {
  const { useClaudePaneModel } = await import('../../src/panes/ClaudePane');
  const { SessionChatView } = await import('../../src/panes/SessionChatView');
  let model!: ReturnType<typeof useClaudePaneModel>;
  function Chat() {
    model = useClaudePaneModel({ paneId: 'receipt', title: 'Manager', isActive: true, cwd: '/repo' });
    return <SessionChatView {...model} />;
  }
  mockSession = makeSnapshot({ isWakeTarget: true });
  const received = vi.fn();
  window.addEventListener('wks:notify-post', received);
  window.electronAPI.managerRequestPrepare = vi.fn()
    .mockResolvedValueOnce({ available: true, requestId: 'one', delivery: 'pending' })
    .mockResolvedValueOnce({ available: true, requestId: 'one', delivery: 'pending' })
    .mockResolvedValueOnce({ available: true, requestId: 'two', delivery: 'pending' });
  window.electronAPI.claudeMessage = vi.fn()
    .mockResolvedValueOnce({ ok: false, requestId: 'one', delivery: 'rejected' })
    .mockResolvedValueOnce({ ok: true, requestId: 'one', delivery: 'accepted' })
    .mockResolvedValueOnce({ ok: true, requestId: 'one', delivery: 'accepted' })
    .mockResolvedValueOnce({ ok: true, requestId: 'two', delivery: 'accepted' });
  const view = render(<Chat />);
  try {
    await act(async () => { await model.handleSend('Same text'); });
    expect(screen.getByText(/Chat delivery rejected/)).toBeTruthy();
    expect(received).not.toHaveBeenCalled();
    await act(async () => { await model.handleSend('Same text'); });
    expect(screen.queryByText(/Chat delivery rejected|capture is pending|Request saved/)).toBeNull();
    expect(received).toHaveBeenCalledTimes(1);
    view.rerender(<Chat />);
    await act(async () => { await model.handleSend('Same text'); });
    expect(received).toHaveBeenCalledTimes(1);
    await act(async () => { await model.handleSend('Same text'); });
    expect(received).toHaveBeenCalledTimes(2);
    expect(received.mock.calls[1][0].detail).toMatchObject({ title: 'Request received', id: 'request-received:two' });
  } finally {
    window.removeEventListener('wks:notify-post', received);
    delete window.electronAPI.managerRequestPrepare;
  }
});
