/**
 * The BoardPane's correctness bar, tested as the two properties the feature
 * rests on rather than as a pile of behaviours:
 *
 *  1. ROUND-TRIP IS BYTE-EXACT. Parse a brief, serialize it, and the file must
 *     be identical — otherwise every archive move is a chance to silently
 *     rewrite the user's own words.
 *  2. A MOVE ONLY MOVES. After a column move or an archive, the multiset of
 *     lines is the input's minus/plus exactly the entry's own lines. Proved by
 *     reconstruction, not by spot-checking formatting.
 *
 * WHERE THE FIXTURES COME FROM. `.workspacer/` is gitignored — the real briefs
 * are the user's private working state and must not be committed into a public
 * repo — so the checked-in fixture below reproduces their SHAPE (emoji markers,
 * bolded headlines, backticked shas, `###` sub-sections, numbered entries, a
 * stray HTML comment, CRLF, no trailing newline) without their contents. The
 * real files are then exercised too, at their real paths, whenever they exist
 * on the machine running the tests; that leg self-skips in CI.
 */
import { describe, it, expect } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  parseBrief,
  serializeBrief,
  moveEntryToColumn,
  removeEntry,
  appendToArchive,
  cardsForBrief,
  cardsForArchive,
  deriveCard,
  cardFor,
  normalizeIndex,
  entryId,
  parseStatus,
  insertPointFor,
  BRIEF_STATUSES,
} from './briefBoard';

const FIXTURE = [
  '# Project — brief',
  '',
  '## Now',
  '- STALE — VERIFY BEFORE TRUSTING: a plain entry with no marker and no date, carrying `backticks` and a — dash.',
  '- ✅ **RESOLVED — fixed and merged as `a1412772`** on 2026-08-22, kept for the diagnosis. See `session:c03bd8ce`.',
  '- 🚧 2026-08-22 **SOMETHING DISPATCHED — the design above is now IN FLIGHT.** session:2375f443 builds the pane.',
  '- ⚠️ 2026-08-22 **ONE HUMAN ACTION NEEDED:** the server sits at ⏸ Pending approval in `~/.claude.json`.',
  '- 📋 2026-08-22 **DESIGNED WITH THE USER, NOT YET DISPATCHED: a thing.** Origin: the user wanted X.',
  '',
  '### A sub-heading inside Now (2026-08-21). Do not re-derive.',
  '- **Wakes are parent-keyed.** Worker-finished wakes route ONLY to a live parent.',
  '- Correction to an earlier note: this is now FIXED.',
  '',
  '### Numbered gaps',
  '1. **Structured worker results.** A worker outcome arrives as PROSE.',
  '2. **A brief primitive.** Every brief update today is Read + Edit + hope.',
  '',
  '## Direction',
  '- A durable goal that spans many sessions.',
  '- Delivery mode: "local" — ship workers merge into the main checkout.',
  '',
  '## Recently',
  '- 2026-08-22: Something landed and was PUSHED (`e6df15be..39e673c2`).',
  '- 2026-08-22: Something else was merged `1c2754d1` (session:78d737c6, sonnet).',
  '',
  '<!-- Older entries moved to brief.archive.md on 2026-08-21 -->',
  '',
  '## User',
  '- Prefers fewer blocking questions.',
  '',
].join('\n');

/** The real briefs on this machine, when they are there. */
function realBriefPaths(): string[] {
  const home = os.homedir();
  const candidates = [
    path.join(home, '.workspacer', 'brief.md'),
    path.join(home, '.workspacer', 'brief.archive.md'),
    path.join(home, 'Work', 'worky', 'workspacer', '.workspacer', 'brief.md'),
    path.join(home, 'Work', 'worky', 'workspacer', '.workspacer', 'brief.archive.md'),
    path.join(home, 'Work', 'preheat', '.workspacer', 'brief.md'),
    // The worktree this may be running in.
    path.resolve(__dirname, '../../../../..', '.workspacer', 'brief.md'),
  ];
  return candidates.filter((p) => {
    try {
      return fs.statSync(p).isFile();
    } catch {
      return false;
    }
  });
}

