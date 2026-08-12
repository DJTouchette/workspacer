/**
 * The editor plugin's file tree — reveal behaviour.
 *
 * The plugin (services/hub/examples/editor) is a single HTML file with inline
 * JS and no build step, so it had no test of any kind. It is also the app's
 * DEFAULT editor, reached from three places that do not go through its tree:
 * the `?file=` param (right-click → Open in editor), a search hit, and a
 * newly-created file. All three used to load the file while the sidebar stayed
 * wherever it was.
 *
 * The functions are lifted out of the HTML by their section markers and run
 * against jsdom with a fake project. It is a regex over source, which is
 * fragile on purpose: if the markers move, this fails loudly rather than
 * quietly testing nothing. `fs.listEntries` is the only backend call the tree
 * makes, so the stub is one function.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import path from 'path';

const HTML = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../../../../services/hub/examples/editor/ui/index.html',
);

/**
 * A fake project with both shapes that matter: a branching path (`src`, which
 * must render one row per level) and a single-child chain (`chain/a/b`, which
 * must compress into one row).
 */
const FS: Record<string, Array<{ name: string; path: string; isDir: boolean }>> = {
  '/repo': [
    { name: 'chain', path: '/repo/chain', isDir: true },
    { name: 'src', path: '/repo/src', isDir: true },
    { name: 'README.md', path: '/repo/README.md', isDir: false },
  ],
  '/repo/chain': [{ name: 'a', path: '/repo/chain/a', isDir: true }],
  '/repo/chain/a': [{ name: 'b', path: '/repo/chain/a/b', isDir: true }],
  '/repo/chain/a/b': [{ name: 'leaf.ts', path: '/repo/chain/a/b/leaf.ts', isDir: false }],
  '/repo/src': [
    { name: 'deep', path: '/repo/src/deep', isDir: true },
    { name: 'index.ts', path: '/repo/src/index.ts', isDir: false },
  ],
  '/repo/src/deep': [{ name: 'buried.ts', path: '/repo/src/deep/buried.ts', isDir: false }],
};

interface TreeApi {
  renderTree: () => Promise<void>;
  revealInTree: (p: string) => Promise<void>;
  markActiveRow: (p: string) => unknown;
  isWithin: (root: string, p: string) => boolean;
  ancestorDirs: (root: string, p: string) => string[];
  baseOf: (p: string) => string;
  treeEl: HTMLElement;
  setCurrent: (c: { path: string; name: string } | null) => void;
  listCalls: () => number;
  scrolled: string[];
}

function section(src: string, re: RegExp, what: string): string {
  const m = src.match(re);
  if (!m) {
    throw new Error(
      `could not lift the ${what} out of the editor plugin — did its section markers change? (${HTML})`,
    );
  }
  return m[0];
}

function loadTree(): TreeApi {
  const src = readFileSync(HTML, 'utf8');
  const paths = section(
    src,
    /\/\/ ── paths ──[\s\S]*?\n {4}\/\*\* Every directory[\s\S]*?\n {4}}\n/,
    'path helpers',
  );
  const tree = section(
    src,
    /\/\/ ── file tree \(lazy\)[\s\S]*?(?=\n {4}\/\/ ── open \/ save \/ watch)/,
    'file-tree block',
  );

  document.body.innerHTML = '<div id="tree"></div>';
  const scrolled: string[] = [];
  // jsdom has no layout, so scrollIntoView does not exist; record it instead.
  (Element.prototype as unknown as { scrollIntoView: () => void }).scrollIntoView = function (
    this: HTMLElement,
  ) {
    scrolled.push(this.dataset.path ?? '');
  };
  let listCalls = 0;

  const prelude = `
    const treeEl = document.getElementById('tree');
    const expanded = new Set();
    let current = null;
    const escapeHtml = (s) => String(s);
    const svg = () => '<svg></svg>';
    const iconForFile = () => ['file', '#fff'];
    const setStatus = () => {};
    const openFile = () => {};
    const call = async (m, p) => { onList(); return { entries: FS[p.path] || [] }; };
    const cwd = '/repo';
  `;
  const api = `
    ; return { renderTree, revealInTree, markActiveRow, isWithin, ancestorDirs, baseOf, treeEl,
                setCurrent: (c) => { current = c; } };`;

  const make = new Function('document', 'FS', 'onList', prelude + paths + tree + api) as (
    d: Document,
    fs: typeof FS,
    onList: () => void,
  ) => Omit<TreeApi, 'listCalls' | 'scrolled'>;

  return {
    ...make(document, FS, () => listCalls++),
    listCalls: () => listCalls,
    scrolled,
  };
}

