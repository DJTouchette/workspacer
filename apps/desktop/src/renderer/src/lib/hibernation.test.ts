import { describe, it, expect } from 'vitest';
import {
  selectPanesToHibernate,
  selectPanesToWake,
  markVisible,
  isHibernatable,
} from './hibernation';
import type { PaneConfig, TabConfig } from '../types/pane';

const MINUTE = 60_000;
const BUDGET = 5 * MINUTE;

function pane(id: string, over: Partial<PaneConfig> = {}): PaneConfig {
  return { id, type: 'browser', title: id, ...over } as PaneConfig;
}

function tab(id: string, panes: PaneConfig[]): TabConfig {
  return { id, title: id, panes, activePaneId: panes[0]?.id ?? '' };
}

/** Two tabs: `active` on screen, `bg` behind it, each with one browser pane. */
function twoTabs(): TabConfig[] {
  return [tab('active', [pane('p-active')]), tab('bg', [pane('p-bg')])];
}

describe('selectPanesToHibernate', () => {
  it('picks a background browser pane once it is past the budget', () => {
    const due = selectPanesToHibernate({
      tabs: twoTabs(),
      activeTabId: 'active',
      lastVisible: { 'p-active': 1_000_000, 'p-bg': 1_000_000 },
      now: 1_000_000 + BUDGET + 1,
      hibernateAfter: BUDGET,
    });
    expect(due).toEqual([{ tabId: 'bg', paneId: 'p-bg' }]);
  });

  it('leaves the active tab alone however long it has been open', () => {
    const due = selectPanesToHibernate({
      tabs: twoTabs(),
      activeTabId: 'active',
      lastVisible: { 'p-active': 0 + 1, 'p-bg': 1_000_000 },
      now: 10 * 60 * MINUTE,
      hibernateAfter: BUDGET,
    });
    expect(due.map((d) => d.paneId)).toEqual(['p-bg']);
  });

  it('waits for the budget to be exceeded, not merely reached', () => {
    const at = (elapsed: number) =>
      selectPanesToHibernate({
        tabs: twoTabs(),
        activeTabId: 'active',
        lastVisible: { 'p-bg': 1_000_000 },
        now: 1_000_000 + elapsed,
        hibernateAfter: BUDGET,
      });
    expect(at(BUDGET - 1)).toEqual([]);
    // Exactly at the budget is not yet due — the comparison is strict.
    expect(at(BUDGET)).toEqual([]);
    expect(at(BUDGET + 1)).toHaveLength(1);
  });

  it('never hibernates a pane it has no sighting for', () => {
    // A pane with no entry (or a 0 stamp) has never been on screen as far as
    // the tracker knows. Hibernating on that reading would tear down panes
    // that were just restored from a saved layout.
    const noSighting: Record<string, number>[] = [{}, { 'p-bg': 0 }];
    for (const lastVisible of noSighting) {
      expect(
        selectPanesToHibernate({
          tabs: twoTabs(),
          activeTabId: 'active',
          lastVisible,
          now: 10 * 60 * MINUTE,
          hibernateAfter: BUDGET,
        }),
      ).toEqual([]);
    }
  });

  it('only reclaims browser panes — nothing else holds a webview', () => {
    const tabs = [
      tab('bg', [
        pane('b', { type: 'browser' }),
        pane('t', { type: 'terminal' }),
        pane('c', { type: 'claude' }),
      ]),
    ];
    const due = selectPanesToHibernate({
      tabs,
      activeTabId: 'other',
      lastVisible: { b: 1, t: 1, c: 1 },
      now: 10 * 60 * MINUTE,
      hibernateAfter: BUDGET,
    });
    expect(due.map((d) => d.paneId)).toEqual(['b']);
  });

  it('skips panes that are already hibernated, so the caller gets no no-ops', () => {
    const tabs = [tab('bg', [pane('awake'), pane('asleep', { hibernated: true })])];
    const due = selectPanesToHibernate({
      tabs,
      activeTabId: 'other',
      lastVisible: { awake: 1, asleep: 1 },
      now: 10 * 60 * MINUTE,
      hibernateAfter: BUDGET,
    });
    expect(due.map((d) => d.paneId)).toEqual(['awake']);
  });

  it('is disabled by a zero or negative budget', () => {
    for (const hibernateAfter of [0, -1]) {
      expect(
        selectPanesToHibernate({
          tabs: twoTabs(),
          activeTabId: 'active',
          lastVisible: { 'p-bg': 1 },
          now: 10 * 60 * MINUTE,
          hibernateAfter,
        }),
      ).toEqual([]);
    }
  });

  it('hibernates every background tab when no tab is active', () => {
    const due = selectPanesToHibernate({
      tabs: twoTabs(),
      activeTabId: undefined,
      lastVisible: { 'p-active': 1, 'p-bg': 1 },
      now: 10 * 60 * MINUTE,
      hibernateAfter: BUDGET,
    });
    expect(due.map((d) => d.paneId)).toEqual(['p-active', 'p-bg']);
  });

  it('returns picks in tab then pane order, so the writes are deterministic', () => {
    const tabs = [tab('t1', [pane('a'), pane('b')]), tab('t2', [pane('c')])];
    const due = selectPanesToHibernate({
      tabs,
      activeTabId: undefined,
      lastVisible: { a: 1, b: 1, c: 1 },
      now: 10 * 60 * MINUTE,
      hibernateAfter: BUDGET,
    });
    expect(due).toEqual([
      { tabId: 't1', paneId: 'a' },
      { tabId: 't1', paneId: 'b' },
      { tabId: 't2', paneId: 'c' },
    ]);
  });
});

