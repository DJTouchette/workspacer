/**
 * The stale-Now check, tested as the three things it promises:
 *
 *  1. IT FINDS A DEAD REFERENCE and leaves a live one alone. Anything less and
 *     it is noise a manager learns to skip.
 *  2. A FINISHED SESSION COUNTS AS GONE — that is the case that strands a line,
 *     so treating a closed worker as live would blind it to its main quarry.
 *  3. IT NEVER WRITES. Proved against a real file on disk: content, size and
 *     mtime are identical afterwards, and the directory gains no new entry (no
 *     lock file, no index, no .bak).
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { checkNowSection, isLiveDispatch, liveSessionIds } from './briefCheck';

const LIVE = 'c03bd8ce-1f4a-4b2c-9d3e-0123456789ab';
const DEAD = 'deadbeef-1f4a-4b2c-9d3e-0123456789ab';

const BRIEF = `# Alpha — project brief

## Now
- dispatched the parser fix — \`session:${LIVE.slice(0, 8)}\`
- dispatched the lexer rewrite — \`session:${DEAD.slice(0, 8)}\`
- USER LINE: keep the parser allocation-free

## Direction
- dispatched work always lands on master — session:${DEAD.slice(0, 8)}

## Recently
- 2026-08-21  shipped X (session:${DEAD.slice(0, 8)})
`;

describe('it flags a dead reference and does not flag a live one', () => {
  const report = checkNowSection(BRIEF, [LIVE], '/p/.workspacer/brief.md');

  it('flags exactly the entry whose session is gone', () => {
    const stale = report.findings.filter((f) => f.reason === 'stale');
    expect(stale).toHaveLength(1);
    expect(stale[0].text).toContain('lexer rewrite');
    expect(stale[0].refs).toEqual([DEAD.slice(0, 8)]);
  });

  it('says nothing about the entry whose session is still live', () => {
    expect(report.findings.some((f) => f.text.includes('parser fix'))).toBe(false);
    expect(report.entriesLive).toBe(1);
  });

  it('matches a SHORT brief reference against the store’s full UUID', () => {
    // The brief writes 8 hex; the store holds the whole thing. If this stopped
    // working, every line in every brief would read as stale.
    expect(report.findings.some((f) => f.refs.includes(LIVE.slice(0, 8)))).toBe(false);
  });

  it('reads ONLY ## Now — Direction and Recently are supposed to name dead sessions', () => {
    expect(report.section).toBe('Now');
    expect(report.entriesChecked).toBe(3);
    expect(report.findings.every((f) => !f.text.includes('always lands on master'))).toBe(true);
    expect(report.findings.every((f) => !f.text.includes('shipped X'))).toBe(true);
  });

  it('leaves a user line that is not a dispatch alone', () => {
    expect(report.findings.some((f) => f.text.includes('USER LINE'))).toBe(false);
  });

  it('reports how many live sessions it matched against, so zero is legible', () => {
    expect(report.liveSessions).toBe(1);
    expect(checkNowSection(BRIEF, [], '/p/b.md').liveSessions).toBe(0);
  });

  it('says "nothing to prune" rather than nothing at all when the brief is clean', () => {
    const clean = checkNowSection(
      `## Now\n- dispatched the parser fix — session:${LIVE.slice(0, 8)}\n`,
      [LIVE],
      '/p/b.md',
    );
    expect(clean.findings).toHaveLength(0);
    expect(clean.note).toMatch(/Nothing to prune/);
  });

  it('handles a brief that does not exist yet', () => {
    const empty = checkNowSection('', [LIVE], '/p/b.md');
    expect(empty.entriesChecked).toBe(0);
    expect(empty.findings).toHaveLength(0);
  });
});

describe('a malformed reference is its own finding — the transcription bug, caught', () => {
  it('flags session:6a-round2 for what it is', () => {
    const report = checkNowSection(
      '## Now\n- dispatched round two of the sweep — session:6a-round2\n',
      [LIVE],
      '/p/b.md',
    );
    expect(report.findings).toHaveLength(1);
    expect(report.findings[0].reason).toBe('malformed');
    expect(report.findings[0].refs).toEqual(['6a-round2']);
    expect(report.findings[0].detail).toMatch(/not a session id/);
  });
});

describe('a dispatch-shaped line with no reference at all', () => {
  it('is flagged, because nothing can tell you whether its worker lives', () => {
    const report = checkNowSection('## Now\n- dispatched a scout to read the hub\n', [], '/p/b.md');
    expect(report.findings.map((f) => f.reason)).toEqual(['unreferenced']);
  });

  it('but a Now line that is NOT dispatch-shaped is left alone — precision over recall', () => {
    const report = checkNowSection(
      '## Now\n- the release is blocked on the notarization ticket\n- decide whether to keep v1\n',
      [],
      '/p/b.md',
    );
    expect(report.findings).toHaveLength(0);
  });
});

describe('a FINISHED session counts as gone', () => {
  it.each([
    [{ sessionId: DEAD, mode: 'stopped' }, false],
    [{ sessionId: DEAD, status: 'ended' }, false],
    [{ sessionId: DEAD, archived: true }, false],
    [{ sessionId: LIVE, mode: 'running' }, true],
    [{ sessionId: LIVE, status: 'active' }, true],
    // Unknown-shaped and federated rows count as LIVE here: this predicate's
    // failure mode to avoid is flagging a line that is fine, the opposite of
    // snapshotGrantsFsRoot's.
    [{ sessionId: LIVE }, true],
    [{ sessionId: LIVE, hub: 'peer' }, true],
  ])('isLiveDispatch(%j) === %s', (snap, want) => {
    expect(isLiveDispatch(snap)).toBe(want);
  });

  it('a closed worker’s Now line is therefore reported stale', () => {
    const ids = liveSessionIds([
      { sessionId: LIVE, mode: 'running' },
      { sessionId: DEAD, mode: 'stopped' },
    ]);
    expect(ids).toEqual([LIVE]);
    const report = checkNowSection(BRIEF, ids, '/p/b.md');
    expect(report.findings.filter((f) => f.reason === 'stale')).toHaveLength(1);
  });
});

describe('it NEVER writes', () => {
  let dir = '';
  let briefPath = '';

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'brief-check-'));
    fs.mkdirSync(path.join(dir, '.workspacer'));
    briefPath = path.join(dir, '.workspacer', 'brief.md');
    fs.writeFileSync(briefPath, BRIEF);
  });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

  it('leaves the file byte-identical, and the directory without a new entry', () => {
    const before = fs.readFileSync(briefPath, 'utf-8');
    const statBefore = fs.statSync(briefPath);
    const entriesBefore = fs.readdirSync(path.dirname(briefPath)).sort();

    const report = checkNowSection(before, [], briefPath);
    expect(report.findings.length).toBeGreaterThan(0); // it really did have work to do

    expect(fs.readFileSync(briefPath, 'utf-8')).toBe(before);
    expect(fs.statSync(briefPath).mtimeMs).toBe(statBefore.mtimeMs);
    expect(fs.statSync(briefPath).size).toBe(statBefore.size);
    expect(fs.readdirSync(path.dirname(briefPath)).sort()).toEqual(entriesBefore);
  });

  it('reports the flagged entries VERBATIM — it does not even rewrite them in the report', () => {
    const report = checkNowSection(BRIEF, [], briefPath);
    const lines = BRIEF.split('\n');
    for (const f of report.findings) expect(lines[f.line]).toBe(f.text);
  });

  it('says out loud that it does not prune', () => {
    expect(checkNowSection(BRIEF, [], briefPath).note).toMatch(/never edits, moves or deletes/);
  });
});