let t: TreeApi;
const rows = () =>
  [...t.treeEl.querySelectorAll('.row')].map((r) => (r as HTMLElement).dataset.path);
const labels = () =>
  [...t.treeEl.querySelectorAll('.row')].map((r) => r.querySelector('.lbl')?.textContent);
const active = () =>
  [...t.treeEl.querySelectorAll('.row.active')].map((r) => (r as HTMLElement).dataset.path);

beforeEach(() => {
  t = loadTree();
});

describe('editor plugin – path helpers', () => {
  // cwd and file arrive from the host as native paths; on Windows that is
  // backslashes, where splitting on '/' returns the whole path as the name.
  it('reads a name and a parent under either separator', () => {
    expect(t.baseOf('/repo/src/a/b.ts')).toBe('b.ts');
    expect(t.baseOf('C:\\repo\\src\\a\\b.ts')).toBe('b.ts');
  });

  it('compares containment on a separator boundary, not as a string prefix', () => {
    expect(t.isWithin('/repo', '/repo/src/a.ts')).toBe(true);
    expect(t.isWithin('/repo/', '/repo/src/a.ts')).toBe(true);
    expect(t.isWithin('/repo', '/repo')).toBe(true);
    // The one that a plain startsWith gets wrong.
    expect(t.isWithin('/repo', '/repo-old/src/a.ts')).toBe(false);
    expect(t.isWithin('C:\\repo', 'C:\\repo-old\\a.ts')).toBe(false);
    expect(t.isWithin('/repo', '/etc/passwd')).toBe(false);
  });

  // Stripping the trailing separator empties a filesystem root, and treating
  // that as "no root" disables every reveal under it.
  it('still contains things when the root IS the separator', () => {
    expect(t.isWithin('/', '/x/y.ts')).toBe(true);
    expect(t.ancestorDirs('/', '/x/y.ts')).toEqual(['/x']);
  });

  it('walks ancestors shallowest-first and stops at the root', () => {
    expect(t.ancestorDirs('/repo', '/repo/src/a/b.ts')).toEqual(['/repo/src', '/repo/src/a']);
    expect(t.ancestorDirs('/repo', '/repo/b.ts')).toEqual([]);
    expect(t.ancestorDirs('C:\\repo', 'C:\\repo\\src\\a\\b.ts')).toEqual([
      'C:\\repo\\src',
      'C:\\repo\\src\\a',
    ]);
    expect(t.ancestorDirs('/repo', '/etc/passwd')).toEqual([]);
  });
});

