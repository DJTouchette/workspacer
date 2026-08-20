/**
 * The command layer's engine semantics (COMMAND_LAYER.md, Phase 2), pinned at
 * the hook level with real window events:
 *  - enabled + timeoutMs 0 → armed until resolved (no 1500ms expiry);
 *  - repeat groups re-arm for repeatMs, and the repeat window (only) dies the
 *    moment focus lands in an editable target;
 *  - any mousedown and window blur disarm;
 *  - `prefix prefix` sends the literal leader byte to the terminal the layer
 *    was armed from, addressed by the arm-time element;
 *  - a data-leader-suppress ancestor (modal dialogs) suppresses arming;
 *  - disabled layer keeps the legacy behavior (1500ms chord timeout).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useKeyboardNav } from '../src/hooks/useKeyboardNav';
import type { TabConfig } from '../src/types/pane';

const tabs: TabConfig[] = [
  { id: 'a', title: 'A', panes: [], activePaneId: '' },
  { id: 'b', title: 'B', panes: [], activePaneId: '' },
] as unknown as TabConfig[];

const layer = { enabled: true, timeoutMs: 0, repeatMs: 500, passthrough: true };

const options = (over: Record<string, unknown> = {}) => ({
  tabs,
  activeTabId: 'a',
  activeTab: undefined,
  setActiveTabId: vi.fn(),
  scrollToTab: vi.fn(),
  addTab: vi.fn(() => 't'),
  splitTab: vi.fn(() => 'p'),
  removeTab: vi.fn(),
  removePane: vi.fn(),
  renameTab: vi.fn(),
  moveTab: vi.fn(),
  setActivePane: vi.fn(),
  onToggleHelp: vi.fn(),
  prefix: 'ctrl+space',
  commandLayer: layer,
  shortcuts: { 'next-tab': 'prefix ]', 'toggle-help': 'prefix v' },
  ...over,
});

const press = (over: Partial<KeyboardEventInit>, target?: EventTarget): KeyboardEvent => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over });
  if (target) (target as HTMLElement).dispatchEvent(e);
  else window.dispatchEvent(e);
  return e;
};
const arm = () => press({ key: ' ', code: 'Space', ctrlKey: true });

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('command layer engine', () => {
  it('timeoutMs 0 = armed until resolved (no legacy 1500ms expiry)', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    vi.advanceTimersByTime(60_000);
    press({ key: 'v' }); // still armed a minute later — the leaf fires
    expect(opts.onToggleHelp).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('disabled layer keeps the legacy 1500ms chord timeout', () => {
    const opts = options({ commandLayer: { enabled: false } });
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    vi.advanceTimersByTime(2000);
    press({ key: 'v' }); // chord expired — nothing fires
    expect(opts.onToggleHelp).not.toHaveBeenCalled();
    unmount();
  });

  it('repeat-group leaves re-arm for repeatMs: prefix ] ] cycles twice', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    press({ key: ']' });
    press({ key: ']' }); // no second leader press
    expect(opts.setActiveTabId).toHaveBeenCalledTimes(2);
    // …and the window closes after repeatMs.
    vi.advanceTimersByTime(600);
    press({ key: ']' });
    expect(opts.setActiveTabId).toHaveBeenCalledTimes(2);
    unmount();
  });

  it('non-repeat leaves do NOT re-arm', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    press({ key: 'v' });
    expect(opts.onToggleHelp).toHaveBeenCalledTimes(1);
    press({ key: 'v' }); // layer disarmed — plain keystroke
    expect(opts.onToggleHelp).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('the repeat window dies when focus lands in an editable target', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    const textarea = document.createElement('textarea');
    document.body.appendChild(textarea);
    arm();
    press({ key: ']' }); // repeat window open
    // The composer autofocus firing mid-window: typed keys must be TYPING.
    textarea.dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
    press({ key: ']' });
    expect(opts.setActiveTabId).toHaveBeenCalledTimes(1);
    textarea.remove();
    unmount();
  });

  it('an explicit arm survives editable focus (only repeat windows cancel)', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    const textarea = document.createElement('textarea');
    document.body.appendChild(textarea);
    arm();
    textarea.dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
    press({ key: 'v' });
    expect(opts.onToggleHelp).toHaveBeenCalledTimes(1);
    textarea.remove();
    unmount();
  });

  it('any mousedown disarms', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    window.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    press({ key: 'v' });
    expect(opts.onToggleHelp).not.toHaveBeenCalled();
    unmount();
  });

  it('window blur disarms', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    window.dispatchEvent(new Event('blur'));
    press({ key: 'v' });
    expect(opts.onToggleHelp).not.toHaveBeenCalled();
    unmount();
  });

  it('prefix prefix sends the literal leader byte, addressed to the arm-time element', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    const term = document.createElement('div');
    document.body.appendChild(term);
    const input = document.createElement('textarea');
    term.appendChild(input);
    input.tabIndex = 0;
    input.focus();

    const received: { target: Element | null; bytes: string }[] = [];
    const onWrite = (e: Event) =>
      received.push((e as CustomEvent).detail as { target: Element | null; bytes: string });
    window.addEventListener('terminal:write-prefix', onWrite);

    arm();
    press({ key: ' ', code: 'Space', ctrlKey: true }); // leader again
    expect(received).toHaveLength(1);
    expect(received[0].bytes).toBe('\x00'); // ctrl+space → NUL
    expect(received[0].target).toBe(input); // the arm-time element, for pane addressing
    // Layer is disarmed after the passthrough.
    press({ key: 'v' });
    expect(opts.onToggleHelp).not.toHaveBeenCalled();

    window.removeEventListener('terminal:write-prefix', onWrite);
    term.remove();
    unmount();
  });

  it('never arms from inside a data-leader-suppress surface (modal dialogs)', () => {
    const opts = options();
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    const dialog = document.createElement('div');
    dialog.setAttribute('data-leader-suppress', 'true');
    const field = document.createElement('input');
    dialog.appendChild(field);
    document.body.appendChild(dialog);

    press({ key: ' ', code: 'Space', ctrlKey: true }, field);
    press({ key: 'v' });
    expect(opts.onToggleHelp).not.toHaveBeenCalled();
    dialog.remove();
    unmount();
  });
});

describe('command layer verbs (Phase 3)', () => {
  it('fires layer verbs from their chords, with the digit step carrying its slot', () => {
    const onZoomPane = vi.fn();
    const onJumpPinned = vi.fn();
    const onChatScroll = vi.fn();
    const opts = options({
      onZoomPane,
      onJumpPinned,
      onChatScroll,
      shortcuts: {
        'zoom-pane': 'prefix z',
        'jump-pinned': 'prefix 1-9',
        'chat-scroll-up': 'prefix shift+k',
        'nav-up': 'prefix k',
      },
    });
    const { unmount } = renderHook(() => useKeyboardNav(opts));

    arm();
    press({ key: 'z' });
    expect(onZoomPane).toHaveBeenCalledTimes(1);

    arm();
    press({ key: '3', code: 'Digit3' });
    expect(onJumpPinned).toHaveBeenCalledWith(3);

    // Case pairs are distinct steps: Shift+K scrolls, bare k navigates.
    arm();
    press({ key: 'K', shiftKey: true });
    expect(onChatScroll).toHaveBeenCalledWith('half-up');
    // chat-scroll is a repeat-group verb — still armed, K again pages again.
    press({ key: 'K', shiftKey: true });
    expect(onChatScroll).toHaveBeenCalledTimes(2);
    unmount();
  });

  it('a modified digit never resolves the 1-9 chord step (Ctrl+3 stays jump-tab territory)', () => {
    const onJumpPinned = vi.fn();
    const opts = options({ onJumpPinned, shortcuts: { 'jump-pinned': 'prefix 1-9' } });
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    arm();
    press({ key: '3', code: 'Digit3', ctrlKey: true });
    expect(onJumpPinned).not.toHaveBeenCalled();
    unmount();
  });
});

describe('cmdline + jumplist + the command:action door (Phase 5)', () => {
  it('prefix : opens the cmdline and prefix ctrl+o / ctrl+i walk the jumplist', () => {
    const onCmdline = vi.fn();
    const onJumpBack = vi.fn();
    const onJumpForward = vi.fn();
    const opts = options({
      onCmdline,
      onJumpBack,
      onJumpForward,
      shortcuts: {
        cmdline: 'prefix :',
        'jump-back': 'prefix ctrl+o',
        'jump-forward': 'prefix ctrl+i',
      },
    });
    const { unmount } = renderHook(() => useKeyboardNav(opts));

    arm();
    press({ key: ':', shiftKey: true }); // shift types the colon — shift-agnostic symbol step
    expect(onCmdline).toHaveBeenCalledTimes(1);

    arm();
    press({ key: 'o', ctrlKey: true });
    expect(onJumpBack).toHaveBeenCalledTimes(1);

    arm();
    press({ key: 'i', ctrlKey: true });
    expect(onJumpForward).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('command:action executes through the dispatcher switch (the cmdline/bus door)', () => {
    const onZoomPane = vi.fn();
    const opts = options({ onZoomPane, shortcuts: {} });
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    window.dispatchEvent(new CustomEvent('command:action', { detail: { action: 'zoom-pane' } }));
    expect(onZoomPane).toHaveBeenCalledTimes(1);
    // Garbage is ignored, not thrown.
    window.dispatchEvent(new CustomEvent('command:action', { detail: {} }));
    unmount();
  });
});

describe('command:action digit pass-through (bus/cmdline parity)', () => {
  it('carries the digit to digit-taking actions', () => {
    const onJumpPinned = vi.fn();
    const opts = options({ onJumpPinned, shortcuts: {} });
    const { unmount } = renderHook(() => useKeyboardNav(opts));
    window.dispatchEvent(
      new CustomEvent('command:action', { detail: { action: 'jump-pinned', digit: 4 } }),
    );
    expect(onJumpPinned).toHaveBeenCalledWith(4);
    unmount();
  });
});
