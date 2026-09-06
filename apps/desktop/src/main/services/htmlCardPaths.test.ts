/**
 * A card's `view_diff` path is model-authored text, so the containment guard is
 * the whole story for that action. These cases exercise the two things a
 * lexical check gets wrong — a `..` that follows a SYMLINK, and a symlink whose
 * name looks contained — against a real filesystem, because that is the only
 * place the difference exists.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { resolveHtmlCardPath } from './htmlCardPaths';

let sandbox: string;
let project: string;
let outside: string;

beforeEach(() => {
  // realpath: /tmp is a symlink on macOS, and an unresolved root would make
  // every case pass or fail for the wrong reason.
  sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-card-path-')));
  project = path.join(sandbox, 'project');
  outside = path.join(sandbox, 'outside');
  fs.mkdirSync(path.join(project, 'src'), { recursive: true });
  fs.mkdirSync(outside, { recursive: true });
  fs.writeFileSync(path.join(project, 'src', 'app.ts'), 'export const a = 1;\n');
  fs.writeFileSync(path.join(outside, 'secrets.txt'), 'nope\n');
});
afterEach(() => fs.rmSync(sandbox, { recursive: true, force: true }));

describe('resolveHtmlCardPath', () => {
  it('allows a relative file inside the owning project and returns it canonical', () => {
    const res = resolveHtmlCardPath('src/app.ts', project);
    expect(res).toEqual({ ok: true, path: path.join(project, 'src', 'app.ts') });
  });

  it('allows an absolute file inside the project', () => {
    const res = resolveHtmlCardPath(path.join(project, 'src', 'app.ts'), project);
    expect(res.ok).toBe(true);
  });

  it('refuses a textual escape', () => {
    const res = resolveHtmlCardPath('../outside/secrets.txt', project);
    expect(res).toEqual({ ok: false, error: 'that file is outside this project' });
  });

  it('refuses an absolute path outside the project', () => {
    expect(resolveHtmlCardPath(path.join(outside, 'secrets.txt'), project).ok).toBe(false);
  });

  it('refuses a file reached THROUGH a symlink that leaves the project', () => {
    // The classic one: `link/app.ts` is textually inside `project`, but the
    // link points out. A lexical join + prefix check allows this.
    fs.symlinkSync(outside, path.join(project, 'link'), 'dir');
    expect(resolveHtmlCardPath('link/secrets.txt', project).ok).toBe(false);
  });

  it("refuses a '..' that pops off a SYMLINK's target rather than its textual parent", () => {
    // project/src/up -> project/src ; so 'src/up/../../outside/secrets.txt'
    // cleans TEXTUALLY to project/outside/... (which does not exist) but
    // resolves through the link to sandbox/outside/secrets.txt.
    fs.mkdirSync(path.join(project, 'src', 'deep'), { recursive: true });
    fs.symlinkSync(path.join(project, 'src'), path.join(project, 'src', 'deep', 'up'), 'dir');
    expect(resolveHtmlCardPath('src/deep/up/../../outside/secrets.txt', project).ok).toBe(false);
  });

  it('refuses a symlink whose TARGET is outside even when its own name is inside', () => {
    fs.symlinkSync(path.join(outside, 'secrets.txt'), path.join(project, 'src', 'inside.txt'));
    expect(resolveHtmlCardPath('src/inside.txt', project).ok).toBe(false);
  });

  it('does not expand a tilde — it is an ordinary filename here', () => {
    // Were '~' expanded, this would leave the project entirely; it must instead
    // be a missing file INSIDE it.
    const res = resolveHtmlCardPath('~/.ssh/id_rsa', project);
    expect(res).toEqual({ ok: false, error: 'that file no longer exists' });
  });

  it('refuses an agent-interpreted config file even inside the project', () => {
    // The second gate every fs.* caller gets, reached through THIS caller:
    // .claude/settings.json defines hooks (commands the CLI runs unprompted)
    // and .mcp.json carries server credentials, so both are refused as reads.
    fs.mkdirSync(path.join(project, '.claude'), { recursive: true });
    fs.writeFileSync(path.join(project, '.claude', 'settings.json'), '{}');
    fs.writeFileSync(path.join(project, '.mcp.json'), '{}');
    for (const target of ['.claude/settings.json', '.mcp.json']) {
      const res = resolveHtmlCardPath(target, project);
      expect(res.ok).toBe(false);
      if (!res.ok) expect(res.error).toContain('credentials or agent configuration');
    }
  });

  it('refuses anything inside .git', () => {
    fs.mkdirSync(path.join(project, '.git'), { recursive: true });
    fs.writeFileSync(path.join(project, '.git', 'config'), '[core]\n');
    expect(resolveHtmlCardPath('.git/config', project).ok).toBe(false);
  });

  it('refuses a file that no longer exists, and a directory', () => {
    expect(resolveHtmlCardPath('src/gone.ts', project)).toEqual({
      ok: false,
      error: 'that file no longer exists',
    });
    expect(resolveHtmlCardPath('src', project)).toEqual({
      ok: false,
      error: 'that path is not a file',
    });
  });

  it('refuses when the host supplied no cwd, rather than falling back to one', () => {
    // A card must never be resolved against the process working directory: an
    // absent cwd is a refusal, not "wherever the app happens to be".
    expect(resolveHtmlCardPath('src/app.ts', '').ok).toBe(false);
    expect(resolveHtmlCardPath('src/app.ts', undefined).ok).toBe(false);
    expect(resolveHtmlCardPath('', project).ok).toBe(false);
    expect(resolveHtmlCardPath(42 as unknown as string, project).ok).toBe(false);
  });
});
