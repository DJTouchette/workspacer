/**
 * The BoardPane's main-process half: reads every brief the fleet has, and
 * performs the board's only two writes — move an entry between the brief's `##`
 * sections, and archive an entry out into `brief.archive.md`.
 *
 * WHY THE WRITES LIVE HERE AND NOT IN THE RENDERER. Agents call `brief_append`
 * on these same files while the board is open — that is not a theoretical race,
 * it is the normal case, because a worker finishing is what makes the manager
 * write a brief line AND what makes the user open the board. So the board's
 * writes take the SAME advisory lock and do the SAME compare-and-swap as
 * briefService (see its header for the full argument): read inside the lock,
 * re-read immediately before publishing, retry against a writer that beat us
 * rather than over it.
 *
 * ARCHIVE ORDER IS DELIBERATE. The archive file is appended FIRST, then the
 * entry is removed from the brief. A crash between the two duplicates the entry
 * (visible, and a second archive drag fixes it); the other order would lose it.
 * The archive is cold storage — append-only by the /checkpoint doctrine — so a
 * duplicate there is a nuisance and never a corruption.
 *
 * PATH CONFINEMENT. `move`/`archive` never take a path from the renderer. They
 * take a lane KEY, which must resolve against the lanes this module itself
 * computed from config; anything else is refused. The renderer cannot name a
 * file for main to write.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { withFileLock } from '../lib/fileLock';
import { configService } from './configService';
import { sessionHistory } from './sessionHistory';
import {
  BOARD_COLUMNS,
  BriefColumnMissing,
  appendToArchive,
  cardsForArchive,
  cardsForBrief,
  isPrependSection,
  moveEntryToColumn,
  normalizeIndex,
  parseBrief,
  removeEntry,
  removeEntryLines,
  type BoardColumn,
  type BriefCard,
} from '../shared/briefBoard';
import { briefSectionStats, type BriefSizeReport } from './briefService';

const BRIEF_DIR = '.workspacer';
const BRIEF_FILE = 'brief.md';
const ARCHIVE_FILE = 'brief.archive.md';
/** The sidecar a synthesis layer may write. Absent today, and the board is
 *  built to not care. */
const INDEX_FILE = 'brief.index.json';

/** Same budgets as briefService: this is the same file with the same writers,
 *  and two different wait policies on one lock is a bug waiting to happen. */
const LOCK_WAIT_MS = 3_000;
const LOCK_STALE_MS = 15_000;
const CAS_ATTEMPTS = 5;

export class BriefBoardLockTimeout extends Error {
  constructor(p: string) {
    super(`brief.md is locked by another writer (waited ${LOCK_WAIT_MS}ms): ${p}`);
    this.name = 'BriefBoardLockTimeout';
  }
}

/** One swimlane: a project's brief, or the manager's own fleet brief. */
export interface BoardLane {
  /** Stable key = the lane's directory. Also what `move`/`archive` take. */
  key: string;
  dir: string;
  label: string;
  kind: 'fleet' | 'project';
  briefPath: string;
  archivePath: string;
  /** False when the project has no brief yet — the lane still shows, with an
   *  invitation, because a missing brief and an unreadable one are different
   *  answers and must not read the same. */
  exists: boolean;
  error?: string;
  cards: BriefCard[];
  /** Entries under a section the board has no column for (`## User`). Shown in
   *  a lane footer: "show everything" has no discard path. */
  extras: BriefCard[];
  /** True when a `brief.index.json` was found and parsed. */
  indexed: boolean;
}

export interface BoardData {
  lanes: BoardLane[];
  columns: readonly BoardColumn[];
}

function readOrNull(p: string): string | null {
  try {
    return fs.readFileSync(p, 'utf-8');
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null;
    throw err;
  }
}

/** The sidecar, when it is there and parseable. A malformed one degrades the
 *  lane to derived cards rather than failing it. */
function readIndex(dir: string): { index: ReturnType<typeof normalizeIndex>; found: boolean } {
  const raw = readOrNull(path.join(dir, BRIEF_DIR, INDEX_FILE));
  if (raw == null) return { index: {}, found: false };
  try {
    return { index: normalizeIndex(JSON.parse(raw)), found: true };
  } catch {
    return { index: {}, found: false };
  }
}

/** How far back to look for directories the fleet has actually worked in. */
const HISTORY_LOOKBACK = 300;

