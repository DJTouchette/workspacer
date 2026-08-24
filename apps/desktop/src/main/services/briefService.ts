/**
 * `brief_append` — the atomic inspect-then-edit primitive for a project's
 * `.workspacer/brief.md`.
 *
 * Every brief update a Fleet Manager makes was Read + Edit + hope: read the
 * file through one tool call, write it back through another, with an unbounded
 * window in between. That window is REAL, not theoretical — a manager updates a
 * project brief at exactly the moment a worker in that same project is editing
 * it, because the trigger for both is the same worker finishing. Two writers,
 * one file, no arbitration.
 *
 * The guarantee this makes, precisely:
 *
 *  1. STRICTLY ADDITIVE. The output is the input with exactly ONE line
 *     inserted. Nothing is rewritten, reordered, reformatted or reflowed —
 *     `appendToBrief` is a pure function and a test asserts that removing the
 *     inserted line reproduces the input byte for byte. The user's own edits
 *     are authoritative and this cannot touch them.
 *  2. SERIALIZED against itself, in-process and cross-process: an O_EXCL
 *     advisory lock (lib/fileLock) spans read→compute→write, so N concurrent
 *     brief_append calls land N lines, never N−1.
 *  3. COMPARE-AND-SWAP against everyone else. The read that feeds the insert
 *     happens INSIDE the lock, immediately before the write. If the file
 *     changed since — an agent's Edit tool, the user's editor, neither of which
 *     honours our lock — the pass is retried against the new content rather
 *     than written over it.
 *
 * What it does NOT claim: an outside writer that renames its own file over ours
 * between our final read and our rename still wins, because no userspace
 * protocol can stop that without both sides participating. The window is the
 * few microseconds of a read+rename rather than the seconds of an agent's
 * think-and-edit, which is the difference between a bug you hit and one you
 * don't.
 */
import * as fs from 'fs';
import * as path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { withFileLock } from '../lib/fileLock';
import { isPrependSection, parseBrief } from '../shared/briefBoard';

/** The four headings a brief has. `User` is fleet-brief-only but is accepted on
 *  any brief: refusing it would make the fleet brief a special case at every
 *  call site, and an unknown heading is created rather than refused anyway. */
export const BRIEF_SECTIONS = ['Now', 'Direction', 'Recently', 'User'] as const;
export type BriefSection = (typeof BRIEF_SECTIONS)[number];

/** Where a project's brief lives, relative to the project directory. */
export const BRIEF_RELATIVE_PATH = path.join('.workspacer', 'brief.md');

/**
 * Cap on one appended line, and it REFUSES rather than truncates.
 *
 * It used to cut at 2000 characters, append an ellipsis and report success. In
 * this repo's own brief that had silently guillotined 13 of 45 entries, about
 * 30 percent, each one mid-sentence. Silence was the defect, not the size, and
 * the reason a refusal is the right answer is the same property the rest of
 * this file rests on: the tool is additive-only, so it cannot go back and
 * repair a line it damaged. A truncated write loses the tail permanently and
 * needs a human or an Edit to fix; a refused write loses nothing, because the
 * caller still holds every character it composed and can split the entry.
 *
 * 4000 rather than 2000 because a refusal at the old cap would have refused
 * about 30 percent of the entries the fleet actually writes, which is how you
 * strand a manager mid-checkpoint. The measured average entry here is ~1200
 * characters, so this sits at more than three times a normal one: past it, a
 * line is a wall of text that belongs in two entries, and saying so out loud is
 * more useful than quietly keeping half of it.
 */
export const BRIEF_LINE_MAX = 4000;

/** How long to wait for the lock. Generous next to configLock's 250ms: this
 *  runs on a bus capability (not the Electron main thread's synchronous IPC
 *  path), and losing a brief line because another append was mid-flight is a
 *  worse outcome than a caller waiting a beat. */
const LOCK_WAIT_MS = 3_000;
/** A lock older than this had its holder die mid-write. No cross-language twin
 *  here (nothing else takes this lock), so it is tuned to the wait above. */
const LOCK_STALE_MS = 15_000;
/** Compare-and-swap attempts before giving up on an outside writer. */
const CAS_ATTEMPTS = 5;

export class BriefLineTooLong extends Error {
  constructor(
    readonly length: number,
    readonly max: number,
  ) {
    super(
      `brief.append: the line is ${length} characters and the limit is ${max}. ` +
        'Nothing was written. Split it into separate entries and append each one, ' +
        'or shorten it: this tool can only add a line, so it cannot repair one it cut.',
    );
    this.name = 'BriefLineTooLong';
  }
}

