import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { act } from 'react';
import { useBrowserHibernation, HIBERNATION_SWEEP_MS } from '../src/hooks/useBrowserHibernation';
import type { PaneConfig, TabConfig } from '../src/types/pane';

const BUDGET = 5 * 60_000;

function pane(id: string, over: Partial<PaneConfig> = {}): PaneConfig {
  return { id, type: 'browser', title: id, ...over } as PaneConfig;
}

function tab(id: string, panes: PaneConfig[]): TabConfig {
  return { id, title: id, panes, activePaneId: panes[0]?.id ?? '' };
}

function setup(over: Partial<Parameters<typeof useBrowserHibernation>[0]> = {}) {
  const hibernatePane = vi.fn();
  const wakePane = vi.fn();
  const props = {
    tabs: [tab('active', [pane('p-active')]), tab('bg', [pane('p-bg')])],
    activeTabId: 'active',
    hibernateAfter: BUDGET,
    enabled: true,
    hibernatePane,
    wakePane,
    ...over,
  };
  const view = renderHook((p: typeof props) => useBrowserHibernation(p), {
    initialProps: props,
  });
  return { ...view, hibernatePane, wakePane, props };
}

/** Advance past N sweep intervals. */
function sweep(times = 1) {
  act(() => {
    vi.advanceTimersByTime(HIBERNATION_SWEEP_MS * times + 1);
  });
}

describe('useBrowserHibernation', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-30T12:00:00Z'));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('hibernates a browser pane once it has been out of sight for the budget', () => {
    const { hibernatePane, rerender, props } = setup({ activeTabId: 'bg' });

    // Leave 'bg' for 'active'. Only now does p-bg have a sighting to age.
    rerender({ ...props, activeTabId: 'active' });
    sweep();
    expect(hibernatePane).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(BUDGET);
    });
    sweep();
    expect(hibernatePane).toHaveBeenCalledWith('bg', 'p-bg');
    // ...and never the tab you are looking at.
    expect(hibernatePane).not.toHaveBeenCalledWith('active', 'p-active');
  });

  it('does nothing while disabled, even long past the budget', () => {
    const { hibernatePane } = setup({ enabled: false });
    act(() => {
      vi.advanceTimersByTime(BUDGET * 10);
    });
    sweep(5);
    expect(hibernatePane).not.toHaveBeenCalled();
  });

  it('does nothing when the budget is zero', () => {
    const { hibernatePane } = setup({ hibernateAfter: 0 });
    act(() => {
      vi.advanceTimersByTime(BUDGET * 10);
    });
    sweep(5);
    expect(hibernatePane).not.toHaveBeenCalled();
  });

  it('re-stamps a tab when it becomes active, resetting its clock', () => {
    // Start on 'bg' so both panes have a sighting, then leave it.
    const { hibernatePane, rerender, props } = setup({ activeTabId: 'bg' });
    rerender({ ...props, activeTabId: 'active' });

    act(() => {
      vi.advanceTimersByTime(BUDGET - 1000);
    });
    // Switch back to 'bg' just before it would have been due.
    rerender({ ...props, activeTabId: 'bg' });
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    sweep();
    // p-bg's clock restarted when we came back to it, so it is spared even
    // though more than a budget has passed since it was first stamped.
    expect(hibernatePane).not.toHaveBeenCalledWith('bg', 'p-bg');
    // p-active, meanwhile, has genuinely been away for longer than the budget.
    expect(hibernatePane).toHaveBeenCalledWith('active', 'p-active');
  });

  /// A pane whose tab has never been focused has no sighting, so it is never
  /// swept — see the `lastSeen > 0` guard in lib/hibernation. Worth stating
  /// out loud: it means a restored layout's unvisited browser tabs keep their
  /// webviews until the user opens them once.
  it('never hibernates a tab the user has not visited in this run', () => {
    const { hibernatePane } = setup({ activeTabId: 'active' });
    act(() => {
      vi.advanceTimersByTime(BUDGET * 5);
    });
    sweep(3);
    expect(hibernatePane).not.toHaveBeenCalled();
  });

  it('wakes hibernated panes in the tab that just became active', () => {
    const tabs = [
      tab('active', [pane('p-active')]),
      tab('bg', [pane('p-bg', { hibernated: true })]),
    ];
    const { wakePane, rerender, props } = setup({ tabs });
    expect(wakePane).not.toHaveBeenCalled();

    rerender({ ...props, tabs, activeTabId: 'bg' });
    expect(wakePane).toHaveBeenCalledWith('bg', 'p-bg');
  });

  it('stops sweeping once unmounted', () => {
    const { hibernatePane, unmount } = setup();
    act(() => {
      vi.advanceTimersByTime(BUDGET);
    });
    unmount();
    sweep(5);
    expect(hibernatePane).not.toHaveBeenCalled();
  });

  it('leaves a pane it has never seen alone', () => {
    // Only the active tab is stamped, so a pane that has never been on screen
    // has no sighting — hibernating on that reading would collapse panes a
    // restored layout just built.
    const tabs = [tab('bg1', [pane('a')]), tab('bg2', [pane('b')])];
    const { hibernatePane } = setup({ tabs, activeTabId: 'nothing-open' });
    act(() => {
      vi.advanceTimersByTime(BUDGET * 3);
    });
    sweep();
    expect(hibernatePane).not.toHaveBeenCalled();
  });
});