describe('selectPanesToWake', () => {
  it('wakes only the hibernated panes of the active tab', () => {
    const tabs = [
      tab('active', [pane('awake'), pane('asleep', { hibernated: true })]),
      tab('bg', [pane('bg-asleep', { hibernated: true })]),
    ];
    expect(selectPanesToWake(tabs, 'active')).toEqual([{ tabId: 'active', paneId: 'asleep' }]);
  });

  it('is empty when there is no active tab, or it has gone away', () => {
    const tabs = [tab('active', [pane('asleep', { hibernated: true })])];
    expect(selectPanesToWake(tabs, undefined)).toEqual([]);
    expect(selectPanesToWake(tabs, 'deleted')).toEqual([]);
  });

  it('wakes any pane type, not just browsers', () => {
    // Hibernation only ever *creates* sleeping browser panes, but a layout
    // restored from disk can carry the flag on anything; waking is the safe
    // direction, so it is deliberately not type-gated.
    const tabs = [tab('active', [pane('t', { type: 'terminal', hibernated: true })])];
    expect(selectPanesToWake(tabs, 'active')).toHaveLength(1);
  });
});

describe('markVisible', () => {
  it('stamps every pane of the active tab', () => {
    const tabs = [tab('active', [pane('a'), pane('b')]), tab('bg', [pane('c')])];
    expect(markVisible({}, tabs, 'active', 500)).toEqual({ a: 500, b: 500 });
  });

  it('leaves sightings for panes that are off screen untouched', () => {
    const tabs = [tab('active', [pane('a')]), tab('bg', [pane('c')])];
    expect(markVisible({ c: 42 }, tabs, 'active', 500)).toEqual({ a: 500, c: 42 });
  });

  it('does not mutate the map it was given', () => {
    const tabs = [tab('active', [pane('a')])];
    const before = { a: 1 };
    const after = markVisible(before, tabs, 'active', 500);
    expect(before).toEqual({ a: 1 });
    expect(after).toEqual({ a: 500 });
  });

  it('returns the same map when there is nothing on screen to stamp', () => {
    const tabs = [tab('active', [pane('a')])];
    const before = { a: 1 };
    expect(markVisible(before, tabs, undefined, 9)).toBe(before);
    expect(markVisible(before, tabs, 'deleted', 9)).toBe(before);
  });
});

describe('isHibernatable', () => {
  it('is true only for an awake browser pane', () => {
    expect(isHibernatable(pane('b'))).toBe(true);
    expect(isHibernatable(pane('b', { hibernated: true }))).toBe(false);
    expect(isHibernatable(pane('t', { type: 'terminal' }))).toBe(false);
  });
});
