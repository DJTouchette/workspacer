/**
 * Regression test: the per-project transcript folder must be encoded the same
 * way the Claude CLI (and this repo's claudemon `encoded_cwd`) does — every
 * '/', '\\' and ':' becomes '-', with NO stripping. A leading slash therefore
 * encodes to a leading '-'. Getting this wrong means listClaudeSessionsForDir
 * looks in a non-existent folder and the resume picker is always empty.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// os.homedir() reads process.env.HOME on POSIX and %USERPROFILE% on Windows, so
// both are redirected: setting only the POSIX half does not fail on Windows, it
// silently points the test at the developer's REAL ~/.claude. (ESM forbids
// spying on os.homedir.)
const realHome = process.env.HOME;
const realUserProfile = process.env.USERPROFILE;
function useHome(dir: string): void {
  process.env.HOME = dir;
  process.env.USERPROFILE = dir;
}
afterEach(() => {
  if (realHome === undefined) delete process.env.HOME;
  else process.env.HOME = realHome;
  if (realUserProfile === undefined) delete process.env.USERPROFILE;
  else process.env.USERPROFILE = realUserProfile;
});

function seedSession(home: string, encodedDir: string, sessionId: string) {
  const dir = path.join(home, '.claude', 'projects', encodedDir);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, `${sessionId}.jsonl`),
    JSON.stringify({ type: 'user', message: { role: 'user', content: 'hi' } }) + '\n',
  );
}

describe('listClaudeSessionsForDir — project folder encoding', () => {
  it('finds sessions stored under the leading-dash encoded unix cwd', async () => {
    const home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    useHome(home);
    // Claude stores '/home/user/myproject' under '-home-user-myproject'.
    seedSession(home, '-home-user-myproject', 'sess-1');

    const { listClaudeSessionsForDir } = await import('./claudeSessionList');
    const ids = listClaudeSessionsForDir('/home/user/myproject').map((s) => s.sessionId);
    expect(ids).toEqual(['sess-1']);
  });

  it('encodes a colon (windows drive) without dropping characters', async () => {
    const home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    useHome(home);
    // 'C:\\Users\\me\\proj' -> 'C--Users-me-proj'
    seedSession(home, 'C--Users-me-proj', 'sess-win');

    const { listClaudeSessionsForDir } = await import('./claudeSessionList');
    const ids = listClaudeSessionsForDir('C:\\Users\\me\\proj').map((s) => s.sessionId);
    expect(ids).toEqual(['sess-win']);
  });
});

/**
 * The corpus half. capspec.unscopedByDecision leaves `claude.sessionsForDir`
 * unconfined by the bus on the stated grounds that the caller's string is never
 * opened as a path — and that sentence was FALSE in both copies, identically.
 * The encoder rewrites only '/', '\' and ':', so '..' survived verbatim, became
 * a real path COMPONENT, and path.join(~/.claude/projects, '..') is ~/.claude:
 * one directory out of the sandbox, where the handler enumerated and summarized
 * every *.jsonl the user owns (on a real machine, history.jsonl).
 *
 * The two copies AGREED on the escape rather than diverging on it, which is why
 * neither side's own tests found it — agreement is not correctness. So the pair
 * is pinned by the shared corpus instead: services/hub/cmd/brain/discovery.go
 * runs the same cases against its claudeProjectDirName.
 */
interface ProjectDirNameCase {
  cwd: string;
  expect: string | null;
  why: string;
}
// src/main/services/ → five levels below the repo root, where contracts/ sits.
const projectDirNameCases: ProjectDirNameCase[] = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
).projectDirNames.cases;

describe('claudeProjectDirName — contracts/path-containment-cases.json projectDirNames', () => {
  it('the corpus block is not empty', () => {
    // A block that decoded to nothing would turn every case below into zero
    // assertions and still report green.
    expect(projectDirNameCases.length).toBeGreaterThan(0);
  });

  // Counted in the BODY and filed by verdict: the refusals are what the capspec
  // exemption for this encoder rests on, and the accepts are the only thing that
  // says it still encodes. `length > 0` above sees neither.
  const tally = new SweepTally();
  for (const c of projectDirNameCases) {
    it(`${JSON.stringify(c.cwd)} → ${JSON.stringify(c.expect)}`, async () => {
      tally.ran(c.expect === null ? 'refuse' : 'accept');
      const { claudeProjectDirName } = await import('./claudeSessionList');
      const got = claudeProjectDirName(c.cwd);
      expect(got, c.why).toBe(c.expect);
      if (got !== null) {
        // Whatever it produces must be ONE plain component, or the exemption's
        // sentence is still not true.
        expect(got, c.why).not.toMatch(/^\.?\.$/);
        expect(got, c.why).not.toMatch(/[/\\]/);
      }
    });
  }

  itSweptTheWholeCorpus(tally, 'the projectDirNames block', 8);

  it('does not enumerate ~/.claude when the cwd is ".."', async () => {
    const home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    useHome(home);
    fs.mkdirSync(path.join(home, '.claude'), { recursive: true });
    fs.writeFileSync(
      path.join(home, '.claude', 'LEAK.jsonl'),
      JSON.stringify({ type: 'summary', summary: 'CONTENTS OF ~/.claude/LEAK.jsonl' }) + '\n',
    );
    seedSession(home, '-proj', 'sess-ok');

    const { listClaudeSessionsForDir } = await import('./claudeSessionList');
    for (const cwd of ['..', '.', '/proj/..']) {
      expect(listClaudeSessionsForDir(cwd).map((s) => s.sessionId)).toEqual([]);
    }
    // The floor: a legitimate lookup still resolves.
    expect(listClaudeSessionsForDir('/proj').map((s) => s.sessionId)).toEqual(['sess-ok']);
  });
});

// clip is the twin of clip() in services/hub/cmd/brain/discovery.go, and the
// summary it produces goes on the wire for claude.sessionsForDir — a method
// whichever provider is registered answers. `.slice(0, 100)` counts UTF-16 CODE
// UNITS, so every non-BMP character costs two: the same transcript came back at
// 100 code points from the brain and 50 from here, and an odd boundary left a
// LONE LEAD SURROGATE that JSON.stringify emits as a bare \ud83d and every
// consumer renders as a replacement char. The Go side has had
// TestClipDoesNotSplitRune since it was written; this side had nothing.
describe('summary clipping counts code points, not UTF-16 units', () => {
  it('keeps 100 whole characters at an odd boundary', async () => {
    const { clip } = await import('./claudeSessionList');
    const odd = 'a' + '\u{1F600}'.repeat(150);
    const got = clip(odd, 100);
    expect(Array.from(got).length).toBe(100);
    // A lone surrogate is the visible symptom; it must not survive.
    expect(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/.test(got), 'lone lead surrogate').toBe(false);
    expect(got.endsWith('\u{1F600}')).toBe(true);
  });

  it('keeps 100 whole characters at an even boundary too', async () => {
    const { clip } = await import('./claudeSessionList');
    // A UTF-16 counter keeps 50 whole characters here — valid text, and
    // therefore invisible unless something counts.
    expect(Array.from(clip('\u{1F600}'.repeat(150), 100)).length).toBe(100);
  });

  it('leaves a short summary alone and matches the Go rune vector', async () => {
    const { clip } = await import('./claudeSessionList');
    expect(clip('hi', 100)).toBe('hi');
    expect(clip('a'.repeat(99) + '\u00e9', 100)).toBe('a'.repeat(99) + '\u00e9');
  });
});
