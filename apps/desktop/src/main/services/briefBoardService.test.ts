/**
 * The board's writes, against a real filesystem.
 *
 * The bar is the one briefService set and this file has to meet, because it
 * writes the SAME files agents write with `brief_append`: a move must not
 * clobber a concurrent writer, and it must not corrupt the brief when it loses
 * the race. The compare-and-swap is only observable by changing the file
 * BETWEEN the service's two reads, so `fs.readFileSync` is mocked with a
 * pass-through a test can temporarily divert — the same technique, and the same
 * reason, as briefService.test.ts.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

const readHook = vi.hoisted(() => ({
  fn: undefined as undefined | ((p: string, data: string) => void),
}));
vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  const readFileSync = ((p: never, enc: never) => {
    const data = actual.readFileSync(p, enc);
    if (readHook.fn && typeof p === 'string' && typeof data === 'string') readHook.fn(p, data);
    return data;
  }) as typeof actual.readFileSync;
  return { ...actual, default: { ...actual, readFileSync }, readFileSync };
});

const projects = vi.hoisted(() => ({ value: {} as Record<string, { label?: string }> }));
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ projects: projects.value }) },
}));

// The lane list also consults session history for directories the fleet has
// worked in. Mocked so these tests describe a fixed fleet rather than whatever
// this machine happens to have run.
const history = vi.hoisted(() => ({ rows: [] as Array<{ cwd: string }> }));
vi.mock('./sessionHistory', () => ({
  sessionHistory: { recent: () => history.rows },
}));

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { applyBoardMove, loadBoard, boardLaneTargets, todayStamp } from './briefBoardService';
import { parseBrief } from '../shared/briefBoard';
import { appendBriefLine } from './briefService';

const BRIEF = [
  '# Demo — project brief',
  '',
  '## Now',
  '- ✅ **A resolved thing** that should have been pruned days ago.',
  '- 🚧 **A live thing** that is genuinely in flight.',
  '',
  '## Direction',
  '- A durable goal.',
  '',
  '## Recently',
  '- 2026-08-22: the newest log line.',
  '',
].join('\n');

let root: string;
let projectDir: string;

const briefPath = (): string => path.join(projectDir, '.workspacer', 'brief.md');
const archivePath = (): string => path.join(projectDir, '.workspacer', 'brief.archive.md');
const read = (p: string): string => fs.readFileSync(p, 'utf-8');

const idOf = (needle: string): string => {
  const e = parseBrief(read(briefPath())).entries.find((x) => x.text.includes(needle));
  if (!e) throw new Error(`no entry containing ${needle}`);
  return e.id;
};

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-board-'));
  projectDir = path.join(root, 'demo');
  fs.mkdirSync(path.join(projectDir, '.workspacer'), { recursive: true });
  fs.writeFileSync(briefPath(), BRIEF);
  projects.value = { [projectDir]: { label: 'Demo' } };
  history.rows = [];
  readHook.fn = undefined;
});

afterEach(() => {
  readHook.fn = undefined;
  fs.rmSync(root, { recursive: true, force: true });
});

describe('boardLaneTargets', () => {
  it('puts the fleet lane first, then every project in config', () => {
    const lanes = boardLaneTargets();
    expect(lanes[0].kind).toBe('fleet');
    expect(lanes[0].dir).toBe(os.homedir());
    expect(lanes.slice(1).map((l) => l.dir)).toContain(projectDir);
    expect(lanes.find((l) => l.dir === projectDir)?.label).toBe('Demo');
  });

  it('finds a project with a brief that config forgot — projects can be empty', () => {
    // config.projects is empty on this machine right now (a known get_config
    // regression). A board that showed one lane in that state would look
    // broken rather than honest, so a directory the fleet has worked in AND
    // already has a brief in still earns a lane.
    projects.value = {};
    history.rows = [{ cwd: projectDir }];
    const lanes = boardLaneTargets();
    expect(lanes.map((l) => l.dir)).toContain(projectDir);
    expect(lanes.find((l) => l.dir === projectDir)?.label).toBe('demo');
  });

  it('does NOT list a worked-in directory with no brief — the board is not a directory list', () => {
    projects.value = {};
    const scratch = path.join(root, 'scratch');
    fs.mkdirSync(scratch, { recursive: true });
    history.rows = [{ cwd: scratch }];
    expect(boardLaneTargets().map((l) => l.dir)).not.toContain(scratch);
  });

  it('lists a REGISTERED project even with no brief, so the invitation can show', () => {
    const bare = path.join(root, 'bare');
    fs.mkdirSync(bare, { recursive: true });
    projects.value = { [bare]: { label: 'Bare' } };
    expect(boardLaneTargets().map((l) => l.dir)).toContain(bare);
  });

  it('does not list the same directory twice when a project IS the fleet root', () => {
    projects.value = { [os.homedir()]: {} };
    const lanes = boardLaneTargets();
    expect(lanes.filter((l) => path.resolve(l.dir) === path.resolve(os.homedir()))).toHaveLength(1);
  });
});

describe('loadBoard', () => {
  it('reads a lane into cards and marks it as having a brief', () => {
    const lane = loadBoard().lanes.find((l) => l.dir === projectDir)!;
    expect(lane.exists).toBe(true);
    expect(lane.indexed).toBe(false);
    expect(lane.cards).toHaveLength(4);
    expect(lane.cards.map((c) => c.title)).toContain('A live thing');
  });

  it('shows a lane with NO brief rather than hiding the project', () => {
    const bare = path.join(root, 'bare');
    fs.mkdirSync(bare, { recursive: true });
    projects.value = { [bare]: { label: 'Bare' } };
    const lane = loadBoard().lanes.find((l) => l.dir === bare)!;
    expect(lane).toBeDefined();
    expect(lane.exists).toBe(false);
    expect(lane.error).toBeUndefined();
    expect(lane.cards).toEqual([]);
  });

  it('uses brief.index.json when it is there, and degrades when it is junk', () => {
    const id = idOf('A live thing');
    const indexPath = path.join(projectDir, '.workspacer', 'brief.index.json');
    fs.writeFileSync(indexPath, JSON.stringify({ cards: { [id]: { title: 'Synthesized' } } }));
    let lane = loadBoard().lanes.find((l) => l.dir === projectDir)!;
    expect(lane.indexed).toBe(true);
    expect(lane.cards.find((c) => c.id === id)?.title).toBe('Synthesized');

    fs.writeFileSync(indexPath, '{ not json');
    lane = loadBoard().lanes.find((l) => l.dir === projectDir)!;
    expect(lane.indexed).toBe(false);
    expect(lane.cards.find((c) => c.id === id)?.title).toBe('A live thing');
  });

  it('folds archived entries into the lane as read-only Archive cards', () => {
    fs.writeFileSync(
      archivePath(),
      '# Brief archive\n\n## 2026-08-21\n- 2026-08-20: an old one.\n',
    );
    const lane = loadBoard().lanes.find((l) => l.dir === projectDir)!;
    const archived = lane.cards.filter((c) => c.archived);
    expect(archived).toHaveLength(1);
    expect(archived[0].column).toBe('archive');
  });
});

describe('applyBoardMove — column moves', () => {
  it('moves the entry and leaves its text byte-identical', () => {
    const id = idOf('A resolved thing');
    const text = parseBrief(read(briefPath())).entries.find((e) => e.id === id)!.text;
    applyBoardMove({ key: projectDir, entryId: id, to: 'Recently' });
    const after = parseBrief(read(briefPath())).entries.find((e) => e.id === id)!;
    expect(after.text).toBe(text);
    expect(after.column).toBe('Recently');
  });

  it('is a real write — the file on disk changed', () => {
    const before = read(briefPath());
    applyBoardMove({ key: projectDir, entryId: idOf('A resolved thing'), to: 'Recently' });
    expect(read(briefPath())).not.toBe(before);
  });

  it('returns the freshly reloaded lane, so the pane never renders stale cards', () => {
    const id = idOf('A resolved thing');
    const lane = applyBoardMove({ key: projectDir, entryId: id, to: 'Direction' });
    expect(lane.cards.find((c) => c.id === id)?.column).toBe('Direction');
  });

  it('refuses a lane key that is not one of this fleet’s projects', () => {
    expect(() =>
      applyBoardMove({ key: '/etc', entryId: idOf('A live thing'), to: 'Recently' }),
    ).toThrow(/not one of this fleet/);
    expect(read(briefPath())).toBe(BRIEF);
  });

  it('refuses an unknown column without writing anything', () => {
    expect(() =>
      applyBoardMove({
        key: projectDir,
        entryId: idOf('A live thing'),
        to: 'Elsewhere' as never,
      }),
    ).toThrow(/unknown column/);
    expect(read(briefPath())).toBe(BRIEF);
  });

  it('refuses an entry that is no longer there, without writing anything', () => {
    expect(() =>
      applyBoardMove({ key: projectDir, entryId: 'ffffffffffffffff', to: 'Recently' }),
    ).toThrow(/no entry/);
    expect(read(briefPath())).toBe(BRIEF);
  });
});

describe('applyBoardMove — archive', () => {
  it('moves the entry OUT of the brief and INTO brief.archive.md, verbatim', () => {
    const id = idOf('A resolved thing');
    const text = parseBrief(read(briefPath())).entries.find((e) => e.id === id)!.text;

    applyBoardMove({ key: projectDir, entryId: id, to: 'archive' });

    expect(read(briefPath())).not.toContain(text);
    const archive = read(archivePath());
    expect(archive).toContain(text);
    expect(archive).toContain(`## ${todayStamp()}`);
    expect(archive).toContain('# Brief archive');
  });

  it('leaves every other line of the brief byte-identical', () => {
    const id = idOf('A resolved thing');
    const entry = parseBrief(BRIEF).entries.find((e) => e.id === id)!;
    applyBoardMove({ key: projectDir, entryId: id, to: 'archive' });
    const expected = BRIEF.split('\n');
    expected.splice(entry.start, entry.end - entry.start);
    expect(read(briefPath())).toBe(expected.join('\n'));
  });

  it('appends into an existing archive without rewriting a line of it', () => {
    const existing = '# Brief archive\n\n## 2026-08-01\n- 2026-07-31: something ancient.\n';
    fs.writeFileSync(archivePath(), existing);
    applyBoardMove({ key: projectDir, entryId: idOf('A resolved thing'), to: 'archive' });
    const after = read(archivePath());
    expect(after.startsWith(existing.replace(/\n$/, ''))).toBe(true);
    expect(after).toContain('- 2026-07-31: something ancient.');
  });

  it('archives two entries in a row into the same batch heading', () => {
    applyBoardMove({ key: projectDir, entryId: idOf('A resolved thing'), to: 'archive' });
    applyBoardMove({ key: projectDir, entryId: idOf('A live thing'), to: 'archive' });
    const archive = read(archivePath());
    expect(archive.match(new RegExp(`## ${todayStamp()}`, 'g'))).toHaveLength(1);
    expect(archive).toContain('A resolved thing');
    expect(archive).toContain('A live thing');
  });

  it('never leaves the archive holding an entry the brief still has', () => {
    const id = idOf('A resolved thing');
    applyBoardMove({ key: projectDir, entryId: id, to: 'archive' });
    const brief = read(briefPath());
    const archived = parseBrief(read(archivePath())).entries.map((e) => e.id);
    for (const a of archived) {
      expect(parseBrief(brief).entries.map((e) => e.id)).not.toContain(a);
    }
  });
});

/**
 * TWO KINDS OF CONCURRENT WRITER, and they are defended differently.
 *
 *  - One that TAKES THE LOCK — `brief_append`, i.e. every agent's brief write.
 *    It is serialized outright: both writes happen, one after the other. That
 *    is proved below by showing the two share a lock, since calling
 *    `appendBriefLine` from inside the board's own lock would (correctly)
 *    deadlock rather than interleave.
 *  - One that does NOT — an agent's Edit tool, the user's editor. Nothing can
 *    serialize those, so the compare-and-swap catches them: the board notices
 *    the file changed under it and recomputes against the new content instead
 *    of writing over it. That is what the diverted read simulates.
 */
