/**
 * Regression test: the per-project transcript folder must be encoded the same
 * way the Claude CLI (and this repo's claudemon `encoded_cwd`) does — every
 * '/', '\\' and ':' becomes '-', with NO stripping. A leading slash therefore
 * encodes to a leading '-'. Getting this wrong means listClaudeSessionsForDir
 * looks in a non-existent folder and the resume picker is always empty.
 */
import { describe, it, expect, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// os.homedir() reads process.env.HOME on POSIX; point it at a temp dir so the
// test never touches the real ~/.claude. (ESM forbids spying on os.homedir.)
const realHome = process.env.HOME;
afterEach(() => {
  if (realHome === undefined) delete process.env.HOME;
  else process.env.HOME = realHome;
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
    process.env.HOME = home;
    // Claude stores '/home/user/myproject' under '-home-user-myproject'.
    seedSession(home, '-home-user-myproject', 'sess-1');

    const { listClaudeSessionsForDir } = await import('./claudeSessionList');
    const ids = listClaudeSessionsForDir('/home/user/myproject').map((s) => s.sessionId);
    expect(ids).toEqual(['sess-1']);
  });

  it('encodes a colon (windows drive) without dropping characters', async () => {
    const home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    process.env.HOME = home;
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

  for (const c of projectDirNameCases) {
    it(`${JSON.stringify(c.cwd)} → ${JSON.stringify(c.expect)}`, async () => {
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

  it('does not enumerate ~/.claude when the cwd is ".."', async () => {
    const home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    process.env.HOME = home;
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
