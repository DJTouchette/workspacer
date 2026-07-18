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
