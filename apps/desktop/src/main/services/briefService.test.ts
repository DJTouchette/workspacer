/**
 * brief_append's two promises, tested as promises rather than as behaviour:
 *
 *  1. STRICTLY ADDITIVE — the output is the input with exactly ONE line
 *     inserted. Proved directly: remove the inserted line from the output and
 *     it must equal the input, byte for byte. That is the property the user's
 *     own brief edits depend on, and it is stronger than any set of
 *     "formatting preserved" spot checks.
 *  2. CANNOT CLOBBER — concurrent writers each land their line. Tested with
 *     real concurrency against a real file, both from this process (the lock)
 *     and from a simulated outside writer that does not take the lock (the
 *     compare-and-swap).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

/**
 * `fs.readFileSync` cannot be spied on in ESM (the namespace is not
 * configurable), and the compare-and-swap is only observable by changing the
 * file BETWEEN briefService's two reads — so the module is mocked with a
 * pass-through that a test can temporarily divert. Everything else is the real
 * fs, against a real temp directory.
 */
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

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  BRIEF_SECTIONS,
  BriefLockTimeout,
  appendBriefLine,
  appendToBrief,
  briefPathFor,
  normalizeBriefLine,
  parseBriefSection,
  type BriefSection,
} from './briefService';

const BRIEF = `# Alpha — project brief

## Now
- worker shipping the parser fix
- USER LINE: do not touch this

## Direction
- keep the parser allocation-free

## Recently
- 2026-08-20  landed the tokenizer
- 2026-08-19  opened the repo
`;

/** The additive guarantee, as a function: output minus the new line === input. */
function assertOnlyInserted(input: string, output: string, inserted: string): void {
  const lines = output.split('\n');
  const at = lines.indexOf(inserted);
  expect(at, `the inserted line ${JSON.stringify(inserted)} is in the output`).toBeGreaterThan(-1);
  lines.splice(at, 1);
  expect(lines.join('\n')).toBe(input);
}

describe('appendToBrief — strictly additive', () => {
  for (const section of BRIEF_SECTIONS.filter((s) => s !== 'User')) {
    it(`adds exactly one line to ## ${section} and changes nothing else`, () => {
      const out = appendToBrief(BRIEF, section, 'new thing');
      assertOnlyInserted(BRIEF, out, '- new thing');
    });
  }

  it('PREPENDS to Recently (newest first) and APPENDS to the others', () => {
    const recently = appendToBrief(BRIEF, 'Recently', '2026-08-21  shipped X').split('\n');
    expect(recently[recently.indexOf('## Recently') + 1]).toBe('- 2026-08-21  shipped X');

    const now = appendToBrief(BRIEF, 'Now', 'dispatched a scout').split('\n');
    // Last line of the Now body, i.e. immediately before its blank separator.
    expect(now[now.indexOf('- USER LINE: do not touch this') + 1]).toBe('- dispatched a scout');
  });

  it('never reorders or rewrites a line the user wrote', () => {
    let doc = BRIEF;
    for (let i = 0; i < 5; i++) doc = appendToBrief(doc, 'Recently', `line ${i}`);
    // Every original line still present, in its original relative order.
    const original = BRIEF.split('\n').filter((l) => l.trim() !== '');
    const after = doc.split('\n').filter((l) => l.trim() !== '');
    let cursor = -1;
    for (const line of original) {
      const at = after.indexOf(line, cursor + 1);
      expect(at, `original line preserved in order: ${line}`).toBeGreaterThan(cursor);
      cursor = at;
    }
  });

  it('creates a MISSING section at the end without touching the rest', () => {
    const out = appendToBrief(BRIEF, 'User', 'prefers local delivery');
    expect(out).toContain('## User\n- prefers local delivery');
    expect(out.startsWith(BRIEF.trimEnd())).toBe(true);
  });

  it('writes a full skeleton for an empty/missing brief', () => {
    const out = appendToBrief('', 'Recently', '2026-08-21  first line');
    for (const s of BRIEF_SECTIONS) expect(out).toContain(`## ${s}`);
    expect(out).toContain('- 2026-08-21  first line');
  });

  it('leaves the author’s blank lines where the author put them', () => {
    const spaced = '## Recently\n\n- old\n';
    const out = appendToBrief(spaced, 'Recently', 'new');
    // Below the author's blank separator, above the existing entries.
    expect(out).toBe('## Recently\n\n- new\n- old\n');
    assertOnlyInserted(spaced, out, '- new');
  });

  it('matches a heading regardless of hash depth or trailing spaces', () => {
    const odd = '### Recently   \n- old\n';
    expect(appendToBrief(odd, 'Recently', 'new').split('\n')[1]).toBe('- new');
  });
});

describe('normalizeBriefLine / parseBriefSection', () => {
  it('bullets an unbulleted line and leaves an already-bulleted one alone', () => {
    expect(normalizeBriefLine('did a thing')).toBe('- did a thing');
    expect(normalizeBriefLine('- did a thing')).toBe('- did a thing');
  });

  it('flattens newlines, so one call can only ever insert ONE line', () => {
    expect(normalizeBriefLine('a\nb\n\nc')).toBe('- a b c');
  });

  it('refuses an empty line rather than inserting a bare bullet', () => {
    expect(() => normalizeBriefLine('   ')).toThrow(/empty/);
  });

  it('caps a pathological line', () => {
    expect(normalizeBriefLine('x'.repeat(5000)).length).toBeLessThan(2100);
  });

  it('accepts a section case-insensitively and REFUSES a typo', () => {
    expect(parseBriefSection('recently')).toBe('Recently');
    expect(parseBriefSection('  NOW ')).toBe('Now');
    // A typo must not create a `## Nwo` heading in the user's own document.
    expect(() => parseBriefSection('Nwo')).toThrow(/unknown section/);
    expect(() => parseBriefSection(undefined)).toThrow(/unknown section/);
  });
});