describe('parseBrief / serializeBrief — byte-exact round trip', () => {
  const cases: Array<[string, string]> = [
    ['the fixture', FIXTURE],
    ['empty', ''],
    ['no trailing newline', '## Now\n- one\n- two'],
    ['CRLF', FIXTURE.replace(/\n/g, '\r\n')],
    ['trailing blank lines', '## Now\n- one\n\n\n\n'],
    ['no headings at all', 'just some prose\nand more of it\n'],
    ['heading with no body', '# T\n\n## Now\n\n## Direction\n- x\n'],
    ['leading blank lines', '\n\n\n# T\n## Now\n- x\n'],
    ['tabs and trailing spaces', '## Now\n-   spaced   \n\t- indented tab bullet\n'],
    ['deep headings', '## Now\n- a\n### s1\n- b\n#### s2\n- c\n## Direction\n- d\n'],
  ];

  for (const [name, content] of cases) {
    it(`round-trips ${name}`, () => {
      expect(serializeBrief(parseBrief(content))).toBe(content);
    });
  }

  it('round-trips the REAL briefs on this machine (skipped where they are absent)', () => {
    const paths = realBriefPaths();
    if (paths.length === 0) {
      // CI, or a fresh checkout. The fixture cases above still hold the bar.
      expect(paths).toEqual([]);
      return;
    }
    for (const p of paths) {
      const content = fs.readFileSync(p, 'utf-8');
      expect(serializeBrief(parseBrief(content)), `round trip failed for ${p}`).toBe(content);
    }
  });

  it('accounts for every line exactly once — entries never overlap', () => {
    const doc = parseBrief(FIXTURE);
    let last = -1;
    for (const e of doc.entries) {
      expect(e.start).toBeGreaterThan(last);
      expect(e.end).toBeGreaterThan(e.start);
      last = e.end - 1;
    }
  });

  it('finds every top-level entry, including numbered ones, and none of the headings', () => {
    const doc = parseBrief(FIXTURE);
    const texts = doc.entries.map((e) => e.text);
    expect(texts).toHaveLength(14);
    expect(texts.some((t) => t.startsWith('1. **Structured'))).toBe(true);
    expect(texts.some((t) => t.startsWith('#'))).toBe(false);
    // A `###` entry belongs to its parent `##` column but remembers its group.
    const nested = doc.entries.find((e) => e.text.startsWith('- **Wakes are parent-keyed'));
    expect(nested?.column).toBe('Now');
    expect(nested?.group).toContain('A sub-heading inside Now');
  });

  it('does not swallow the blank separator lines between entries', () => {
    const doc = parseBrief(FIXTURE);
    for (const e of doc.entries) expect(e.lines.some((l) => l.trim() === '')).toBe(false);
  });
});

