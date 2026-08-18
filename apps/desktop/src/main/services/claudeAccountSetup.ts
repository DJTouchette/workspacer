/**
 * Second-Claude-account setup: build a CLAUDE_CONFIG_DIR that is a new LOGIN
 * but the same brain.
 *
 * A Claude profile already lets a spawn run against any config dir (that is
 * how two accounts run side by side — each PTY gets its own env), but a bare
 * directory forks everything: no transcripts, no auto-memory, no skills, no
 * hooks — and without claudemon's hooks in its settings.json the session is
 * invisible to the whole app. This module materializes an account dir that
 * SHARES all of that with the primary config dir via links and keeps only the
 * identity local:
 *
 *   shared (symlinked): projects/ (transcripts + auto-memory + resume
 *     history — the History pane unifies for free), skills/, agents/,
 *     commands/, plugins/, todos/, settings.json (hooks + permissions +
 *     statusline: claudemon init keeps writing through the link), CLAUDE.md,
 *     keybindings.json;
 *   local (never linked): .credentials.json and .claude.json — the login
 *     itself, plus per-account onboarding/trust state.
 *
 * Account dirs live at <primary>/accounts/<slug> — INSIDE the primary config
 * root — deliberately: `.claude.json` is denied by basename everywhere
 * (pathConfinement AGENT_CONFIG_BASENAMES), and the linked settings.json
 * canonicalizes to `<primary>/settings.json`, which the `.claude/<child>`
 * confinement rule covers. A dir elsewhere would leave copied fallbacks
 * unguarded.
 *
 * Windows: directory links use junctions (no privilege needed); file links
 * try symlink → hardlink → copy, and a copy is reported as a warning since
 * later edits to the primary won't propagate.
 *
 * The first spawn against a fresh account dir shows Claude Code's login (and
 * per-project trust) prompt in the pane — logging in once persists to that
 * dir's credentials and every later spawn with the profile just works.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/** Subdirectories of the primary config dir an account shares wholesale. */
const SHARED_DIRS = ['projects', 'skills', 'agents', 'commands', 'plugins', 'todos'];

/** Files an account shares. settings.json is the load-bearing one (hooks). */
const SHARED_FILES = ['settings.json', 'settings.local.json', 'CLAUDE.md', 'keybindings.json'];

export interface AccountDirResult {
  /** The new CLAUDE_CONFIG_DIR (absolute). */
  dir: string;
  /** Entry names that ended up shared (linked or, on fallback, copied). */
  shared: string[];
  /** Non-fatal problems, e.g. a file that had to be copied instead of linked. */
  warnings: string[];
}

/** The primary Claude config root this process (and the default profile) use. */
export function primaryClaudeConfigDir(): string {
  const override = process.env.CLAUDE_CONFIG_DIR;
  return override && override.trim() !== '' ? override : path.join(os.homedir(), '.claude');
}

function slugify(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'account';
}

/** Link `target` at `linkPath`; directories become junctions on Windows. */
function linkDir(target: string, linkPath: string): void {
  fs.symlinkSync(target, linkPath, process.platform === 'win32' ? 'junction' : 'dir');
}

/** Symlink → hardlink → copy, in that order. Returns a warning for a copy. */
function linkFile(target: string, linkPath: string): string | null {
  try {
    fs.symlinkSync(target, linkPath, 'file');
    return null;
  } catch {
    /* fall through */
  }
  try {
    fs.linkSync(target, linkPath);
    return null;
  } catch {
    /* fall through */
  }
  fs.copyFileSync(target, linkPath);
  return `${path.basename(linkPath)} could not be linked and was copied — future edits to the primary copy won't propagate to this account`;
}

/**
 * Create the account config dir under `<primaryRoot>/accounts/` and share the
 * primary's brain into it. Idempotent per name only in the sense that a taken
 * slug gets a numeric suffix — callers get a fresh dir every time.
 */
export function createAccountConfigDir(name: string, primaryRoot?: string): AccountDirResult {
  const primary = primaryRoot ?? primaryClaudeConfigDir();
  const accountsRoot = path.join(primary, 'accounts');
  fs.mkdirSync(accountsRoot, { recursive: true });

  const base = slugify(name);
  let dir = path.join(accountsRoot, base);
  for (let n = 2; fs.existsSync(dir); n++) dir = path.join(accountsRoot, `${base}-${n}`);
  fs.mkdirSync(dir);

  const shared: string[] = [];
  const warnings: string[] = [];

  for (const entry of SHARED_DIRS) {
    const target = path.join(primary, entry);
    // projects/ is the point of the whole exercise (memories + transcripts) —
    // materialize it in the primary if it doesn't exist yet. The others are
    // shared only when the primary actually has them.
    if (!fs.existsSync(target)) {
      if (entry !== 'projects') continue;
      fs.mkdirSync(target, { recursive: true });
    }
    try {
      linkDir(target, path.join(dir, entry));
      shared.push(entry);
    } catch (err) {
      warnings.push(
        `could not share ${entry}/: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }

  for (const entry of SHARED_FILES) {
    const target = path.join(primary, entry);
    // settings.json carries claudemon's hooks — without it the account's
    // sessions are invisible to the app. Materialize an empty one in the
    // primary if needed; claudemon init merges into it through the link.
    if (!fs.existsSync(target)) {
      if (entry !== 'settings.json') continue;
      fs.writeFileSync(target, '{}\n');
    }
    try {
      const warning = linkFile(target, path.join(dir, entry));
      shared.push(entry);
      if (warning) warnings.push(warning);
    } catch (err) {
      warnings.push(
        `could not share ${entry}: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }

  return { dir, shared, warnings };
}

/**
 * Best-effort "has anyone logged in here yet". Credentials live in
 * `.credentials.json` on Linux/Windows; on macOS they sit in the Keychain, so
 * a populated `.claude.json` oauthAccount is accepted as the signal there.
 * Wrong answers are cheap either way: the pane itself shows the login prompt.
 */
export function accountLoginStatus(configDir: string): boolean {
  const root = configDir.replace(/^~/, os.homedir());
  if (fs.existsSync(path.join(root, '.credentials.json'))) return true;
  try {
    const claudeJson = JSON.parse(fs.readFileSync(path.join(root, '.claude.json'), 'utf-8')) as {
      oauthAccount?: unknown;
    };
    return !!claudeJson.oauthAccount;
  } catch {
    return false;
  }
}
