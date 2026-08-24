/**
 * navigatePane must agree with the grid ScrollContainer actually renders.
 *
 * The count===3 layout is a main pane spanning both rows on the left, with
 * two panes stacked on the right (see TilingLayout in ScrollContainer.tsx):
 *
 *   [0000][1111]
 *   [0000][2222]
 *
 * Flat index arithmetic (idx +/- tilingColumns(count)) assumes every cell is
 * a uniform 1x1 grid square, which is false here: pane 1 sits directly above
 * pane 2, but idx(1) + cols(2) = 3, out of range for a 3-pane tab, so
 * nav-down silently did nothing from pane 1 even though a pane visually
 * sits right below it.
 */
import { describe, it, expect, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useKeyboardNav } from '../src/hooks/useKeyboardNav';
import type { TabConfig } from '../src/types/pane';

const baseOptions = () => ({
  tabs: [],
  activeTabId: 'tab-1',
  setActiveTabId: vi.fn(),
  scrollToTab: vi.fn(),
  addTab: vi.fn(() => 'tab-1'),
  splitTab: vi.fn(() => 'pane-1'),
  removeTab: vi.fn(),
  removePane: vi.fn(),
  renameTab: vi.fn(),
  moveTab: vi.fn(),
  onToggleHelp: vi.fn(),
});

const threePaneTab = (activePaneId: string): TabConfig =>
  ({
    id: 'tab-1',
    title: 'tab',
    panes: [
      { id: 'p0', type: 'terminal', title: 'p0' },
      { id: 'p1', type: 'terminal', title: 'p1' },
      { id: 'p2', type: 'terminal', title: 'p2' },
    ],
    activePaneId,
  }) as unknown as TabConfig;

const press = (over: Partial<KeyboardEventInit>): void => {
  window.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over }));
};

const armAndFire = (key: string) => {
  press({ key: ' ', code: 'Space', ctrlKey: true }); // arm the prefix chord
  press({ key }); // leaf
};

describe('navigatePane — spatial adjacency in the count===3 layout', () => {
  it('nav-down from the top-right pane (p1) moves to the bottom-right pane (p2)', () => {
    const setActivePane = vi.fn();
    const activeTab = threePaneTab('p1');
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        activeTab,
        setActivePane,
        prefix: 'ctrl+space',
        shortcuts: { 'nav-down': 'prefix j' },
      }),
    );
    armAndFire('j');
    expect(setActivePane).toHaveBeenCalledWith('tab-1', 'p2');
    unmount();
  });

  it('nav-up from the bottom-right pane (p2) moves to the top-right pane (p1), not the spanning left pane', () => {
    const setActivePane = vi.fn();
    const activeTab = threePaneTab('p2');
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        activeTab,
        setActivePane,
        prefix: 'ctrl+space',
        shortcuts: { 'nav-up': 'prefix k' },
      }),
    );
    armAndFire('k');
    expect(setActivePane).toHaveBeenCalledWith('tab-1', 'p1');
    unmount();
  });

  it('nav-right from the spanning left pane (p0) does nothing without a clear target when down is also valid', () => {
    // p0 spans both rows; nothing sits below it, so nav-down from p0 must be
    // a no-op rather than jumping across to p2 (the old idx+cols formula did).
    const setActivePane = vi.fn();
    const activeTab = threePaneTab('p0');
    const { unmount } = renderHook(() =>
      useKeyboardNav({
        ...baseOptions(),
        activeTab,
        setActivePane,
        prefix: 'ctrl+space',
        shortcuts: { 'nav-down': 'prefix j' },
      }),
    );
    armAndFire('j');
    expect(setActivePane).not.toHaveBeenCalled();
    unmount();
  });
});