describe('entryId', () => {
  it('is stable, and different for different text', () => {
    expect(entryId('- hello')).toBe(entryId('- hello'));
    expect(entryId('- hello')).not.toBe(entryId('- hellp'));
  });
  it('distinguishes entries that differ only by emoji', () => {
    expect(entryId('- ✅ done')).not.toBe(entryId('- 🚧 done'));
  });
  it('is 16 hex chars', () => {
    expect(entryId('- x')).toMatch(/^[0-9a-f]{16}$/);
  });
  it('gives every entry in the real fixture a distinct id', () => {
    const ids = parseBrief(FIXTURE).entries.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe('moveEntryToColumn — a move only moves', () => {
  const idOf = (content: string, needle: string): string => {
    const e = parseBrief(content).entries.find((x) => x.text.includes(needle));
    if (!e) throw new Error(`no entry containing ${needle}`);
    return e.id;
  };

  it('moves an entry between columns without altering a single character of it', () => {
    const id = idOf(FIXTURE, 'RESOLVED — fixed and merged');
    const before = parseBrief(FIXTURE).entries.find((e) => e.id === id)!;
    const after = moveEntryToColumn(FIXTURE, id, 'Recently');
    const moved = parseBrief(after).entries.find((e) => e.id === id);
    expect(moved).toBeDefined();
    expect(moved!.text).toBe(before.text);
    expect(moved!.column).toBe('Recently');
  });

  it('leaves the rest of the file byte-identical (reconstruction proof)', () => {
    const id = idOf(FIXTURE, 'RESOLVED — fixed and merged');
    const entry = parseBrief(FIXTURE).entries.find((e) => e.id === id)!;
    const after = moveEntryToColumn(FIXTURE, id, 'Recently');
    // Delete the entry's lines from BOTH files: what remains must be identical.
    const strip = (content: string): string => {
      const doc = parseBrief(content);
      const e = doc.entries.find((x) => x.id === id)!;
      const lines = doc.lines.slice();
      lines.splice(e.start, e.end - e.start);
      return lines.join('\n');
    };
    expect(strip(after)).toBe(strip(FIXTURE));
    // And nothing but that entry's lines changed count.
    expect(after.split('\n')).toHaveLength(FIXTURE.split('\n').length);
    expect(entry.lines).toHaveLength(1);
  });

  it('prepends into Recently (newest first) and appends into the others', () => {
    const id = idOf(FIXTURE, 'DESIGNED WITH THE USER');
    const toRecently = parseBrief(moveEntryToColumn(FIXTURE, id, 'Recently'));
    const recently = toRecently.entries.filter((e) => e.column === 'Recently');
    expect(recently[0].id).toBe(id);

    const toDirection = parseBrief(moveEntryToColumn(FIXTURE, id, 'Direction'));
    const direction = toDirection.entries.filter((e) => e.column === 'Direction');
    expect(direction[direction.length - 1].id).toBe(id);
  });

  it('does not move an entry past a ### sub-heading when appending', () => {
    const id = idOf(FIXTURE, 'A durable goal');
    const after = moveEntryToColumn(FIXTURE, id, 'Now');
    const doc = parseBrief(after);
    const moved = doc.entries.find((e) => e.id === id)!;
    // It lands in the top-level part of `## Now`, above the first `###`.
    expect(moved.group).toBeUndefined();
    const firstSub = doc.sections.find((s) => s.level === 3)!;
    expect(moved.start).toBeLessThan(firstSub.headingLine);
  });

  it('is a no-op for a card dropped on the column it came from', () => {
    const id = idOf(FIXTURE, 'A durable goal');
    expect(moveEntryToColumn(FIXTURE, id, 'Direction')).toBe(FIXTURE);
  });

  it('promotes an entry out of a ### sub-heading when moved to its own column', () => {
    const id = idOf(FIXTURE, 'Wakes are parent-keyed');
    const after = moveEntryToColumn(FIXTURE, id, 'Now');
    const moved = parseBrief(after).entries.find((e) => e.id === id)!;
    expect(moved.group).toBeUndefined();
    expect(moved.column).toBe('Now');
  });

  it('refuses an unknown entry rather than writing something', () => {
    expect(() => moveEntryToColumn(FIXTURE, 'deadbeefdeadbeef', 'Now')).toThrow(/no entry/);
  });

  it('refuses a column the brief does not have', () => {
    const id = idOf(FIXTURE, 'A durable goal');
    expect(() => moveEntryToColumn(FIXTURE, id, 'Nowhere')).toThrow(/no "## Nowhere"/);
  });

  it('survives a brief with no trailing newline', () => {
    const src = '## Now\n- one\n\n## Direction\n- two';
    const id = parseBrief(src).entries.find((e) => e.text === '- two')!.id;
    const after = moveEntryToColumn(src, id, 'Now');
    expect(after).toBe('## Now\n- one\n- two\n\n## Direction');
  });

  it('round-trips every entry through every column and back, losing nothing', () => {
    for (const start of parseBrief(FIXTURE).entries) {
      let content = FIXTURE;
      for (const col of ['Recently', 'Direction', 'Now'] as const) {
        content = moveEntryToColumn(content, start.id, col);
      }
      const lines = (s: string): string[] => s.split('\n').slice().sort();
      expect(lines(content), `lost a line moving ${start.text.slice(0, 40)}`).toEqual(
        lines(FIXTURE),
      );
    }
  });
});

describe('archive', () => {
  it('removes the entry from the brief and appends it verbatim to the archive', () => {
    const entry = parseBrief(FIXTURE).entries.find((e) => e.text.includes('RESOLVED'))!;
    const { content, entry: removed } = removeEntry(FIXTURE, entry.id);
    expect(content).not.toContain(entry.text);
    expect(removed.text).toBe(entry.text);

    const archive = appendToArchive('', removed.lines, '2026-08-22');
    expect(archive).toContain('# Brief archive');
    expect(archive).toContain('## 2026-08-22');
    expect(archive).toContain(entry.text);
    expect(serializeBrief(parseBrief(archive))).toBe(archive);
  });

  it('appends into an existing batch heading without touching a line already there', () => {
    const existing = '# Brief archive\n\nCold storage.\n\n## 2026-08-22\n- older one\n';
    const next = appendToArchive(existing, ['- newer one'], '2026-08-22');
    expect(next).toBe(
      '# Brief archive\n\nCold storage.\n\n## 2026-08-22\n- older one\n- newer one\n',
    );
  });

  it('opens a new batch heading at the BOTTOM for a new date', () => {
    const existing = '# Brief archive\n\n## 2026-08-21\n- older\n';
    const next = appendToArchive(existing, ['- newer'], '2026-08-22');
    expect(next.indexOf('## 2026-08-21')).toBeLessThan(next.indexOf('## 2026-08-22'));
    expect(next).toContain('- older');
    // Append-only: every original line is still present, in order.
    expect(next.startsWith(existing.replace(/\n$/, ''))).toBe(true);
  });

  it('never reorders an existing archive (append-only, proved by prefix)', () => {
    const existing = '# Brief archive\n\n## 2026-08-21\n- a\n- b\n\n## 2026-08-22\n- c\n';
    const next = appendToArchive(existing, ['- d'], '2026-08-21');
    const origLines = existing.split('\n').filter((l) => l.trim());
    const nextLines = next.split('\n').filter((l) => l.trim());
    // Original lines survive in their original relative order.
    let cursor = 0;
    for (const l of origLines) {
      const at = nextLines.indexOf(l, cursor);
      expect(at, `archive reordered around ${l}`).toBeGreaterThanOrEqual(cursor);
      cursor = at + 1;
    }
  });
});

describe('deriveCard — the degraded path never produces a blank card', () => {
  const cardFrom = (line: string, column: 'Now' | 'Direction' | 'Recently' = 'Now') => {
    const doc = parseBrief(`## ${column}\n${line}\n`);
    return deriveCard(doc.entries[0], column);
  };

  it('prefers a short bolded headline', () => {
    const c = cardFrom('- ✅ 2026-08-22 **The approval hang** — three defects in the hook router.');
    expect(c.title).toBe('The approval hang');
    expect(c.date).toBe('2026-08-22');
    expect(c.marker).toBe('✅');
    expect(c.status).toBe('landed');
    expect(c.summary).toContain('three defects');
  });

  it('NEVER titles a retracted entry with the claim it retracts', () => {
    // The shape that motivated the rule: the only bold span IS the debunked
    // claim, so a naive bold-span rule headlines the card with a falsehood.
    const c = cardFrom(
      '- ❌ WRONG, RETRACTED SAME DAY — **the nudge fires on every turn, not just on finish** was never true; the latch is per-session.',
    );
    expect(c.retracted).toBe(true);
    expect(c.title).not.toContain('the nudge fires on every turn');
    expect(c.title.toUpperCase()).toContain('WRONG');
  });

  it('catches a worded retraction with no glyph, and a glyph with no wording', () => {
    expect(cardFrom('- SUPERSEDED by the entry below — **a bold claim** here.').retracted).toBe(
      true,
    );
    expect(cardFrom('- ❌ **a bold claim** that did not hold up.').retracted).toBe(true);
    expect(cardFrom('- ✅ **a bold claim** that did hold up.').retracted).toBeUndefined();
  });

  it('cuts at the first em-dash when there is no usable bold span', () => {
    const c = cardFrom(
      '- The hub can be hot-swapped in a dev build — kill it and let Electron respawn it.',
    );
    expect(c.title).toBe('The hub can be hot-swapped in a dev build');
  });

  it('cuts at the first sentence end when there is no em-dash', () => {
    const c = cardFrom(
      '- The hub can be hot-swapped in a dev build. Kill it and let Electron respawn.',
    );
    expect(c.title).toBe('The hub can be hot-swapped in a dev build.');
  });

  it('falls back to a truncated first line when there is no break at all', () => {
    const long = `- ${'x'.repeat(300)}`;
    const c = cardFrom(long);
    expect(c.title.length).toBeLessThanOrEqual(97);
    expect(c.title.endsWith('…')).toBe(true);
  });

  it('ignores a bold span too short to be a headline', () => {
    const c = cardFrom('- **NEW** candidate fixes were found by the PWA worker but not applied.');
    expect(c.title).toContain('candidate fixes');
  });

  it('ignores a bold span so long it is a bolded paragraph, not a headline', () => {
    const c = cardFrom(`- **${'word '.repeat(40)}** trailing detail.`);
    expect(c.title.startsWith('word word')).toBe(true);
    expect(c.title.length).toBeLessThanOrEqual(97);
  });

  it('gives an awkward but present title to an entry bundling unrelated topics', () => {
    // A real shape: three unrelated fixes in one bullet. No title represents
    // it; the rule must not contort, and must not drop the entry.
    const c = cardFrom(
      '- Chores: ✅ one thing done and ✅ another deleted, both today. STILL OPEN: a third.',
    );
    expect(c.title.trim()).not.toBe('');
  });

  it('always yields a non-empty title, for every entry in the fixture and the real briefs', () => {
    const contents = [FIXTURE, ...realBriefPaths().map((p) => fs.readFileSync(p, 'utf-8'))];
    for (const content of contents) {
      const { cards, extras } = cardsForBrief(content);
      for (const c of [...cards, ...extras]) {
        expect(c.title.trim(), `blank title for: ${c.text.slice(0, 60)}`).not.toBe('');
        if (c.status !== undefined) expect(BRIEF_STATUSES).toContain(c.status);
      }
    }
  });

  it('shows EVERY entry — the card count equals the entry count', () => {
    const doc = parseBrief(FIXTURE);
    const { cards, extras } = cardsForBrief(FIXTURE);
    expect(cards.length + extras.length).toBe(doc.entries.length);
  });

  it('puts entries from a column-less section (## User) in extras rather than dropping them', () => {
    const { extras } = cardsForBrief(FIXTURE);
    expect(extras.map((e) => e.text)).toContain('- Prefers fewer blocking questions.');
  });

  it('reads status from the author’s own glyph first, then from keywords', () => {
    expect(cardFrom('- ⚠️ something').status).toBe('waiting-on-you');
    expect(cardFrom('- 🚧 something').status).toBe('in-flight');
    expect(cardFrom('- this was merged yesterday', 'Direction').status).toBe('landed');
    expect(cardFrom('- this one is NOT YET DISPATCHED', 'Direction').status).toBe('next-up');
  });

  it('NEVER derives status from which section the entry sits in', () => {
    // The stale-but-unpruned `## Now` entry is the case that matters: six in a
    // sample of sixty-five real ones. Calling those "in flight" because of
    // where they sit would relabel the rot the board exists to surface.
    const line = '- a plain line that says nothing about its own state';
    for (const col of ['Now', 'Direction', 'Recently'] as const) {
      expect(cardFrom(line, col).status).toBeUndefined();
    }
    // And a resolved entry still sitting in `## Now` reads as landed, not live.
    expect(cardFrom('- ✅ RESOLVED, merged last week and still listed here.', 'Now').status).toBe(
      'landed',
    );
  });

  it('does not sweep un-landed entries into "Next up" on weak evidence', () => {
    // `/standup` generates "Next up"; brief entries rarely assert it. Only
    // explicit backlog language may claim it.
    expect(cardFrom('- a design that was proposed and considered').status).toBeUndefined();
    expect(cardFrom('- deferred for now, no owner').status).toBeUndefined();
    expect(cardFrom('- NEXT UP: wire the reporter').status).toBe('next-up');
  });

  it('does not read a topic-kind glyph (📋 ⭐ 🐛) as a state', () => {
    expect(cardFrom('- 📋 a design note with no state words').status).toBeUndefined();
    expect(cardFrom('- 🐛 an open bug with no state words').status).toBeUndefined();
  });

  it('lets a blocking entry outrank a landed one', () => {
    const c = cardFrom('- RESOLVED and merged, but ONE HUMAN ACTION NEEDED to finish it.');
    expect(c.status).toBe('waiting-on-you');
  });

  it('extracts session ids and backticked shas as refs', () => {
    const c = cardFrom('- ✅ fixed in `a1412772` and `f0700b38` (session:c03bd8ce).');
    expect(c.refs).toContain('a1412772');
    expect(c.refs).toContain('session:c03bd8ce');
  });
});

describe('the synthesis seam', () => {
  const doc = parseBrief(FIXTURE);
  const entry = doc.entries[0];

  it('uses the sidecar index when it has a row', () => {
    const index = normalizeIndex({
      cards: {
        [entry.id]: { title: 'A synthesized title', status: 'waiting on you', summary: 'Short.' },
      },
    });
    const c = cardFor(entry, 'Now', index);
    expect(c.title).toBe('A synthesized title');
    expect(c.status).toBe('waiting-on-you');
    expect(c.synthesized).toBe(true);
  });

  it('degrades per-entry when the index is absent, empty, or missing that row', () => {
    const derived = deriveCard(entry, 'Now');
    for (const index of [undefined, {}, normalizeIndex({ cards: { other: { title: 'x' } } })]) {
      const c = cardFor(entry, 'Now', index);
      expect(c.title).toBe(derived.title);
      expect(c.synthesized).toBe(false);
    }
  });

  it('accepts a bare map as well as a wrapped one', () => {
    const bare = normalizeIndex({ [entry.id]: { title: 'Bare' } });
    expect(cardFor(entry, 'Now', bare).title).toBe('Bare');
  });

  it('ignores a status the enum does not have, rather than showing it', () => {
    const index = normalizeIndex({ cards: { [entry.id]: { status: 'on fire' } } });
    expect(cardFor(entry, 'Now', index).status).toBe(deriveCard(entry, 'Now').status);
  });

  it('survives a malformed index file', () => {
    for (const raw of [null, 'nope', 42, [], { cards: 'no' }, { cards: { x: null } }]) {
      expect(() => normalizeIndex(raw)).not.toThrow();
    }
    expect(normalizeIndex(null)).toEqual({});
  });

  it('parses the standup labels as well as the slugs', () => {
    expect(parseStatus('waiting on you')).toBe('waiting-on-you');
    expect(parseStatus('IN_FLIGHT')).toBe('in-flight');
    expect(parseStatus('nonsense')).toBeUndefined();
  });
});

describe('cardsForArchive', () => {
  it('renders archived entries read-only, grouped by their batch heading', () => {
    const archive = '# Brief archive\n\n## 2026-08-21\n- 2026-08-20: a thing happened.\n';
    const cards = cardsForArchive(archive);
    expect(cards).toHaveLength(1);
    expect(cards[0].archived).toBe(true);
    expect(cards[0].column).toBe('archive');
    expect(cards[0].group).toBe('2026-08-21');
  });
});

describe('insertPointFor', () => {
  it('returns -1 for a column the brief does not have', () => {
    expect(insertPointFor(parseBrief('## Now\n- a\n'), 'Recently')).toBe(-1);
  });
  it('skips the author’s blank line under a prepending heading', () => {
    const doc = parseBrief('## Recently\n\n- newest\n');
    expect(doc.lines[insertPointFor(doc, 'Recently')]).toBe('- newest');
  });
});

/**
 * THE REAL FILES, MOVED FOR REAL. The fixture above pins the rules; this pins
 * them against the actual briefs on this machine — long, emoji-laden, full of
 * nested markdown and numbered sub-lists. Every entry is archived, and moved to
 * every column, and the result is checked line for line. Self-skips where the
 * files are absent (CI, a fresh checkout).
 *
 * This is the test that says the archive drag cannot corrupt a brief. If it
 * ever goes red, the feature is unsafe to ship, not merely buggy.
 */
describe('real briefs survive real moves', () => {
  const realBriefs = (): string[] => realBriefPaths().filter((p) => p.endsWith('brief.md'));

  it('archives EVERY entry of every real brief, losing not one byte', () => {
    for (const p of realBriefs()) {
      const orig = fs.readFileSync(p, 'utf-8');
      for (const entry of parseBrief(orig).entries) {
        const { content, entry: removed } = removeEntry(orig, entry.id);
        expect(removed.text).toBe(entry.text);
        // The brief, minus exactly this entry's lines and nothing else.
        const expected = orig.split('\n');
        expected.splice(entry.start, entry.end - entry.start);
        expect(content, `${p}: archiving disturbed another line`).toBe(expected.join('\n'));
        expect(appendToArchive('', removed.lines, '2026-08-22')).toContain(entry.text);
      }
    }
  });

  it('moves EVERY entry to every column without losing a line', () => {
    const sorted = (s: string): string[] => s.split('\n').slice().sort();
    for (const p of realBriefs()) {
      const orig = fs.readFileSync(p, 'utf-8');
      for (const entry of parseBrief(orig).entries) {
        for (const col of ['Now', 'Direction', 'Recently']) {
          const next = moveEntryToColumn(orig, entry.id, col);
          expect(sorted(next), `${p}: lost a line moving ${entry.text.slice(0, 50)}`).toEqual(
            sorted(orig),
          );
          const moved = parseBrief(next).entries.find((e) => e.id === entry.id);
          expect(moved, `${p}: entry vanished moving to ${col}`).toBeDefined();
          expect(moved!.text).toBe(entry.text);
        }
      }
    }
  });
});