describe('appendBriefLine — on disk', () => {
  let dir: string;
  let brief: string;
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-brief-'));
    brief = briefPathFor(dir);
  });
  afterEach(() => {
    readHook.fn = undefined;
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('creates .workspacer/brief.md on the first call and reports created', () => {
    const res = appendBriefLine(brief, 'Recently', '2026-08-21  first');
    expect(res.created).toBe(true);
    expect(res.line).toBe('- 2026-08-21  first');
    expect(fs.readFileSync(brief, 'utf-8')).toContain('- 2026-08-21  first');

    expect(appendBriefLine(brief, 'Recently', 'second').created).toBe(false);
  });

  it('leaves no lock file behind, on success or on failure', () => {
    appendBriefLine(brief, 'Now', 'a');
    expect(fs.existsSync(`${brief}.lock`)).toBe(false);
    expect(() => appendBriefLine(brief, 'Now', '  ')).toThrow();
    expect(fs.existsSync(`${brief}.lock`)).toBe(false);
  });

  // ── THE CLOBBER PROOF ─────────────────────────────────────────────────────

  it('CONCURRENT writers each land a line — none is lost', async () => {
    fs.mkdirSync(path.dirname(brief), { recursive: true });
    fs.writeFileSync(brief, BRIEF);

    const N = 12;
    // Promise.all over synchronous work is not concurrency by itself, so each
    // call is pushed onto its own macrotask: the interleaving is real, and the
    // lock is what serializes them.
    await Promise.all(
      Array.from(
        { length: N },
        (_, i) =>
          new Promise<void>((resolve, reject) =>
            setTimeout(() => {
              try {
                appendBriefLine(brief, i % 2 ? 'Recently' : 'Now', `concurrent ${i}`);
                resolve();
              } catch (e) {
                reject(e);
              }
            }, 0),
          ),
      ),
    );

    const out = fs.readFileSync(brief, 'utf-8');
    for (let i = 0; i < N; i++) expect(out).toContain(`- concurrent ${i}`);
    // …and every original line survived all twelve read-modify-writes.
    for (const line of BRIEF.split('\n').filter((l) => l.trim())) expect(out).toContain(line);
    // Exactly N lines were added — no duplicates from a retry.
    expect(out.split('\n').filter((l) => l.startsWith('- concurrent ')).length).toBe(N);
  });

  it('COMPARE-AND-SWAP: an outside writer that ignores the lock is not overwritten', () => {
    fs.mkdirSync(path.dirname(brief), { recursive: true });
    fs.writeFileSync(brief, BRIEF);

    // Simulate an agent's Edit tool landing between our read and our
    // verification read: the first read sees the old content, the second sees
    // the outsider's. Without the CAS, the outsider's line would be gone.
    let reads = 0;
    readHook.fn = (p, data) => {
      if (p === brief && reads++ === 0) fs.writeFileSync(brief, `${data}- OUTSIDER wrote this\n`);
    };

    appendBriefLine(brief, 'Recently', 'ours');

    readHook.fn = undefined;
    const out = fs.readFileSync(brief, 'utf-8');
    expect(out, "the outsider's line survived").toContain('- OUTSIDER wrote this');
    expect(out, 'and so did ours').toContain('- ours');
  });

  it('refuses rather than writing when another process holds the lock', () => {
    fs.mkdirSync(path.dirname(brief), { recursive: true });
    fs.writeFileSync(brief, BRIEF);
    fs.writeFileSync(`${brief}.lock`, '99999 held\n');
    try {
      expect(() => appendBriefLine(brief, 'Now', 'blocked')).toThrow(BriefLockTimeout);
      expect(fs.readFileSync(brief, 'utf-8')).toBe(BRIEF); // untouched
    } finally {
      fs.rmSync(`${brief}.lock`, { force: true });
    }
  }, 10_000);

  it('gives up (writing nothing) when an outside writer never stops', () => {
    fs.mkdirSync(path.dirname(brief), { recursive: true });
    fs.writeFileSync(brief, BRIEF);
    let n = 0;
    // Change the file on EVERY read, so the CAS can never confirm.
    readHook.fn = (p, data) => {
      if (p === brief) fs.writeFileSync(brief, `${data}- churn ${n++}\n`);
    };

    expect(() => appendBriefLine(brief, 'Now', 'ours')).toThrow(/another writer/);
    readHook.fn = undefined;
    // Nothing of OURS landed — a refusal, not a half-write.
    expect(fs.readFileSync(brief, 'utf-8')).not.toContain('- ours');
  });
});

describe('the section vocabulary is the doctrine’s', () => {
  it('is exactly Now / Direction / Recently / User', () => {
    // The manager doctrine (renderer lib/fleetManager.ts) and the /checkpoint
    // skill both name these four; a fifth added here without them would be a
    // heading no reader of a brief expects.
    expect([...BRIEF_SECTIONS]).toEqual<BriefSection[]>(['Now', 'Direction', 'Recently', 'User']);
  });
});
