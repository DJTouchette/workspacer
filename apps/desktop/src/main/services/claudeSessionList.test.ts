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