/**
 * Every lane the board shows: the fleet brief first, then every project.
 *
 * WHY THIS IS A UNION AND NOT JUST `config.projects`. The registry is the
 * primary source and a registered project always gets a lane, brief or not, so
 * the "no brief here yet" invitation can appear. But `projects` can be empty
 * while the fleet is plainly working in several repos — it is empty on this
 * machine right now, from a known `get_config` regression — and a board that
 * showed one lane in that state would look broken rather than honest. So the
 * same config-derived directories `renderer/lib/projectRegistry` counts as
 * projects are included, plus the working directories of past sessions, which
 * is how the mobile PWA finds briefs.
 *
 * Discovered directories must ALREADY HAVE a brief to earn a lane. Registered
 * ones do not. Without that asymmetry the board would fill with every scratch
 * directory an agent ever ran in.
 */
export function boardLaneTargets(): Array<{
  dir: string;
  label: string;
  kind: 'fleet' | 'project';
}> {
  const out: Array<{ dir: string; label: string; kind: 'fleet' | 'project' }> = [
    { dir: os.homedir(), label: 'Fleet', kind: 'fleet' },
  ];
  const seen = new Set([path.resolve(os.homedir())]);
  const add = (dir: string, label?: string): void => {
    const resolved = path.resolve(dir);
    if (!resolved || seen.has(resolved)) return;
    seen.add(resolved);
    out.push({
      dir: resolved,
      label: label || path.basename(resolved) || resolved,
      kind: 'project',
    });
  };

  const config = configService.getConfig();
  const projects = config.projects ?? {};
  for (const [dir, identity] of Object.entries(projects)) add(dir, identity?.label);
  // The rest of what projectRegistry treats as a known project: a directory you
  // favourited, opened, or configured a script or widget board for.
  for (const dir of config.directories?.favourites ?? []) add(dir);
  for (const dir of config.directories?.recent ?? []) add(dir);
  for (const dir of Object.keys(config.scripts ?? {})) add(dir);
  for (const dir of Object.keys(config.widgets ?? {})) add(dir);

  // Directories the fleet has actually worked in, but only where a brief is
  // already written.
  for (const dir of recentWorkingDirs()) {
    if (seen.has(path.resolve(dir))) continue;
    try {
      if (fs.statSync(briefPathFor(dir)).isFile()) add(dir);
    } catch {
      /* no brief there — not a project for this board's purposes */
    }
  }

  return out;
}

/** Working directories of recent sessions, newest first. Reads the local
 *  history store (synchronous SQLite), not the daemon — the board must not need
 *  a running claudemon to list projects. */
function recentWorkingDirs(): string[] {
  try {
    const dirs: string[] = [];
    for (const rec of sessionHistory.recent(HISTORY_LOOKBACK)) {
      const cwd = String(rec.cwd ?? '').replace(/[/\\]+$/, '');
      if (cwd && !dirs.includes(cwd)) dirs.push(cwd);
    }
    return dirs;
  } catch {
    return [];
  }
}

function briefPathFor(dir: string): string {
  return path.join(dir, BRIEF_DIR, BRIEF_FILE);
}
function archivePathFor(dir: string): string {
  return path.join(dir, BRIEF_DIR, ARCHIVE_FILE);
}

function loadLane(target: { dir: string; label: string; kind: 'fleet' | 'project' }): BoardLane {
  const briefPath = briefPathFor(target.dir);
  const archivePath = archivePathFor(target.dir);
  const base: BoardLane = {
    key: target.dir,
    dir: target.dir,
    label: target.label,
    kind: target.kind,
    briefPath,
    archivePath,
    exists: false,
    cards: [],
    extras: [],
    indexed: false,
  };

  let content: string | null;
  try {
    content = readOrNull(briefPath);
  } catch (err) {
    return { ...base, error: (err as Error).message };
  }
  if (content == null) return base;

  const { index, found } = readIndex(target.dir);
  const { cards, extras } = cardsForBrief(content, index);
  let archived: BriefCard[] = [];
  try {
    const archive = readOrNull(archivePath);
    if (archive != null) archived = cardsForArchive(archive, index);
  } catch {
    // An unreadable archive costs the Archive column its contents, not the lane.
  }

  return { ...base, exists: true, indexed: found, cards: [...cards, ...archived], extras };
}

/** Read the whole board. Synchronous like the rest of main's file services;
 *  these are a handful of small files and the pane loads once per open. */