export class BriefLockTimeout extends Error {
  constructor(p: string) {
    super(`brief.md is locked by another writer (waited ${LOCK_WAIT_MS}ms): ${p}`);
    this.name = 'BriefLockTimeout';
  }
}

/** Normalize a caller's line to one brief bullet: single-line, bulleted, capped.
 *
 *  Newlines are flattened rather than refused — a manager composing a sentence
 *  with a stray wrap should not get an error, and a multi-line insert would
 *  break the "exactly one line" guarantee everything else here rests on.
 *
 *  Interior SPACES are left alone, deliberately. The obvious `\s+ → ' '` also
 *  eats the double space in the doctrine's own dated-log format
 *  (`- YYYY-MM-DD  <what happened>`), so the tool that exists to write those
 *  lines would have been the one thing that could not write one.
 *
 *  The length is measured AFTER flattening, so a caller whose sentence arrived
 *  wrapped is judged on what will actually be written, not on its newlines. */
export function normalizeBriefLine(line: string): string {
  const flat = (line ?? '')
    .replace(/[ \t\f\v]*[\r\n]+[ \t\f\v]*/g, ' ')
    .replace(/[\t\f\v]+/g, ' ')
    .trim();
  if (!flat) throw new Error('brief.append: line is empty');
  if (flat.length > BRIEF_LINE_MAX) throw new BriefLineTooLong(flat.length, BRIEF_LINE_MAX);
  return flat.startsWith('- ') || flat.startsWith('#') ? flat : `- ${flat}`;
}

/** A `## <Section>` heading line, tolerating trailing whitespace and any number
 *  of leading hashes ≥ 2 (briefs in the wild use `##`; be liberal in reading). */
function isHeadingFor(line: string, section: string): boolean {
  const m = /^#{1,6}\s+(.*?)\s*$/.exec(line);
  return !!m && m[1].toLowerCase() === section.toLowerCase();
}

/** Any heading line — where a section's body ends. */
function isHeading(line: string): boolean {
  return /^#{1,6}\s+\S/.test(line);
}

/**
 * The pure core: `content` with `line` inserted into `section`. Exported so the
 * additive guarantee can be tested directly, without a filesystem.
 *
 * Missing section → appended as a new `## <Section>` at the end of the file.
 * Missing file (empty content) → a brief skeleton, so the very first append on
 * a fresh project produces a brief with the shape the doctrine expects rather
 * than one orphan heading.
 */
export function appendToBrief(content: string, section: BriefSection, line: string): string {
  const bullet = normalizeBriefLine(line);
  if (content.trim() === '') {
    // A fresh brief. Ordered as the doctrine describes them, with the caller's
    // line in its section. Note this is the ONE case that writes lines the
    // caller did not supply — there is nothing here to preserve.
    return (
      BRIEF_SECTIONS.map((s) => `## ${s}\n${s === section ? `${bullet}\n` : ''}`).join('\n') + ''
    );
  }

  // Split KEEPING the exact line endings: a brief written on Windows, or one
  // whose last line has no trailing newline, must come back the way it went in.
  const lines = content.split('\n');
  let headingAt = -1;
  for (let i = 0; i < lines.length; i++) {
    if (isHeadingFor(lines[i], section)) {
      headingAt = i;
      break;
    }
  }

  if (headingAt < 0) {
    // No such section. Append it, separated from whatever precedes it, without
    // touching a single existing line.
    const trailing = lines[lines.length - 1] === '' ? '' : '\n';
    return `${content}${trailing}\n## ${section}\n${bullet}\n`;
  }

  // The section body runs to the next heading (or EOF).
  let end = lines.length;
  for (let i = headingAt + 1; i < lines.length; i++) {
    if (isHeading(lines[i])) {
      end = i;
      break;
    }
  }

  let at: number;
  if (isPrependSection(section)) {
    // Newest first: straight after the heading, but below a blank line the
    // author put there for spacing (inserting above it would move their blank).
    at = headingAt + 1;
    while (at < end && lines[at].trim() === '') at++;
  } else {
    // End of the section: after its last non-blank line, so trailing blank
    // separator lines before the next heading stay where the author put them.
    at = end;
    while (at > headingAt + 1 && lines[at - 1].trim() === '') at--;
  }

  lines.splice(at, 0, bullet);
  return lines.join('\n');
}

/** Resolve a project directory to its brief path. */
export function briefPathFor(projectDir: string): string {
  return path.join(projectDir, BRIEF_RELATIVE_PATH);
}

