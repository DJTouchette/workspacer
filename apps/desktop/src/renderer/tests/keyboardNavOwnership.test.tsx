/**
 * useKeyboardNav ownership of library-picker and toggle-inspector.
 *
 * Both actions used to live in separate listeners (usePluginHotkeys /
 * ClaudePane) whose inline matchers never resolved the `mod` token — so their
 * default bindings (mod+shift+l / mod+shift+e) were DEAD in production: the
 * parsed combo demanded ctrlKey=false while the real keystroke carried Ctrl.
 * They now dispatch through executeAction like every other binding; these
 * tests pin the regression at the hook level, real window events included.
 */
import { describe, it, expect, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useKeyboardNav } from '../src/hooks/useKeyboardNav';

const baseOptions = () => ({
  tabs: [],
  activeTabId: '',
  activeTab: undefined,
  setActiveTabId: vi.fn(),
  scrollToTab: vi.fn(),
  addTab: vi.fn(() => 'tab-1'),
  splitTab: vi.fn(() => 'pane-1'),
  removeTab: vi.fn(),
  removePane: vi.fn(),
  renameTab: vi.fn(),
  moveTab: vi.fn(),
  setActivePane: vi.fn(),
  onToggleHelp: vi.fn(),
});

const press = (over: Partial<KeyboardEventInit>): KeyboardEvent => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over });
  window.dispatchEvent(e);
  return e;
};

describe('useKeyboardNav — library-picker / toggle-inspector ownership', () => {
  it('fires onLibraryPicker on the REAL resolved combo (mod+shift+l → Ctrl+Shift+L)', () => {
    const onLibraryPicker = vi.fn();
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        onLibraryPicker,
        shortcuts: { 'library-picker': 'mod+shift+l' },
      }),
    );
    const e = press({ key: 'L', ctrlKey: true, shiftKey: true });
    expect(onLibraryPicker).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true); // owned → consumed
    unmount();
  });

  it('fires onToggleInspector on Ctrl+Shift+E and consumes the event', () => {
    const onToggleInspector = vi.fn();
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        onToggleInspector,
        shortcuts: { 'toggle-inspector': 'mod+shift+e' },
      }),
    );
    const e = press({ key: 'E', ctrlKey: true, shiftKey: true });
    expect(onToggleInspector).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true);
    unmount();
  });

  it('leaves the event unconsumed when no handler is wired (fall-through preserved)', () => {
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        shortcuts: { 'library-picker': 'mod+shift+l' },
      }),
    );
    const e = press({ key: 'L', ctrlKey: true, shiftKey: true });
    expect(e.defaultPrevented).toBe(false); // not owned → other listeners may take it
    unmount();
  });

  it('fires them as chord leaves too (prefix-bound rebind)', () => {
    const onLibraryPicker = vi.fn();
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        onLibraryPicker,
        prefix: 'ctrl+space',
        shortcuts: { 'library-picker': 'prefix /' },
      }),
    );
    press({ key: ' ', code: 'Space', ctrlKey: true }); // arm
    press({ key: '/' }); // leaf
    expect(onLibraryPicker).toHaveBeenCalledTimes(1);
    unmount();
  });
});
