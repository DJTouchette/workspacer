/**
 * replay.* is confined to the SUBTREE the guarded cwd named, not to the whole
 * repository.
 *
 * replay.open's only confinement is `assertPathAllowed(cwd, workspaceRoots())`
 * in hubCapabilities — and it then cuts its worktree from `rev-parse
 * --show-toplevel`, a root DERIVED from that cwd and never itself checked
 * against the allow-list. With an agent cwd of `<repo>/frontend` (the only
 * workspace root) the checkout was the WHOLE repository, so replay.read returned
 * the committed content of `<repo>/backend/prod-key.pem` — a file in no agent
 * cwd and no config store, which fs.read and fs.watch refuse for the same
 * caller.
 *
 * capspec.unscopedByDecision grants replay.read / replay.diff to a plugin with
 * NO fsRoots at all, on the stated grounds that "the path is a repo-relative
 * coordinate inside a worktree the replay service itself created … containment
 * is structural". That containment was real, but it was measured against a
 * namespace strictly larger than the one the guard approved.
 */
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { timelineReplay } from './timelineReplayService';

let repo: string;

function git(args: string[]): void {
  execFileSync('git', args, {
    cwd: repo,
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 't',
      GIT_AUTHOR_EMAIL: 't@e',
      GIT_COMMITTER_NAME: 't',
      GIT_COMMITTER_EMAIL: 't@e',
    },
    stdio: 'pipe',
  });
}

beforeAll(() => {
  repo = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-replay-scope-')));
  fs.mkdirSync(path.join(repo, 'frontend', 'src'), { recursive: true });
  fs.mkdirSync(path.join(repo, 'backend'), { recursive: true });
  fs.writeFileSync(path.join(repo, 'frontend', 'src', 'app.ts'), 'export const app = 1;\n');
  fs.writeFileSync(path.join(repo, 'backend', 'prod-key.pem'), 'PRIVATE-KEY-MATERIAL\n');
  git(['init', '-q']);
  git(['add', '-A']);
  git(['commit', '-qm', 'base']);
});

afterAll(async () => {
  // Close BEFORE the repo goes: close() runs `git worktree remove` from the
  // repo root, and a missing cwd surfaces as an unhandled ENOENT rejection.
  for (const id of ['scope-session', 'root-session']) {
    await timelineReplay.close(id).catch(() => undefined);
  }
  fs.rmSync(repo, { recursive: true, force: true });
});

describe('replay.* is scoped to the cwd the guard approved', () => {
  it('reads inside the guarded subtree and refuses a sibling one', async () => {
    const cwd = path.join(repo, 'frontend');
    const opened = await timelineReplay.open(cwd, 'scope-session');
    expect(opened.dir).toBeTruthy();

    // The floor: the subtree the caller actually named still works, and the
    // path it gets back is worktree-relative (what git wants).
    const inside = await timelineReplay.read('scope-session', 'frontend/src/app.ts');
    expect(inside.content).toBe('export const app = 1;\n');

    // The sibling subtree is in no agent cwd and no config store.
    await expect(timelineReplay.read('scope-session', 'backend/prod-key.pem')).rejects.toThrow(
      /outside the replay worktree/,
    );
    await expect(timelineReplay.read('scope-session', '../backend/prod-key.pem')).rejects.toThrow(
      /outside the replay worktree/,
    );
    // …and so is a diff of it.
    await expect(timelineReplay.diff('scope-session', 'backend/prod-key.pem')).rejects.toThrow(
      /outside the replay worktree/,
    );
  });

  it('a replay opened AT the repo root still sees the whole repo', async () => {
    // The scope is the offset of the guarded cwd, so a root cwd narrows nothing
    // — otherwise this fix would be indistinguishable from "refuse everything".
    await timelineReplay.open(repo, 'root-session');
    const got = await timelineReplay.read('root-session', 'backend/prod-key.pem');
    expect(got.content).toBe('PRIVATE-KEY-MATERIAL\n');
  });
});
