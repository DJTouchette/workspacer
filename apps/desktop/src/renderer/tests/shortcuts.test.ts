import { describe, it, expect } from 'vitest';
import { formatBinding, buildChordTree, chordNodeAt, chordMenu, chordBreadcrumb } from '../src/lib/shortcuts';

describe('formatBinding', () => {
  it('formats a direct combo', () => {
    expect(formatBinding('ctrl+shift+p')).toBe('Ctrl+Shift+P');
  });
  it('formats a single-step chord with the prefix', () => {
    expect(formatBinding('prefix v', 'ctrl+space')).toBe('Ctrl+Space V');
  });
  it('formats a multi-step chord', () => {
    expect(formatBinding('prefix t w', 'ctrl+space')).toBe('Ctrl+Space T W');
    expect(formatBinding('prefix t [', 'ctrl+space')).toBe('Ctrl+Space T [');
  });
});

describe('chord tree', () => {
  const shortcuts = {
    'command-palette': 'ctrl+shift+p', // direct — must be ignored by the tree
    'new-terminal': 'prefix n t',
    'new-claude': 'prefix n c',
    'close-pane': 'prefix t w',
    'next-tab': 'prefix t ]',
    'cycle-view': 'prefix v',
  };

  it('builds groups from multi-step paths and ignores direct bindings', () => {
    const tree = buildChordTree(shortcuts);
    const rootKeys = tree.children.map((c) => c.step).sort();
    expect(rootKeys).toEqual(['n', 't', 'v']);
  });

  it('descends into a submenu node', () => {
    const tree = buildChordTree(shortcuts);
    const nNode = chordNodeAt(tree, ['n']);
    expect(nNode?.children.map((c) => c.step).sort()).toEqual(['c', 't']);
    // leaf carries the action
    expect(chordNodeAt(tree, ['n', 't'])?.action).toBe('new-terminal');
  });

  it('lists groups before actions in the menu, with labels', () => {
    const tree = buildChordTree(shortcuts);
    const root = chordMenu(tree, []);
    // groups (New, Tab) come first, then the leaf (cycle-view → v)
    expect(root.map((i) => i.label)).toEqual(['New', 'Tab', 'Cycle view mode']);
    expect(root.find((i) => i.step === 'n')?.isGroup).toBe(true);
    expect(root.find((i) => i.step === 'v')?.isGroup).toBe(false);
  });

  it('resolves submenu items and breadcrumbs', () => {
    const tree = buildChordTree(shortcuts);
    const tabMenu = chordMenu(tree, ['t']);
    expect(tabMenu.map((i) => i.label).sort()).toEqual(['Close pane', 'Next tab']);
    expect(chordBreadcrumb(['t'])).toEqual(['Tab']);
  });
});
