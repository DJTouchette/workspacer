/**
 * claudeAccountSetup — the second-account config dir must be a new LOGIN but
 * the same brain: transcripts/memories (projects/), skills, and the hook-
 * bearing settings.json all resolve to the PRIMARY config dir's copies, while
 * nothing credential-shaped is created or linked. Everything runs against a
 * temp primary root; no real ~/.claude is touched.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  createAccountConfigDir,
  accountLoginStatus,
  claudeJsonPathFor,
  syncAccountTrust,
} from './claudeAccountSetup';

let primary: string;

beforeEach(() => {
  primary = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-acct-'));
});

afterEach(() => {
  fs.rmSync(primary, { recursive: true, force: true });
});

describe('createAccountConfigDir', () => {
  it('creates the dir under <primary>/accounts and shares projects + settings.json', () => {
    const res = createAccountConfigDir('Work', primary);
    expect(res.dir).toBe(path.join(primary, 'accounts', 'work'));
    expect(fs.statSync(res.dir).isDirectory()).toBe(true);

    // projects/ (memories + transcripts) is materialized in the primary if
    // absent and shared — a file written through the account path must appear
    // in the primary.
    expect(res.shared).toContain('projects');
    fs.writeFileSync(path.join(res.dir, 'projects', 'probe.txt'), 'x');
    expect(fs.readFileSync(path.join(primary, 'projects', 'probe.txt'), 'utf-8')).toBe('x');

    // settings.json (claudemon hooks) is materialized and shared: an edit to
    // the primary shows through the account path.
    expect(res.shared).toContain('settings.json');
    fs.writeFileSync(path.join(primary, 'settings.json'), '{"hooks":{}}');
    expect(fs.readFileSync(path.join(res.dir, 'settings.json'), 'utf-8')).toBe('{"hooks":{}}');
  });

  it('shares optional entries only when the primary has them', () => {
    fs.mkdirSync(path.join(primary, 'skills'));
    fs.writeFileSync(path.join(primary, 'CLAUDE.md'), '# global');
    const res = createAccountConfigDir('Work', primary);
    expect(res.shared).toContain('skills');
    expect(res.shared).toContain('CLAUDE.md');
    expect(res.shared).not.toContain('plugins');
    expect(res.shared).not.toContain('keybindings.json');
    expect(fs.readFileSync(path.join(res.dir, 'CLAUDE.md'), 'utf-8')).toBe('# global');
  });

  it('seeds .claude.json past onboarding without ever copying identity', () => {
    // Primary state: theme + trusted projects (safe) AND account identity
    // (must never cross — the login is the one thing an account dir owns).
    fs.writeFileSync(
      path.join(primary, '.claude.json'),
      JSON.stringify({
        theme: 'dark',
        projects: { '/home/u/work/repo': { hasTrustDialogAccepted: true } },
        oauthAccount: { emailAddress: 'primary@example.com' },
        userID: 'u-123',
      }),
    );
    const res = createAccountConfigDir('Work', primary);
    const seeded = JSON.parse(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8'));
    // A bare config dir boots into first-run onboarding, which fires no hooks
    // and left the pane "connecting" forever — the seed skips straight to the
    // /login-only REPL.
    expect(seeded.hasCompletedOnboarding).toBe(true);
    expect(seeded.theme).toBe('dark');
    expect(seeded.projects['/home/u/work/repo'].hasTrustDialogAccepted).toBe(true);
    expect(seeded.oauthAccount).toBeUndefined();
    expect(seeded.userID).toBeUndefined();
    // And nothing credential-shaped exists.
    expect(fs.existsSync(path.join(res.dir, '.credentials.json'))).toBe(false);
  });

  it('seeds onboarding-done even without a primary .claude.json', () => {
    const res = createAccountConfigDir('Work', primary);
    const seeded = JSON.parse(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8'));
    expect(seeded.hasCompletedOnboarding).toBe(true);
    expect(seeded.projects).toBeUndefined();
  });

  it('uniquifies a taken slug instead of reusing the dir', () => {
    const a = createAccountConfigDir('Work', primary);
    const b = createAccountConfigDir('Work!', primary); // same slug after cleanup
    expect(a.dir).not.toBe(b.dir);
    expect(b.dir).toBe(path.join(primary, 'accounts', 'work-2'));
  });
});

describe('claudeJsonPathFor', () => {
  it('keeps .claude.json inside an explicit (CLAUDE_CONFIG_DIR-style) root', () => {
    expect(claudeJsonPathFor('/opt/claude-root')).toBe('/opt/claude-root/.claude.json');
  });

  it('resolves the DEFAULT root to ~/.claude.json — not ~/.claude/.claude.json', () => {
    // The env-less login keeps its state file at the home root. Reading
    // inside ~/.claude found nothing, so accounts were seeded with no theme
    // and no trust map — every project's first profile spawn then parked on
    // the interactive trust dialog.
    const defaultRoot = path.join(os.homedir(), '.claude');
    expect(claudeJsonPathFor(defaultRoot)).toBe(path.join(os.homedir(), '.claude.json'));
  });
});

describe('syncAccountTrust', () => {
  const primaryJson = (projects: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
    fs.writeFileSync(
      path.join(primary, '.claude.json'),
      JSON.stringify({ projects, theme: 'dark', oauthAccount: { emailAddress: 'p@x.y' }, ...extra }),
    );

  it('copies trust for the cwd and trusted ancestors, and nothing else', () => {
    primaryJson({
      '/home/u/work': { hasTrustDialogAccepted: true, history: ['secret prompt'] },
      '/home/u/work/repo': { hasTrustDialogAccepted: true },
      '/home/u/elsewhere': { hasTrustDialogAccepted: true },
      '/home/u/work/untrusted': {},
    });
    const res = createAccountConfigDir('Work', primary);
    // Wipe the seed's map to model a pre-fix account (seeded from the wrong
    // path, i.e. empty).
    fs.writeFileSync(
      path.join(res.dir, '.claude.json'),
      JSON.stringify({ hasCompletedOnboarding: true }),
    );

    syncAccountTrust(res.dir, '/home/u/work/repo/sub', primary);

    const acct = JSON.parse(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8'));
    // cwd's trusted ancestors arrive under their own keys (trust covers
    // subdirectories) — as bare trust flags, never prompt history.
    expect(acct.projects['/home/u/work']).toEqual({ hasTrustDialogAccepted: true });
    expect(acct.projects['/home/u/work/repo']).toEqual({ hasTrustDialogAccepted: true });
    // Unrelated and untrusted primary entries never cross.
    expect(acct.projects['/home/u/elsewhere']).toBeUndefined();
    expect(acct.projects['/home/u/work/untrusted']).toBeUndefined();
    // Identity never crosses; the dropped theme is backfilled.
    expect(acct.oauthAccount).toBeUndefined();
    expect(acct.theme).toBe('dark');
    expect(acct.hasCompletedOnboarding).toBe(true);
  });

  it('preserves the account file and is a no-op when trust is already there', () => {
    primaryJson({ '/home/u/work': { hasTrustDialogAccepted: true } });
    const res = createAccountConfigDir('Work', primary);
    fs.writeFileSync(
      path.join(res.dir, '.claude.json'),
      JSON.stringify({
        hasCompletedOnboarding: true,
        theme: 'light',
        oauthAccount: { emailAddress: 'work@x.y' },
        projects: { '/home/u/work': { hasTrustDialogAccepted: true, allowedTools: ['Bash'] } },
      }),
    );
    const before = fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8');

    syncAccountTrust(res.dir, '/home/u/work', primary);

    // Already trusted → no rewrite at all (the file may belong to a LIVE
    // claude process; don't touch it without a reason).
    expect(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8')).toBe(before);
  });

  it('merges into existing entries without clobbering claude-owned fields', () => {
    primaryJson({ '/home/u/work': { hasTrustDialogAccepted: true } });
    const res = createAccountConfigDir('Work', primary);
    fs.writeFileSync(
      path.join(res.dir, '.claude.json'),
      JSON.stringify({
        hasCompletedOnboarding: true,
        projects: { '/home/u/work': { allowedTools: ['Bash'] } },
      }),
    );

    syncAccountTrust(res.dir, '/home/u/work', primary);

    const acct = JSON.parse(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8'));
    expect(acct.projects['/home/u/work']).toEqual({
      allowedTools: ['Bash'],
      hasTrustDialogAccepted: true,
    });
  });

  it('creates a minimal account .claude.json when none exists yet', () => {
    primaryJson({ '/home/u/work': { hasTrustDialogAccepted: true } });
    const dir = path.join(primary, 'accounts', 'bare');
    fs.mkdirSync(dir, { recursive: true });

    syncAccountTrust(dir, '/home/u/work', primary);

    const acct = JSON.parse(fs.readFileSync(path.join(dir, '.claude.json'), 'utf-8'));
    expect(acct.hasCompletedOnboarding).toBe(true);
    expect(acct.projects['/home/u/work']).toEqual({ hasTrustDialogAccepted: true });
  });

  it('does nothing for an untrusted cwd or a missing primary map', () => {
    primaryJson({ '/home/u/work': { hasTrustDialogAccepted: true } });
    const res = createAccountConfigDir('Work', primary);
    const before = fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8');
    syncAccountTrust(res.dir, '/home/u/never-trusted', primary);
    expect(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8')).toBe(before);
    // And a sibling name that merely shares a prefix is NOT an ancestor.
    syncAccountTrust(res.dir, '/home/u/workspace', primary);
    expect(fs.readFileSync(path.join(res.dir, '.claude.json'), 'utf-8')).toBe(before);
  });
});

describe('accountLoginStatus', () => {
  it('is false for a fresh account dir, true once credentials exist', () => {
    const res = createAccountConfigDir('Work', primary);
    expect(accountLoginStatus(res.dir)).toBe(false);
    fs.writeFileSync(path.join(res.dir, '.credentials.json'), '{}');
    expect(accountLoginStatus(res.dir)).toBe(true);
  });

  it('accepts a populated .claude.json oauthAccount (macOS keychain case)', () => {
    const res = createAccountConfigDir('Work', primary);
    fs.writeFileSync(
      path.join(res.dir, '.claude.json'),
      JSON.stringify({ oauthAccount: { emailAddress: 'a@b.c' } }),
    );
    expect(accountLoginStatus(res.dir)).toBe(true);
  });
});
