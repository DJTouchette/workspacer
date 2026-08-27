/**
 * The TypeScript half of contracts/brief-board-cases.json.
 *
 * The Go half is services/hub/cmd/brain/briefboard_test.go. Both providers
 * answer `brief.archive` and `brief.check` now — this copy under a desktop
 * client, the brain's for every headless/MCP caller — so a Fleet Manager must
 * not get a different answer about its own brief depending on which one ran.
 *
 * What actually drifts is not the verb but the ENTRY BOUNDARY underneath it:
 * which lines belong to a bullet, which heading closes a block, and which END of
 * a section holds its oldest entries. This file runs the desktop's real
 * `parseBrief`, its real `archiveOldestEntries` (against a real filesystem, the
 * same as briefBoardService.test.ts) and its real `checkNowSection` over the
 * shared corpus.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';

// briefBoardService reaches for config and session history to build its lane
// list. `archiveOldestEntries` takes a directory and does not, but the imports
// run at module load, so they are stubbed the way briefBoardService.test.ts
// stubs them — these cases describe a fixed brief, not this machine.
import { vi } from 'vitest';
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ projects: {} }) },
}));
vi.mock('./sessionHistory', () => ({
  sessionHistory: { recent: () => [] },
}));

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import { parseBrief } from '../shared/briefBoard';
import { archiveOldestEntries } from './briefBoardService';
import { checkNowSection, liveSessionIds } from './briefCheck';

interface EntryCase {
  name: string;
  brief: string;
  expect: Array<{
    column: string;
    group?: string;
    start: number;
    end: number;
    lines: string[];
  }>;
  why: string;
}

interface ArchiveCase {
  name: string;
  brief: string;
  archive?: string;
  section: string;
  count?: number;
  keep?: number;
  date: string;
  expect: {
    brief: string;
    archive: string;
    archived: number;
    entriesInSection: number;
    bytesInSection: number;
    bytesInBrief: number;
  };
  why: string;
}

interface CheckCase {
  name: string;
  brief: string;
  sessions: unknown[];
  expect: {
    entriesChecked: number;
    entriesLive: number;
    liveSessions: number;
    findings: Array<{ line: number; text: string; reason: string; refs: string[] }>;
  };
  why: string;
}

interface BriefBoardFixture {
  owners: Record<string, string>;
  entries: EntryCase[];
  archive: ArchiveCase[];
  check: CheckCase[];
}

/** This side's keys in the fixture's `owners` map. Renaming one of these files
 *  without updating the fixture must FAIL, not silently stop testing anything. */
const OWNER_KEYS = [
  'apps/desktop/src/main/shared/briefBoard.ts',
  'apps/desktop/src/main/services/briefBoardService.ts',
  'apps/desktop/src/main/services/briefCheck.ts',
];

// apps/desktop/src/main/services/ → five levels below the repo root.
const fixture: BriefBoardFixture = JSON.parse(
  fs.readFileSync(path.join(__dirname, '../../../../../contracts/brief-board-cases.json'), 'utf-8'),
);

describe('brief board contract — the fixture itself', () => {
  it('loads and names this side as an owner of all three copies', () => {
    for (const key of OWNER_KEYS) {
      expect(fixture.owners[key], `the fixture must name ${key}`).toBeDefined();
    }
    expect(fixture.entries.length).toBeGreaterThan(0);
    expect(fixture.archive.length).toBeGreaterThan(0);
    expect(fixture.check.length).toBeGreaterThan(0);
  });
});

describe('brief document model — cross-language contract', () => {
  const tally = new SweepTally();
  for (const c of fixture.entries) {
    it(c.name, () => {
      tally.ran('other');
      const doc = parseBrief(c.brief);
      expect(
        doc.entries.map((e) => ({
          column: e.column,
          ...(e.group ? { group: e.group } : {}),
          start: e.start,
          end: e.end,
          lines: e.lines,
        })),
        c.why,
      ).toEqual(c.expect);
      // The round-trip property, on every case: an archive move splices against
      // these indexes, so a parse that lost or reflowed a line would corrupt the
      // user's own document.
      expect(doc.lines.join('\n')).toBe(c.brief);
    });
  }
  itSweptTheWholeCorpus(tally, 'the brief entry corpus', 4, { allow: 0, deny: 0 });
});

describe('brief archive — cross-language contract', () => {
  const tally = new SweepTally();
  let dir: string;
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'brief-board-contract-'));
    fs.mkdirSync(path.join(dir, '.workspacer'), { recursive: true });
  });
  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const briefPath = (): string => path.join(dir, '.workspacer', 'brief.md');
  const archivePath = (): string => path.join(dir, '.workspacer', 'brief.archive.md');

  for (const c of fixture.archive) {
    it(c.name, () => {
      tally.ran('other');
      fs.writeFileSync(briefPath(), c.brief);
      if (c.archive) fs.writeFileSync(archivePath(), c.archive);

      const res = archiveOldestEntries({
        dir,
        section: c.section,
        count: c.count,
        keep: c.keep,
        date: c.date,
      });

      expect(fs.readFileSync(briefPath(), 'utf-8'), c.why).toBe(c.expect.brief);
      // '' means the file must not exist: a move that wrote nothing must not
      // leave an empty archive beside the brief either.
      const archive = fs.existsSync(archivePath()) ? fs.readFileSync(archivePath(), 'utf-8') : '';
      expect(archive, c.why).toBe(c.expect.archive);
      expect(
        {
          archived: res.archived,
          entriesInSection: res.entriesInSection,
          bytesInSection: res.bytesInSection,
          bytesInBrief: res.bytesInBrief,
        },
        c.why,
      ).toEqual({
        archived: c.expect.archived,
        entriesInSection: c.expect.entriesInSection,
        bytesInSection: c.expect.bytesInSection,
        bytesInBrief: c.expect.bytesInBrief,
      });
      expect({ section: res.section, date: res.date, path: res.path }).toEqual({
        section: c.section,
        date: c.date,
        path: briefPath(),
      });
    });
  }
  itSweptTheWholeCorpus(tally, 'the brief archive corpus', 5, { allow: 0, deny: 0 });
});

describe('brief check — cross-language contract', () => {
  const tally = new SweepTally();
  const BRIEF_PATH = '/p/.workspacer/brief.md';
  for (const c of fixture.check) {
    it(c.name, () => {
      tally.ran('other');
      const report = checkNowSection(c.brief, liveSessionIds(c.sessions), BRIEF_PATH);
      expect(
        {
          entriesChecked: report.entriesChecked,
          entriesLive: report.entriesLive,
          liveSessions: report.liveSessions,
        },
        c.why,
      ).toEqual({
        entriesChecked: c.expect.entriesChecked,
        entriesLive: c.expect.entriesLive,
        liveSessions: c.expect.liveSessions,
      });
      // `detail` is deliberately not in the fixture — it is a sentence for a
      // model, and the Go side pins the wording against this file's source
      // directly (TestBriefCheckWordingMatchesTheDesktop). What IS pinned is
      // that every finding carries one.
      expect(
        report.findings.map((f) => ({
          line: f.line,
          text: f.text,
          reason: f.reason,
          refs: f.refs,
        })),
        c.why,
      ).toEqual(c.expect.findings);
      for (const f of report.findings) expect(f.detail.trim()).not.toBe('');
      expect(report.section).toBe('Now');
      expect(report.path).toBe(BRIEF_PATH);
    });
  }
  itSweptTheWholeCorpus(tally, 'the brief check corpus', 6, { allow: 0, deny: 0 });
});