/**
 * What a section looks like after a write, so a caller learns the brief is over
 * budget WITHOUT reading it.
 *
 * The concrete failure this exists for: a manager could not read this repo's
 * `## Recently` at all, because 49 entries had grown past a read cap, and it
 * found out by failing. A count coming back on every append means the next one
 * knows to run /checkpoint before it hits that wall. Three numbers, deliberately
 * boring: two for the section the caller just wrote to, one for the file, since
 * a brief can be over budget as a whole while every single section looks fine.
 */
export interface BriefSizeReport {
  /** Entries under that heading after the write, `###` sub-groups included. */
  entriesInSection: number;
  /** Bytes of the section's body (its lines, not the heading). */
  bytesInSection: number;
  /** Bytes of the whole brief. */
  bytesInBrief: number;
}

export interface BriefAppendResult extends BriefSizeReport {
  path: string;
  section: BriefSection;
  /** The line as it was actually written (normalized). */
  line: string;
  /** True when this call created the brief. */
  created: boolean;
}

/**
 * Measure one section of `content`. Uses the board's parser rather than a second
 * one: `parseBrief` already models a brief as sections and entries with the same
 * boundary rules `appendToBrief` inserts by, and two counters that disagree
 * about what an entry is would be worse than no counter.
 *
 * A section the brief does not have reports zeroes. That is the honest answer
 * for a heading `appendToBrief` would have to create.
 */
export function briefSectionStats(content: string, section: string): BriefSizeReport {
  const wanted = section.trim().toLowerCase();
  const doc = parseBrief(content);
  const block = doc.sections.find((s) => s.level === 2 && s.title.toLowerCase() === wanted);
  const body = block ? doc.lines.slice(block.bodyStart, block.bodyEnd).join('\n') : '';
  return {
    entriesInSection: doc.entries.filter((e) => e.column.toLowerCase() === wanted).length,
    bytesInSection: Buffer.byteLength(body, 'utf8'),
    bytesInBrief: Buffer.byteLength(content, 'utf8'),
  };
}

/**
 * Append `line` to `section` of the brief at `briefPath`, atomically.
 *
 * `briefPath` is already resolved and already confined by the caller — this
 * module does no authorization; see hubCapabilities' brief.append registration,
 * which runs the same path guard fs.write does.
 */
export function appendBriefLine(
  briefPath: string,
  section: BriefSection,
  line: string,
): BriefAppendResult {
  // Normalize (and reject an empty line) BEFORE taking the lock: a caller error
  // should not make anyone else wait.
  const bullet = normalizeBriefLine(line);

  return withFileLock(
    briefPath,
    {
      staleMs: LOCK_STALE_MS,
      maxWaitMs: LOCK_WAIT_MS,
      onTimeout: (p) => new BriefLockTimeout(p),
    },
    () => {
      let lastErr: Error | undefined;
      for (let attempt = 0; attempt < CAS_ATTEMPTS; attempt++) {
        const before = readOrEmpty(briefPath);
        const next = appendToBrief(before.content, section, bullet);
        // COMPARE-AND-SWAP: re-read immediately before publishing. An agent's
        // Edit tool does not take our lock, so "nobody changed it while I was
        // computing" is a claim that has to be checked, not assumed.
        const check = readOrEmpty(briefPath);
        if (check.content !== before.content) {
          lastErr = new Error('brief.md changed under us');
          continue; // recompute against the writer that beat us
        }
        atomicWriteFileSync(briefPath, next);
        return {
          path: briefPath,
          section,
          line: bullet,
          created: !before.exists,
          // Measured on the bytes just published, not on a re-read: a re-read
          // would report whatever an outside writer did in between as if this
          // call had produced it.
          ...briefSectionStats(next, section),
        };
      }
      throw new Error(
        `brief.append: ${briefPath} is being rewritten by another writer faster than this ` +
          `could land a line (${CAS_ATTEMPTS} attempts). Nothing was written — retry, or edit ` +
          `the file directly. Last: ${lastErr?.message ?? 'unknown'}`,
      );
    },
  );
}

function readOrEmpty(p: string): { content: string; exists: boolean } {
  try {
    return { content: fs.readFileSync(p, 'utf-8'), exists: true };
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return { content: '', exists: false };
    throw err;
  }
}

/** Parse a caller-supplied section name, case-insensitively. Refused rather
 *  than defaulted: a typo'd section silently creating a `## Nwo` heading in the
 *  user's brief is exactly the kind of damage this tool must not do. */
export function parseBriefSection(name: unknown): BriefSection {
  const wanted = String(name ?? '')
    .trim()
    .toLowerCase();
  const hit = BRIEF_SECTIONS.find((s) => s.toLowerCase() === wanted);
  if (!hit) {
    throw new Error(
      `brief.append: unknown section ${JSON.stringify(name)} — expected one of ${BRIEF_SECTIONS.join(', ')}`,
    );
  }
  return hit;
}
