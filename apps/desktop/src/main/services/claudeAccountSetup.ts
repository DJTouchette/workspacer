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
 *     itself, plus per-account onboarding/trust state. `.claude.json` is
 *     SEEDED (see seedClaudeJson): a truly bare config dir boots Claude into
 *     first-run onboarding (theme picker → trust → login), which fires no
 *     hooks — the pane's GUI view showed an eternal "connecting" while sends
 *     queued against a prompt that never settled, which read as "claudemon is
 *     unresponsive". Seeding onboarding-done + the primary's theme and
 *     per-project trust map boots the account straight to a live REPL whose
 *     only remaining step is /login.
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

/**
 * Where a config root keeps its `.claude.json`. Claude Code writes it INSIDE
 * the dir when `CLAUDE_CONFIG_DIR` points there (that's what account dirs
 * are), but the default env-less login keeps it at `~/.claude.json` — NOT
 * `~/.claude/.claude.json`. Reading the latter is how the account seed
 * silently copied nothing: no theme, no per-project trust map, so every
 * project's first spawn under a profile parked on the interactive
 * folder-trust dialog — a screen the GUI pane never renders, which read as
 * "the profile spawn is dead / fell back to the default account".
 */
export function claudeJsonPathFor(configRoot: string): string {
  return path.resolve(configRoot) === path.join(os.homedir(), '.claude')
    ? path.join(os.homedir(), '.claude.json')
    : path.join(configRoot, '.claude.json');
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

  seedClaudeJson(primary, dir);

  return { dir, shared, warnings };
}

/**
 * Seed the account's `.claude.json` so the first spawn boots a live REPL
 * instead of parking at first-run onboarding, which fires no hooks and left
 * the pane "connecting" forever (verified against a bare config dir: theme
 * picker → trust dialog → login, each an interactive screen only the terminal
 * view can answer). Copied from the primary, WHITELIST only:
 *
 *   - hasCompletedOnboarding (forced true) + theme — skips the theme screen;
 *   - projects — the per-project trust map (and prompt history), so folders
 *     the user already trusts on the primary login are trusted here too.
 *
 * Never the account identity (oauthAccount, userID, …): a whitelist can't
 * leak a key it doesn't name, which is the point — the login is the ONLY
 * thing an account dir is supposed to own. After this, the sole remaining
 * interactive step is /login itself (ClaudePane banners it).
 */
function seedClaudeJson(primary: string, dir: string): void {
  const seed: Record<string, unknown> = { hasCompletedOnboarding: true };
  try {
    const primaryJson = JSON.parse(fs.readFileSync(claudeJsonPathFor(primary), 'utf-8')) as Record<
      string,
      unknown
    >;
    if (primaryJson.theme !== undefined) seed.theme = primaryJson.theme;
    if (primaryJson.projects !== undefined) seed.projects = primaryJson.projects;
  } catch {
    // No primary .claude.json (or unreadable) — the defaults still skip the
    // theme screen; trust dialogs simply appear once per project.
  }
  fs.writeFileSync(path.join(dir, '.claude.json'), JSON.stringify(seed, null, 2) + '\n', {
    mode: 0o600,
  });
}

/**
 * Propagate folder trust from the primary login into a profile's config dir,
 * called best-effort right before every profile spawn (both transports).
 *
 * The account's `.claude.json` is deliberately NOT linked to the primary (it
 * owns the login), so its per-project trust map is a snapshot from account
 * creation — and the original seed read the wrong path entirely (see
 * claudeJsonPathFor), leaving it empty. Either way the symptom is the same:
 * spawn the profile in any project the ACCOUNT copy doesn't trust and claude
 * boots into the interactive trust dialog instead of a REPL — no hooks fire,
 * claudemon reports mode "unknown", and the pane just looks dead.
 *
 * A folder the user already trusted on the primary login is the same human on
 * the same machine, so the account inherits exactly that decision and nothing
 * else: only `hasTrustDialogAccepted: true` crosses, under the primary map's
 * own keys (trust covers subdirectories, so ancestor entries count). Genuinely
 * new folders still prompt — in the Terminal view, where the dialog renders.
 * `primaryRoot` is a test seam; production callers pass nothing.
 */
export function syncAccountTrust(
  configDir: string,
  cwd: string | undefined,
  primaryRoot?: string,
): void {
  if (!cwd || !configDir.trim()) return;
  const accountJsonPath = path.join(configDir.replace(/^~/, os.homedir()), '.claude.json');
  try {
    const primaryJson = JSON.parse(
      fs.readFileSync(claudeJsonPathFor(primaryRoot ?? primaryClaudeConfigDir()), 'utf-8'),
    ) as { projects?: Record<string, { hasTrustDialogAccepted?: unknown }>; theme?: unknown };
    const primaryProjects = primaryJson?.projects;
    if (!primaryProjects || typeof primaryProjects !== 'object') return;
    const resolvedCwd = path.resolve(cwd);
    const trustedKeys = Object.keys(primaryProjects).filter((key) => {
      if (primaryProjects[key]?.hasTrustDialogAccepted !== true) return false;
      const resolvedKey = path.resolve(key);
      return resolvedCwd === resolvedKey || resolvedCwd.startsWith(resolvedKey + path.sep);
    });
    if (trustedKeys.length === 0) return;

    let account: Record<string, unknown>;
    try {
      account = JSON.parse(fs.readFileSync(accountJsonPath, 'utf-8')) as Record<string, unknown>;
    } catch {
      // Account dir without a .claude.json yet (pre-seed or hand-built
      // profile): start it the same way seedClaudeJson would.
      account = { hasCompletedOnboarding: true };
    }
    const accountProjects =
      account.projects && typeof account.projects === 'object'
        ? (account.projects as Record<string, Record<string, unknown>>)
        : {};
    let changed = false;
    for (const key of trustedKeys) {
      if (accountProjects[key]?.hasTrustDialogAccepted === true) continue;
      accountProjects[key] = { ...(accountProjects[key] ?? {}), hasTrustDialogAccepted: true };
      changed = true;
    }
    // The broken seed also dropped the theme — backfill it while we're here.
    if (account.theme === undefined && primaryJson.theme !== undefined) {
      account.theme = primaryJson.theme;
      changed = true;
    }
    if (!changed) return;
    account.projects = accountProjects;
    fs.writeFileSync(accountJsonPath, JSON.stringify(account, null, 2) + '\n', { mode: 0o600 });
  } catch {
    // Best-effort: worst case the pane shows the trust dialog, as before.
  }
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