describe('concurrent writers', () => {
  /** An outside writer that does not honour our lock, landing between the
   *  service's read and its re-read. */
  const interposeOnce = (mutate: (content: string) => string): void => {
    let fired = false;
    readHook.fn = (p) => {
      if (fired || !p.endsWith('brief.md')) return;
      fired = true;
      readHook.fn = undefined;
      fs.writeFileSync(briefPath(), mutate(read(briefPath())));
    };
  };

  it('shares its lock with brief_append, so agent writes are serialized not raced', () => {
    // Both take `<briefPath>.lock`. Observable without contention: hold the
    // lock by hand and watch brief_append refuse to write through it.
    const lockPath = `${briefPath()}.lock`;
    fs.writeFileSync(lockPath, 'held', { flag: 'wx' });
    try {
      expect(() =>
        applyBoardMove({ key: projectDir, entryId: idOf('A live thing'), to: 'Recently' }),
      ).toThrow(/locked by another writer/);
      expect(() => appendBriefLine(briefPath(), 'Now', 'blocked too')).toThrow(
        /locked by another writer/,
      );
      expect(read(briefPath())).toBe(BRIEF);
    } finally {
      fs.rmSync(lockPath, { force: true });
    }
    // Two 3s lock waits, taken back to back on purpose.
  }, 15_000);

  it('lands an agent’s brief_append line and the user’s drag, both, when they interleave', () => {
    const id = idOf('A resolved thing');
    // The agent's line arrives while the drag is being computed.
    interposeOnce((content) =>
      content.replace('## Direction', '- a line an agent landed mid-drag\n\n## Direction'),
    );

    applyBoardMove({ key: projectDir, entryId: id, to: 'Recently' });

    const after = read(briefPath());
    // BOTH survived: the agent's line and the user's move.
    expect(after).toContain('a line an agent landed mid-drag');
    const moved = parseBrief(after).entries.find((e) => e.id === id)!;
    expect(moved.column).toBe('Recently');
  });

  it('does not archive an entry twice when the compare-and-swap retries', () => {
    const id = idOf('A resolved thing');
    interposeOnce((content) =>
      content.replace('- A durable goal.', '- A durable goal.\n- another agent line'),
    );

    applyBoardMove({ key: projectDir, entryId: id, to: 'archive' });

    const archive = read(archivePath());
    const occurrences = parseBrief(archive).entries.filter((e) => e.id === id);
    expect(occurrences).toHaveLength(1);
    expect(read(briefPath())).toContain('another agent line');
    expect(read(briefPath())).not.toContain('A resolved thing');
  });

  it('gives up rather than writing when it can never win the race', () => {
    const id = idOf('A resolved thing');
    // An outside writer that lands on EVERY read: the CAS can never pass.
    let n = 0;
    readHook.fn = (p) => {
      if (!p.endsWith('brief.md')) return;
      const saved = readHook.fn;
      readHook.fn = undefined;
      fs.writeFileSync(briefPath(), `${read(briefPath())}\n- churn ${n++}`);
      readHook.fn = saved;
    };
    expect(() => applyBoardMove({ key: projectDir, entryId: id, to: 'Recently' })).toThrow(
      /rewritten faster/,
    );
    readHook.fn = undefined;
    // The entry is still where it was — nothing was half-written.
    expect(read(briefPath())).toContain('A resolved thing');
  });
});