export function loadBoard(): BoardData {
  return { lanes: boardLaneTargets().map(loadLane), columns: BOARD_COLUMNS };
}

/** Resolve a renderer-supplied lane key to a target this module vouches for.
 *  A key that is not in the computed set is refused — the renderer does not get
 *  to name a file for main to write. */
function resolveLane(key: string): { dir: string; label: string; kind: 'fleet' | 'project' } {
  const hit = boardLaneTargets().find(
    (t) => t.dir === key || path.resolve(t.dir) === path.resolve(key),
  );
  if (!hit) throw new Error(`brief board: "${key}" is not one of this fleet's projects`);
  return hit;
}

/** Local YYYY-MM-DD. The archive's batch headings are the user's dates, and a
 *  UTC one would file an evening move under tomorrow. */
export function todayStamp(now: Date = new Date()): string {
  const p = (n: number): string => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** What one attempt computed: the brief's new content, plus anything else that
 *  must hit disk with it. `before` runs only once the compare-and-swap has
 *  passed — a side effect performed inside the compute would be repeated by
 *  every retry, which for the archive means a duplicated entry per attempt. */
interface BriefMutation {
  next: string;
  beforeWrite?: () => void;
}

/**
 * Run `mutate` against the brief under the lock, with a compare-and-swap that
 * retries against an outside writer instead of clobbering it. Exactly the
 * discipline briefService established.
 */
function withBrief(briefPath: string, mutate: (content: string) => BriefMutation): void {
  withFileLock(
    briefPath,
    {
      staleMs: LOCK_STALE_MS,
      maxWaitMs: LOCK_WAIT_MS,
      onTimeout: (p) => new BriefBoardLockTimeout(p),
    },
    () => {
      for (let attempt = 0; attempt < CAS_ATTEMPTS; attempt++) {
        const before = readOrNull(briefPath);
        if (before == null) throw new Error(`brief board: no brief at ${briefPath}`);
        const { next, beforeWrite } = mutate(before);
        // Re-read immediately before publishing: an agent's Edit tool and the
        // user's editor do not take this lock, so "nothing changed while I was
        // computing" is a claim to check, not to assume.
        if (readOrNull(briefPath) !== before) continue;
        if (next === before) return;
        beforeWrite?.();
        atomicWriteFileSync(briefPath, next);
        return;
      }
      throw new Error(
        `brief board: ${briefPath} is being rewritten faster than this move could land ` +
          `(${CAS_ATTEMPTS} attempts). Nothing was written — reload the board and try again.`,
      );
    },
  );
}

export interface BoardMoveRequest {
  /** Lane key, i.e. the project directory. */
  key: string;
  /** `entryId` of the card being dragged. */
  entryId: string;
  to: BoardColumn;
}

/**
 * The drag. Moving to `Now`/`Direction`/`Recently` relocates the entry's lines
 * between two headings; moving to `archive` takes them out of the brief and
 * appends them to `brief.archive.md`. Nothing else mutates a brief from here.
 */
export function applyBoardMove(req: BoardMoveRequest): BoardLane {
  const target = resolveLane(req.key);
  if (!(BOARD_COLUMNS as readonly string[]).includes(req.to)) {
    throw new Error(`brief board: unknown column ${JSON.stringify(req.to)}`);
  }
  const briefPath = briefPathFor(target.dir);

  if (req.to === 'archive') {
    const archivePath = archivePathFor(target.dir);
    const date = todayStamp();
    withBrief(briefPath, (content) => {
      const { content: pruned, entry } = removeEntry(content, req.entryId);
      return {
        next: pruned,
        // ARCHIVE FIRST, and only once the CAS has passed. See the header: a
        // crash between the two writes must duplicate the entry, never lose it.
        // (The archive itself is not separately locked — it is only ever
        // appended to, and this lane's brief lock serializes every board write
        // that touches it.)
        beforeWrite: () =>
          atomicWriteFileSync(
            archivePath,
            appendToArchive(readOrNull(archivePath) ?? '', entry.lines, date),
          ),
      };
    });
  } else {
    withBrief(briefPath, (content) => ({
      next: moveEntryToColumn(content, req.entryId, req.to),
    }));
  }

  return loadLane(target);
}

export interface BriefArchiveRequest {
  /** The project directory. Already confined by the caller: this module composes
   *  both filenames itself, exactly as brief.append does. */
  dir: string;
  /** Which `##` heading to trim. */
  section: string;
  /** Archive this many of the section's OLDEST entries. */
  count?: number;
  /** Or: leave this many of its newest and archive everything past them. */
  keep?: number;
  /** The archive's batch heading. Defaults to today, and exists for the tests. */
  date?: string;
}

export interface BriefArchiveResult extends BriefSizeReport {
  path: string;
  archivePath: string;
  section: string;
  /** How many entries actually moved. */
  archived: number;
  /** The `## <date>` heading they landed under. */
  date: string;
}

/**
 * Move the OLDEST entries of one section out to `brief.archive.md`.
 *
 * WHY THIS EXISTS AS A CAPABILITY. The board could already do this move, but
 * only one card at a time and only by dragging, so /checkpoint improvised the
 * same thing in shell: copy the brief, cut the overflow out with sed, hand-write
 * a batch heading. One morning of that left three differently worded headings in
 * the archive and four .bak files beside the brief. Nothing about the move was
 * missing, only a way to call it.
 *
 * WHICH END IS THE OLDEST. `## Recently` is kept newest-first, so its oldest
 * entries are its LAST; every other section is written in arrival order, so its
 * oldest are its FIRST. That is `isPrependSection`, the same rule brief_append
 * inserts by, read from the one place it is written down.
 *
 * The move itself is `removeEntryLines` + `appendToArchive`, unchanged: whole
 * lines are spliced out of the brief and appended to the archive verbatim, in
 * the order they sat in, so an archived entry is byte-identical to the entry
 * that left. Nothing here rewrites a line, and the whole pass runs under the
 * brief's lock with the same compare-and-swap every other write here takes.
 */
export function archiveOldestEntries(req: BriefArchiveRequest): BriefArchiveResult {
  const { dir, section } = req;
  const briefPath = briefPathFor(dir);
  const archivePath = archivePathFor(dir);
  const date = req.date ?? todayStamp();
  const wanted = section.trim().toLowerCase();

  // Exactly one of the two, because they answer different questions and a
  // caller that gave both has not decided which. `keep` is the idempotent form
  // (run it twice, the second run moves nothing), which is why /checkpoint uses
  // it; `count` is for a caller that has already counted.
  const hasCount = typeof req.count === 'number';
  const hasKeep = typeof req.keep === 'number';
  if (hasCount === hasKeep) {
    throw new Error(
      'brief.archive: give either count (archive this many of the oldest) or keep ' +
        '(leave this many of the newest), and not both',
    );
  }
  const bound = hasCount ? req.count! : req.keep!;
  if (!Number.isInteger(bound) || bound < 0) {
    throw new Error(
      `brief.archive: ${hasCount ? 'count' : 'keep'} must be a whole number, 0 or more`,
    );
  }

  let archived = 0;
  withBrief(briefPath, (content) => {
    const doc = parseBrief(content);
    const hasSection = doc.sections.some((s) => s.level === 2 && s.title.toLowerCase() === wanted);
    if (!hasSection) throw new BriefColumnMissing(section);

    const inSection = doc.entries.filter((e) => e.column.toLowerCase() === wanted);
    const oldestFirst = isPrependSection(section) ? [...inSection].reverse() : inSection;
    const want = hasCount ? req.count! : Math.max(0, inSection.length - req.keep!);
    const chosen = new Set(
      oldestFirst.slice(0, Math.min(want, inSection.length)).map((e) => e.start),
    );
    // Back in document order, so the archive reads the way the section did.
    const moving = inSection.filter((e) => chosen.has(e.start));
    archived = moving.length;

    // Splice from the bottom up: every entry's line indexes point into the same
    // array, and removing an earlier one would shift the rest.
    let lines = doc.lines;
    for (const entry of [...moving].reverse()) lines = removeEntryLines(lines, entry);

    return {
      next: lines.join('\n'),
      // ARCHIVE FIRST, and only once the compare-and-swap has passed, for the
      // reason in this file's header: a crash between the two writes must
      // duplicate an entry, never lose it. The archive is read HERE rather than
      // outside, so a retry composes against what is on disk now.
      beforeWrite: () =>
        atomicWriteFileSync(
          archivePath,
          moving.reduce(
            (acc, entry) => appendToArchive(acc, entry.lines, date),
            readOrNull(archivePath) ?? '',
          ),
        ),
    };
  });

  return {
    path: briefPath,
    archivePath,
    section,
    archived,
    date,
    ...briefSectionStats(readOrNull(briefPath) ?? '', section),
  };
}
