/**
 * Regression: openPaneIn dedupes single-pane tabs by (type, title) only. For
 * editor-plugin panes the title is the file's basename, so two different files
 * with the same basename (src/index.ts vs test/index.ts) collide onto one pane
 * and the second open never navigates (it only sets activeTabId, never updating
 * url). The dedupe key must include the pane url — mirroring openMarkdownPreview,
 * which dedupes on the full previewPath.
 */
import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager, GLOBAL_WORKSPACE_ID } from '../src/hooks/useAgentManager';

describe('openPaneIn — editor plugin dedupe by full url, not basename', () => {
  it('opens a distinct pane when a different file shares a basename', () => {
    const { result } = renderHook(() => useAgentManager());

    let firstTab = '';
    act(() => {
      firstTab = result.current.openPaneIn(
        GLOBAL_WORKSPACE_ID,
        'plugin',
        'index.ts',
        'plugin://editor?file=src/index.ts',
        'src',
        'workspacer.editor',
      );
    });

    let secondTab = '';
    act(() => {
      secondTab = result.current.openPaneIn(
        GLOBAL_WORKSPACE_ID,
        'plugin',
        'index.ts',
        'plugin://editor?file=test/index.ts',
        'test',
        'workspacer.editor',
      );
    });

    const global = result.current.agents.find((a: any) => a.id === GLOBAL_WORKSPACE_ID)!;
    const activeTab = global.tabs.find((t: any) => t.id === global.activeTabId)!;

    // The focused editor must show the file the user actually clicked.
    expect(activeTab.panes[0].url).toBe('plugin://editor?file=test/index.ts');
    // Two different files must not collapse onto a single pane.
    expect(secondTab).not.toBe(firstTab);
  });

  it('still dedupes when the same file (same url) is opened twice', () => {
    const { result } = renderHook(() => useAgentManager());
    let a = '';
    let b = '';
    act(() => {
      a = result.current.openPaneIn(GLOBAL_WORKSPACE_ID, 'plugin', 'x.ts', 'u://x', 'd', 'p');
    });
    act(() => {
      b = result.current.openPaneIn(GLOBAL_WORKSPACE_ID, 'plugin', 'x.ts', 'u://x', 'd', 'p');
    });
    expect(b).toBe(a);
  });
});

it('restores and focuses the one global Recent agents pane even in a split tab', () => {
  const { result } = renderHook(() => useAgentManager());
  let first = '';
  act(() => {
    first = result.current.openPaneIn(GLOBAL_WORKSPACE_ID, 'recentagents', 'Recent agents');
  });
  const saved = JSON.parse(JSON.stringify(result.current.agents));
  const ws = saved.find((a: { id: string }) => a.id === GLOBAL_WORKSPACE_ID);
  const tab = ws.tabs.find((t: { id: string }) => t.id === first);
  const recentId = tab.panes[0].id;
  tab.panes.push({ id: 'other', type: 'board', title: 'Board' });
  tab.activePaneId = 'other';
  act(() => {
    result.current.loadAgentsFromSession(saved, GLOBAL_WORKSPACE_ID);
  });
  let again = '';
  act(() => {
    again = result.current.openPaneIn(GLOBAL_WORKSPACE_ID, 'recentagents', 'Recent agents');
  });
  expect(again).toBe(first);
  const restored = result.current.agents.find((a) => a.id === GLOBAL_WORKSPACE_ID)!;
  expect(
    restored.tabs.flatMap((t) => t.panes).filter((p) => p.type === 'recentagents'),
  ).toHaveLength(1);
  expect(restored.tabs.find((t) => t.id === first)?.activePaneId).toBe(recentId);
});