// Single-child chains collapse into one row, the same rule the review pane's
// tree uses. A chain is invisible from outside, so this costs a listing per
// directory shown — which is what the per-render cache is for.
describe('editor plugin – folder chain compression', () => {
  it('renders a single-child chain as one row, addressed by its deepest dir', async () => {
    await t.renderTree();
    expect(labels()).toEqual(['chain/a/b', 'src', 'README.md']);
    // The row's identity is the deepest directory — the one whose children it
    // expands to, and the one revealInTree puts in `expanded` on its way down.
    expect(rows()).toEqual(['/repo/chain/a/b', '/repo/src', '/repo/README.md']);
  });

  it('does not compress a directory that branches', async () => {
    await t.renderTree();
    await t.revealInTree('/repo/src/deep/buried.ts');
    // `src` holds a dir AND a file, so it stays its own row.
    expect(labels()).toContain('src');
    expect(labels()).toContain('deep');
  });

  it('lists each directory once per render, not once per probe', async () => {
    const before = t.listCalls();
    await t.renderTree();
    const listed = t.listCalls() - before;
    // /repo + the three chain levels + /repo/src. compressChain probes the same
    // directories renderLevel then renders; without the cache these double.
    expect(listed).toBe(5);
  });
});

describe('editor plugin – revealing an externally-opened file', () => {
  it('starts collapsed, showing only the top level', async () => {
    await t.renderTree();
    expect(rows()).toEqual(['/repo/chain/a/b', '/repo/src', '/repo/README.md']);
    expect(active()).toEqual([]);
  });

  // The reported bug: the file loaded, the sidebar did not move.
  it('expands every ancestor, highlights the file, and scrolls to it', async () => {
    await t.renderTree();
    t.setCurrent({ path: '/repo/src/deep/buried.ts', name: 'buried.ts' });
    await t.revealInTree('/repo/src/deep/buried.ts');

    expect(rows()).toEqual([
      '/repo/chain/a/b',
      '/repo/src',
      '/repo/src/deep',
      '/repo/src/deep/buried.ts',
      '/repo/src/index.ts',
      '/repo/README.md',
    ]);
    expect(active()).toEqual(['/repo/src/deep/buried.ts']);
    expect(t.scrolled).toEqual(['/repo/src/deep/buried.ts']);
  });

  // Compression and reveal have to agree on what a directory row IS. They meet
  // here: ancestorDirs walks every level, the chain row answers to the deepest.
  it('reveals a file that lives inside a compressed chain', async () => {
    await t.renderTree();
    t.setCurrent({ path: '/repo/chain/a/b/leaf.ts', name: 'leaf.ts' });
    await t.revealInTree('/repo/chain/a/b/leaf.ts');

    expect(rows()).toEqual([
      '/repo/chain/a/b',
      '/repo/chain/a/b/leaf.ts',
      '/repo/src',
      '/repo/README.md',
    ]);
    expect(active()).toEqual(['/repo/chain/a/b/leaf.ts']);
  });

  // Refresh and the fs.changed redraw both rebuild from scratch, which destroys
  // every row element and the highlight with it.
  it('keeps the open file marked across a re-render', async () => {
    await t.renderTree();
    t.setCurrent({ path: '/repo/src/deep/buried.ts', name: 'buried.ts' });
    await t.revealInTree('/repo/src/deep/buried.ts');

    await t.renderTree();
    expect(active()).toEqual(['/repo/src/deep/buried.ts']);
  });

  it('does not rebuild the tree when the file is already visible', async () => {
    await t.renderTree();
    t.setCurrent({ path: '/repo/src/deep/buried.ts', name: 'buried.ts' });
    await t.revealInTree('/repo/src/deep/buried.ts');

    const before = t.listCalls();
    await t.revealInTree('/repo/src/index.ts');
    expect(t.listCalls()).toBe(before); // no fs.listEntries round trip
    expect(active()).toEqual(['/repo/src/index.ts']);
  });

  // markActiveRow clears every row it does not match, so an unguarded reveal of
  // a path outside cwd deselects the file that is actually open.
  it('leaves the tree and the highlight alone for a path outside cwd', async () => {
    await t.renderTree();
    t.setCurrent({ path: '/repo/src/index.ts', name: 'index.ts' });
    await t.revealInTree('/repo/src/index.ts');
    const before = rows();

    await t.revealInTree('/etc/passwd');
    expect(rows()).toEqual(before);
    expect(active()).toEqual(['/repo/src/index.ts']);
  });
});
