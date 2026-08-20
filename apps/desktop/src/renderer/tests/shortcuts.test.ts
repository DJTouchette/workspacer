import { describe, it, expect } from 'vitest';
import {
  formatBinding,
  buildChordTree,
  buildXtermAppKeyPredicate,
  chordNodeAt,
  findChordConflicts,
  leaderPassthroughBytes,
  chordMenu,
  chordBreadcrumb,
  resolveLeader,
} from '../src/lib/shortcuts';

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

describe('resolveLeader', () => {
  it('substitutes the IME-grabbed ctrl+space with a lone Alt tap on Linux', () => {
    expect(resolveLeader('ctrl+space', true)).toBe('alt');
    // Case/whitespace-insensitive on the stored default.
    expect(resolveLeader(' Ctrl+Space ', true)).toBe('alt');
  });
  it('leaves ctrl+space alone off Linux', () => {
    expect(resolveLeader('ctrl+space', false)).toBe('ctrl+space');
  });
  it('never touches a leader other than ctrl+space, even on Linux', () => {
    // A user's own rebind survives on every platform.
    expect(resolveLeader('ctrl+shift+j', true)).toBe('ctrl+shift+j');
    expect(resolveLeader('alt', true)).toBe('alt');
    expect(resolveLeader('meta+space', true)).toBe('meta+space');
  });
  it('renders the substituted leader as a lone Alt in chord displays', () => {
    expect(formatBinding('prefix t', resolveLeader('ctrl+space', true))).toBe('Alt T');
  });
});

describe('chord tree', () => {
  const shortcuts = {
    'command-palette': 'ctrl+shift+p', // direct — must be ignored by the tree
    'new-terminal': 'prefix n t',
    'new-claude': 'prefix n c',
    'close-pane': 'prefix t w',
    'next-tab': 'prefix t ]',
    'toggle-help': 'prefix v',
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
    // groups (New, Tab) come first, then the leaf (toggle-help → v)
    expect(root.map((i) => i.label)).toEqual(['New', 'Tab', 'Toggle help']);
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

// jsdom is not macOS, so `mod` resolves to Ctrl in these tests.
const key = (over: Partial<KeyboardEvent>): KeyboardEvent =>
  ({
    key: 'a',
    code: '',
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...over,
  }) as KeyboardEvent;

describe('buildXtermAppKeyPredicate — the config-derived terminal boundary', () => {
  // A realistic slice of the default map: direct combos, chords, ranges,
  // scoped keys, an F-key, and a shift-only hypothetical rebind.
  const shortcuts: Record<string, string> = {
    'command-palette': 'mod+k',
    'toggle-sidebar': 'mod+b',
    'library-picker': 'mod+shift+l',
    'toggle-help': 'f1',
    'jump-tab': 'ctrl+1-9',
    'move-tab': 'ctrl+shift+1-9',
    'new-terminal': 'prefix t',
    'nav-left': 'prefix h',
    'fleet-open': 'enter', // scoped — matched only inside the deck
    'inbox-move-down': 'j', // scoped
    'custom-shift-only': 'shift+e', // shift alone is typing, never app nav
  };
  const appOwns = buildXtermAppKeyPredicate(shortcuts, 'ctrl+space');

  it('owns every bound direct combo, resolving the mod token', () => {
    expect(appOwns(key({ key: 'k', ctrlKey: true }))).toBe(true);
    expect(appOwns(key({ key: 'b', ctrlKey: true }))).toBe(true);
    expect(appOwns(key({ key: 'L', ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(appOwns(key({ key: 'F1' }))).toBe(true);
  });

  it('returns UNBOUND relic keys to the shell (the drifted-hardcoded-list fix)', () => {
    // The old hardcoded list blocked all of these from the PTY even though
    // nothing in the app binds them anymore — dead keys stealing readline's
    // transpose (Ctrl+T), delete-word (Ctrl+W) and EOF (Ctrl+D).
    expect(appOwns(key({ key: 't', ctrlKey: true }))).toBe(false);
    expect(appOwns(key({ key: 'w', ctrlKey: true }))).toBe(false);
    expect(appOwns(key({ key: 'd', ctrlKey: true }))).toBe(false);
    expect(appOwns(key({ key: '/', ctrlKey: true }))).toBe(false);
  });

  it('owns digit-range bindings via e.code', () => {
    expect(appOwns(key({ key: '3', code: 'Digit3', ctrlKey: true }))).toBe(true);
    // Bare digits are typing.
    expect(appOwns(key({ key: '3', code: 'Digit3' }))).toBe(false);
    // Numpad digits are typing too (ranges match Digit1-9 codes only).
    expect(appOwns(key({ key: '3', code: 'Numpad3', ctrlKey: true }))).toBe(false);
  });

  it('owns the resolved combo leader so a chord can arm from inside a terminal', () => {
    expect(appOwns(key({ key: ' ', code: 'Space', ctrlKey: true }))).toBe(true);
  });

  it('needs nothing for a lone-modifier leader (the Linux Alt tap)', () => {
    const altLeader = buildXtermAppKeyPredicate(shortcuts, 'alt');
    // A bare Alt keydown stays with xterm (it types nothing anyway); the tap
    // arms on the window's key-up, which xterm never consumes.
    expect(altLeader(key({ key: 'Alt', altKey: true }))).toBe(false);
  });

  it('keeps the shared structural rules: ctrl+shift wholesale, alt-arrows, ctrl+alt-arrows', () => {
    expect(appOwns(key({ key: 'Z', ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(appOwns(key({ key: 'ArrowLeft', altKey: true }))).toBe(true);
    expect(appOwns(key({ key: 'ArrowRight', ctrlKey: true, altKey: true }))).toBe(true);
  });

  it('never blocks typing: bare, shift-only, and surface-scoped keys stay with the terminal', () => {
    expect(appOwns(key({ key: 'j' }))).toBe(false); // scoped inbox key
    expect(appOwns(key({ key: 'Enter' }))).toBe(false); // scoped fleet key
    expect(appOwns(key({ key: 'E', shiftKey: true }))).toBe(false); // shift-only rebind
    expect(appOwns(key({ key: 'h' }))).toBe(false); // chord step, not direct
  });
});

describe('findChordConflicts / leaderPassthroughBytes', () => {
  it('flags duplicate leaf paths and prefix shadowing, case/modifier-aware', () => {
    const conflicts = findChordConflicts({
      a: 'prefix t',
      b: 'prefix t', // duplicate of a
      c: 'prefix g',
      d: 'prefix g g', // shadowed by c
      e: 'prefix shift+k', // distinct from f — shift is part of the step
      f: 'prefix k',
      g: 'ctrl+t', // direct combos are not chords
    });
    expect(conflicts).toEqual([
      { kind: 'duplicate', path: 't', actions: ['a', 'b'] },
      { kind: 'shadow', path: 'g', actions: ['c', 'd'] },
    ]);
  });

  it('derives the passthrough byte from the CONFIGURED leader', () => {
    expect(leaderPassthroughBytes('ctrl+space')).toBe('\x00');
    expect(leaderPassthroughBytes('ctrl+a')).toBe('\x01'); // tmux classic
    expect(leaderPassthroughBytes('ctrl+b')).toBe('\x02');
    expect(leaderPassthroughBytes('alt')).toBe(''); // no byte encoding — no-op
    expect(leaderPassthroughBytes('ctrl+shift+p')).toBe('\x10'); // shift is display-only here
    expect(leaderPassthroughBytes('meta+space')).toBe('');
  });
});
