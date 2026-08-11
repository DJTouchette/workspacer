/**
 * Filesystem path confinement — the desktop copy of the shared containment rule.
 *
 * Extracted out of services/hubCapabilities.ts so the cross-language contract
 * (contracts/path-containment-cases.json) can pin the predicate itself rather
 * than reaching it through a capability handler. Two other copies implement the
 * same algorithm: the Go brain (cmd/brain/fsguard.go), which is the DEFAULT
 * answerer for the fs.* and library.* methods under DELEGATE_CATALOG_TO_BRAIN,
 * and the bus's per-plugin grant confinement (internal/bus/policy.go). The three
 * copies must agree;
 * the fixture is what keeps them agreeing.
 *
 * The rule, in one line: canonicalize the caller's path per component, require
 * the result to sit at or inside one of the allowed roots, then refuse it anyway
 * if it is a credential.
 *
 * Two properties are load-bearing and easy to lose in a refactor:
 *
 *   1. NO TILDE EXPANSION, anywhere. '~' is an ordinary filename here. The brain
 *      used to expand it at its guard call sites while this side did not, so the
 *      same string was allowed by one provider and denied by the other.
 *
 *   2. CANONICALIZATION IS A PER-COMPONENT WALK, never a textual clean.
 *      path.resolve / path.normalize / filepath.Clean collapse 'link/..' before
 *      any symlink is read, so the guard would check <root>/x while the handler
 *      opened <somewhere-else>/x. Only fs.lstatSync and fs.readlinkSync are used
 *      below, and every caller must hand the RETURNED canonical path to the
 *      filesystem operation — check-path and opened-path cannot be allowed to
 *      differ.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { getConfigDir } from '../services/configService';

/** Cap on symlinks followed in one walk. The walk is hand-rolled (the platform
 *  realpath is what we are deliberately not using), so the ELOOP protection has
 *  to be hand-rolled too or a two-link cycle spins forever.
 *
 *  A TWIN-PARITY constant: `maxLinkHops` in cmd/brain/fsguard.go and
 *  internal/bus/policy.go must hold the same number, and until
 *  contracts/path-containment-cases.json declared it none of the three was
 *  compared to anything — raising this one to 4000 left the whole suite green,
 *  and the guard would then canonicalize chains the kernel refuses at 40, so its
 *  answer would stop describing a path the OS can open. Exported so the drift
 *  guard can read it. */
export const MAX_LINK_HOPS = 40;

/**
 * Whether a MISSING component may be walked through.
 *
 * The walk continues past a component that does not exist (a file `fs.write` is
 * about to create), but "does not exist" has two meanings and only POSIX
 * distinguishes them: a genuinely missing name is ENOENT, a name under a
 * REGULAR FILE is ENOTDIR. Windows reports both as ERROR_PATH_NOT_FOUND, which
 * Node surfaces as ENOENT — so without this the guard walks through files on
 * Windows and returns contained where POSIX returns an error. A parent that
 * does not exist is the ordinary missing-tail case; only an existing
 * non-directory stops the walk.
 *
 * Twin of parentIsWalkable in services/hub/cmd/brain/fsguard.go and
 * services/hub/internal/bus/policy.go.
 */
function parentIsWalkable(parent: string): boolean {
  try {
    return fs.lstatSync(parent).isDirectory();
  } catch {
    return true; // missing tail: deeper components are missing too
  }
}

const WIN32 = process.platform === 'win32';
/** Splitting alphabet. Windows accepts both slashes; POSIX only '/'. */
const SEP_RE = WIN32 ? /[\\/]+/ : /\/+/;

function isSepChar(ch: string): boolean {
  return ch === '/' || (WIN32 && ch === '\\');
}

interface SplitPath {
  /** '/' on POSIX; 'C:\' or '\\server\share\' on Windows. Always ends in a separator. */
  volume: string;
  /** The component region, split on separator runs, with '' and '.' discarded.
   *  '..' is NOT discarded — the walk handles it. */
  comps: string[];
}

function splitComponents(region: string): string[] {
  return region.split(SEP_RE).filter((c) => c !== '' && c !== '.');
}

/**
 * Split an ABSOLUTE path into its volume prefix and components, or null when it
 * is not absolute. A leading '~' makes a path relative, not special: nothing in
 * this module expands it.
 */
function splitAbsolute(s: string): SplitPath | null {
  if (WIN32) {
    const drive = /^([A-Za-z]:)[\\/]/.exec(s);
    if (drive) return { volume: `${drive[1]}\\`, comps: splitComponents(s.slice(2)) };
    const unc = /^[\\/]{2}([^\\/]+)[\\/]+([^\\/]+)(?:[\\/]|$)/.exec(s);
    if (unc) {
      return {
        volume: `\\\\${unc[1]}\\${unc[2]}\\`,
        comps: splitComponents(s.slice(unc[0].length)),
      };
    }
    return null;
  }
  if (!s.startsWith('/')) return null;
  return { volume: '/', comps: splitComponents(s.slice(1)) };
}

/** Append one component to a path this module built. Pure string arithmetic —
 *  no normalization, so it can be handed a component that is a '..' target of a
 *  symlink without collapsing anything. */
function appendComponent(base: string, name: string): string {
  return isSepChar(base.slice(-1)) ? base + name : base + path.sep + name;
}

/** The textual parent of an ALREADY-RESOLVED path, clamped at the volume. Safe
 *  precisely because `base` is symlink-free by construction, so its textual
 *  parent is its real parent — the property a whole-path clean destroys. */
function parentOf(base: string, volume: string): string {
  if (base === volume) return volume;
  const idx = base.lastIndexOf(path.sep);
  if (idx < 0) return volume;
  const parent = base.slice(0, idx);
  if (parent.length < volume.length || parent === '') return volume;
  return parent;
}

/**
 * What Win32 does to every component before the filesystem sees it: trailing
 * spaces and dots are stripped. The identity on POSIX, where both are ordinary
 * filename characters.
 *
 * Returns null for a component made ONLY of dots and spaces ('...', '   '),
 * which Win32 trims to nothing — a component that names no file is never
 * "probably fine". '.' and '..' are recognised before the trim, because
 * trimming dots would erase them.
 *
 * Twin of winCanonComponent in services/hub/cmd/brain/fsguard.go and
 * services/hub/internal/bus/policy.go. The escape it closes was measured on the
 * Windows runner: the guard allowed '<root>/layouts/.. ' as a literal child, and
 * the handler — going through Win32, which reads it as '..' — listed the config
 * dir instead.
 */
function winCanonComponent(c: string): string | null {
  if (!WIN32) return c;
  const trimmed = c.replace(/[ .]+$/, '');
  if (trimmed !== '') return trimmed;
  const spaceOnly = c.replace(/ +$/, '');
  return spaceOnly === '.' || spaceOnly === '..' ? spaceOnly : null;
}

/**
 * Canonicalize `target`: absolute, with '.', '..' and symlinks resolved by
 * walking one component at a time against the already-resolved prefix.
 *
 * Throws — and every caller treats a throw as a denial — when the target is
 * empty/whitespace-only, is not absolute, hits a symlink cycle, or produces any
 * lstat error other than ENOENT. ENOENT is the one continue-condition: a file
 * fs.write is about to create does not exist yet, and the remaining components
 * are appended (not switched to a lexical mode — a later '..' can pop back onto
 * a path that does exist, and the walk resumes resolving there).
 */
export function canonicalizePath(target: string): string {
  if (target.trim() === '') throw new Error('path is empty');
  // Everything past the emptiness test uses the ORIGINAL string: a filename may
  // legitimately begin or end with a space.
  const split = splitAbsolute(target);
  if (!split) throw new Error('path is not absolute');

  const { volume } = split;
  const queue = [...split.comps];
  let resolved = volume;
  let hops = 0;

  while (queue.length > 0) {
    const raw = queue.shift() as string;
    // Win32 strips trailing spaces/dots before the filesystem sees the name, so
    // the walk must too — otherwise its answer names a different file than the
    // caller then opens. Identity on POSIX.
    const c = winCanonComponent(raw);
    if (c === null) throw new Error('path component names nothing');
    if (c === '.') continue; // '. ' normalizes to '.' on Windows
    if (c === '..') {
      resolved = parentOf(resolved, volume);
      continue;
    }
    const next = appendComponent(resolved, c);
    let st: fs.Stats;
    try {
      st = fs.lstatSync(next);
    } catch (err) {
      // ENOENT — and ONLY ENOENT — keeps walking. ENOTDIR, EACCES, EPERM,
      // ELOOP, ENAMETOOLONG and anything unrecognised fail the whole call:
      // swallowing them into "contained" is an escape, and swallowing them into
      // "not contained, keep looking" turns the guard into an existence oracle.
      //
      // ENOENT is not self-sufficient on Windows. There, a path THROUGH a
      // regular file raises ERROR_PATH_NOT_FOUND, which Node reports as ENOENT
      // and Go maps to not-exist — the same code a genuinely missing name gets,
      // where POSIX separates them with ENOTDIR. So the walk asks the question
      // directly instead of inheriting the platform's opinion of it. Twin of
      // parentIsWalkable in services/hub/cmd/brain/fsguard.go and
      // services/hub/internal/bus/policy.go; all three collapsed identically.
      if ((err as NodeJS.ErrnoException).code === 'ENOENT' && parentIsWalkable(resolved)) {
        resolved = next;
        continue;
      }
      throw err;
    }
    if (st.isSymbolicLink()) {
      hops += 1;
      if (hops > MAX_LINK_HOPS) throw new Error('too many levels of symbolic links');
      const link = fs.readlinkSync(next); // any error here fails closed
      const linkSplit = splitAbsolute(link);
      if (linkSplit) {
        resolved = linkSplit.volume;
        queue.unshift(...linkSplit.comps);
      } else {
        // Relative link: interpreted against the directory that CONTAINS the
        // link, which is `resolved` — so `resolved` deliberately does not
        // advance to `next` in this branch.
        queue.unshift(...splitComponents(link));
      }
      continue;
    }
    resolved = next;
  }
  return resolved;
}

/**
 * Canonical form of an allowed root, or null to DISCARD it.
 *
 * A discarded root is skipped and nothing else: one stale session snapshot with
 * an empty cwd must not disable every other root, and must not silently become
 * the process working directory either (path.resolve('') returns exactly that).
 * A root that does not exist yet still canonicalizes — the config-dir stores are
 * created lazily and have to be comparable before anything has been saved.
 */
export function canonicalRoot(root: string): string | null {
  if (root.trim() === '') return null;
  if (!splitAbsolute(root)) return null; // relative, or a '~' that nobody expands
  try {
    return canonicalizePath(root);
  } catch {
    return null;
  }
}

/** Containment between two ALREADY-canonical paths. */
// Exported so the empty-root arm can be asserted DIRECTLY, the way the brain's
// twin (containsPath) is. Reaching it through isWithin/pathWithinRoots is not
// enough: canonicalRoot('') already fails, so those two answer false for their
// own reason and the comparison below is never asked the question.
export function containsCanonical(canonicalRootPath: string, canonicalTarget: string): boolean {
  // An empty root grants NOTHING. Without this it is a WILDCARD: neither branch
  // below sees a trailing separator, so it falls through to
  // `startsWith('/')` — true for every absolute path. canonicalRoot discards a
  // root it cannot resolve, but the last line of defence must not itself be the
  // widest possible grant.
  if (canonicalRootPath === '') return false;
  if (canonicalTarget === canonicalRootPath) return true;
  // A root that canonicalized to a volume prefix ('/', 'C:\', '\\srv\share\')
  // already ends in a separator and contains everything below it. Appending a
  // second separator would produce '//' and match nothing — that is not
  // fail-closed, it is simply wrong containment.
  if (canonicalRootPath.endsWith(path.sep)) return canonicalTarget.startsWith(canonicalRootPath);
  // The separator is mandatory: without it, root '/srv/fo' contains '/srv/foo'.
  return canonicalTarget.startsWith(canonicalRootPath + path.sep);
}

/** Fold A-Z only. Deliberately not `toLowerCase()`: the three copies have to fold
 *  IDENTICALLY, and JavaScript's Unicode lowering and Go's disagree (U+0130 'İ'
 *  already bit the filename slugs — and Go's ToLower and JS's toLowerCase do not
 *  even agree with EACH OTHER on it: 'i' vs 'i'+U+0307).
 *
 *  Exported so the fixture's `asciiFold` vectors can pin the primitive directly,
 *  the way the two Go twins are. Reaching it through a containment verdict is not
 *  enough: every case-variant case in the corpus uses pure A-Z spellings, which
 *  both folds handle identically, so `return s.toLowerCase()` passed the whole
 *  corpus and both full suites. */
export function asciiLower(s: string): string {
  let out = '';
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    out += c >= 65 && c <= 90 ? String.fromCharCode(c + 32) : s[i];
  }
  return out;
}

/**
 * `containsCanonical` with ASCII case folded away. Used ONLY by the secret gate,
 * and only in the direction where folding DENIES more: deciding a target is
 * inside the config dir, never that it is inside a store carve-out.
 *
 * Containment for ROOTS stays byte-exact, because there folding would widen an
 * allow-list. The secret gate is the mirror image: macOS (APFS, case-insensitive
 * by default) and Windows (NTFS) both open `<configDir>/remote-token` when
 * handed `<configHome>/WORKSPACER/remote-token`, and the per-component walk
 * cannot see it — `lstatSync` succeeds on the caller's spelling and the walk
 * appends the caller's spelling, so a byte-exact gate answers for the STRING
 * rather than for the file. Folding is fail-closed on every platform.
 */
function containsCanonicalFolded(canonicalRootPath: string, canonicalTarget: string): boolean {
  return containsCanonical(asciiLower(canonicalRootPath), asciiLower(canonicalTarget));
}

/** True when an already-canonicalized `canonicalTarget` sits at or inside `root`.
 *  A root that cannot be canonicalized is discarded, i.e. contains nothing. */
export function isWithin(canonicalTarget: string, root: string): boolean {
  const cr = canonicalRoot(root);
  if (cr === null) return false;
  return containsCanonical(cr, canonicalTarget);
}

/** True when an already-canonicalized `canonicalTarget` sits at or inside one of
 *  `roots`. An empty (or entirely discarded) allow-list means NOTHING is
 *  allowed; it never means unrestricted. */
export function pathWithinRoots(roots: string[], canonicalTarget: string): boolean {
  return roots.some((r) => isWithin(canonicalTarget, r));
}

/**
 * Resolve ONE directory-listing entry against the store directory it came from,
 * or null to skip it. The returned path is the string the caller must open
 * (BINDING DECISION 2).
 *
 * The entry name is a bare basename, so the join cannot escape textually — but a
 * SYMLINK named `x.yaml` is a perfectly legal entry and readFileSync follows it,
 * and <configDir>/layouts and <configDir>/sessions are exactly the directories a
 * bus caller may write into. Without this, `layouts.list` / `sessions.list`
 * launder out-of-store bytes into their own responses; the Go twin
 * (cmd/brain/stores.go storeEntryPath) additionally used to COPY them to a
 * `.broken-*` sibling that `fs.read` then handed back.
 */
export function resolveStoreEntry(dir: string, name: string): string | null {
  try {
    const canonical = canonicalizePath(path.join(dir, name));
    return isWithin(canonical, dir) ? canonical : null;
  } catch {
    return null; // unverifiable → skip, same posture as the fs.* guard
  }
}

/** The config-dir subtrees the web/remote UI legitimately reads and writes.
 *  Mirrors configStoreRoots() in the Go brain (cmd/brain/fsguard.go) — the brain
 *  is the DEFAULT answerer for fs.*, so the two lists have to be the same list. */
export function configStoreRoots(): string[] {
  const cfg = getConfigDir();
  return ['library', 'layouts', 'sessions'].map((store) => path.join(cfg, store));
}

/** Credential files denied by name wherever they resolve — a root is only as
 *  narrow as the cwds an agent runs in, and `workspacer plugin dev <dir>` puts a
 *  .bus-token inside an ordinary project. Same list as the brain's. */
export const SECRET_BASENAMES = new Set(['.bus-token', '.settings.json']);

/** Last component of an already-canonical path; '' for a bare volume prefix. */
function canonicalBasename(canonicalTarget: string): string {
  return canonicalTarget.slice(canonicalTarget.lastIndexOf(path.sep) + 1);
}

/** The one directory name that turns an ordinary write into command execution. */
export const GIT_METADATA_DIR = '.git';

/** Provider CONFIG-HOME directory names. Everything at or under one of these is
 *  refused: the whole directory is the provider's own configuration namespace,
 *  and every one of them has at least one file in it that is a command line.
 *
 *  `.opencode` holds `plugin/*.js`, which opencode LOADS AND RUNS at startup —
 *  before it prints anything, with no manifest and no other file required. That
 *  is reachable from `providers.listModels`, a capability the consent list calls
 *  "List available models". `.codex` holds `config.toml`, whose `mcp_servers`
 *  entries are `command` + `args` + `env`.
 *
 *  TWIN: agentConfigDirs in cmd/brain/fsguard.go and internal/bus/policy.go. */
export const AGENT_CONFIG_DIRS = new Set(['.opencode', '.codex']);

/** Provider config FILES, denied by basename wherever they resolve.
 *
 *  `.mcp.json` is Claude Code's project MCP-server file: each entry is a
 *  `command` + `args` the agent's CLI launches. `.claude.json` is the per-user
 *  twin of the same thing. `opencode.json`/`opencode.jsonc` carry opencode's
 *  `mcp` block, same shape.
 *
 *  TWIN: agentConfigBasenames in cmd/brain/fsguard.go and internal/bus/policy.go. */
export const AGENT_CONFIG_BASENAMES = new Set([
  '.mcp.json',
  '.claude.json',
  'opencode.json',
  'opencode.jsonc',
]);

/** The `.claude` subtree is NOT denied wholesale — `library.save` legitimately
 *  writes `.claude/skills/<id>/SKILL.md`, `.claude/agents/<id>.md` and
 *  `.claude/commands/<id>.md` through this very guard, and those are
 *  INSTRUCTIONS, which an agent still executes only through its own approvals.
 *  These three children are the different kind: they are read as POLICY and as
 *  ARGV, ahead of the approvals rather than through them.
 *
 *  TWIN: claudeConfigChildren in cmd/brain/fsguard.go and internal/bus/policy.go. */
export const CLAUDE_CONFIG_CHILDREN = new Set(['settings.json', 'settings.local.json', 'hooks']);

/** The provider config-home component `CLAUDE_CONFIG_CHILDREN` hangs off. */
export const CLAUDE_CONFIG_DIR_NAME = '.claude';

/**
 * True when an ALREADY-canonical path is agent-interpreted configuration: a file
 * a provider CLI reads as hooks, permissions, or an MCP command line rather than
 * as project data.
 *
 * This is the `.git` argument applied to the OTHER programs this host runs. A
 * `.git/config` is refused because git "discovers the repository at whatever cwd
 * it is handed and then executes whatever `.git/config` tells it to"; a
 * `<cwd>/.claude/settings.json` is the same sentence with `claude` in it. Its
 * `hooks.SessionStart[].hooks[].command` runs as the desktop user at session
 * start — before any model call, with no approval prompt, no permission mode and
 * no PreToolUse gate — and its `permissions.allow` / `permissions.defaultMode`
 * silently rewrite the approval policy for every agent started in that project,
 * including one the LOCAL user starts.
 *
 * Every guard treated these as ordinary project DATA: inside a root, not a
 * credential basename, no `.git` component. The composition that made that fatal
 * takes two calls that are each correctly confined —
 *
 *     fs.write   <agentCwd>/.claude/settings.json   (inside a live agent cwd)
 *     agents.spawn { cwd: <agentCwd> }              (unconfined BY DECISION)
 *
 * — and needs nothing else. `agents.spawn({})` with no cwd normalizes to $HOME
 * (spawnCwd.ts), which makes the home tree a root and puts `~/.claude/settings.json`
 * in reach: the same two calls then plant a hook that fires for EVERY claude
 * session on the host, in any project, until the file is removed.
 *
 * The codebase had already made this exact argument twice and stopped short of
 * the write gate both times. claudeProfiles.ts drops `configDir` from a
 * bus-written profile because "that directory supplies claude's settings.json —
 * permissions.allow and hooks, i.e. commands claude runs unprompted … there is no
 * subtree we could allow that the same caller can't also fill in", and
 * plugin/manager.go drops a plugin path scope that resolves to `/` because it
 * "granted the plugin fs.write on ~/.claude/settings.json (hooks are arbitrary
 * commands)". Both closed one door onto the file. This closes the file.
 *
 * Refused for READS as well as writes, on the `.git/config` footing: a
 * settings.json carries `apiKeyHelper`, `awsAuthRefresh` and env blocks, and a
 * `.mcp.json` carries server credentials in `env`.
 *
 * TWIN: pathIsAgentInterpretedConfig in cmd/brain/fsguard.go and
 * internal/bus/policy.go.
 */
export function isAgentInterpretedConfigPath(canonicalTarget: string): boolean {
  const comps = canonicalTarget.split(path.sep).map(asciiLower);
  if (AGENT_CONFIG_BASENAMES.has(comps[comps.length - 1] ?? '')) return true;
  for (let i = 0; i < comps.length; i++) {
    if (AGENT_CONFIG_DIRS.has(comps[i])) return true;
    // `.claude/<child>` and everything under it. The child is matched at i+1
    // only: `.claude/skills/hooks/…` is a skill named `hooks`, not the hook
    // directory, and denying it would take a legitimate library.save target out.
    if (
      comps[i] === CLAUDE_CONFIG_DIR_NAME &&
      i + 1 < comps.length &&
      CLAUDE_CONFIG_CHILDREN.has(comps[i + 1])
    ) {
      return true;
    }
  }
  return false;
}

/**
 * True when any component of an ALREADY-canonical path is the repository
 * metadata directory.
 *
 * A `.git` directory is not data, it is a program: `git` discovers the
 * repository at whatever cwd it is handed and then executes whatever
 * `.git/config` tells it to. The `-c` prefix in lib/gitExec.ts neutralizes every
 * exec-valued key with a FIXED name, but it structurally cannot cover the
 * namespaced ones — `filter.<drv>.clean` (which `git add`, i.e. git.stage, runs),
 * `diff.<drv>.command`/`textconv`, `merge.<drv>.driver`, `trailer.<t>.command` —
 * because the driver name belongs to whoever wrote the file. So the WRITE is
 * refused here instead. `.git/hooks/*`, `.git/config.worktree` and
 * `.git/info/attributes` are the same surface and the same rule covers them.
 *
 * This is HALF of the definition-site answer, not all of it: git reads the same
 * namespaced keys out of the per-user global config, which carries no `.git`
 * component at all. See isGitGlobalConfigPath for the other half.
 *
 * Reads are refused on the same footing as the credential basenames: a
 * `.git/config` routinely carries a remote URL with an embedded token and the
 * name of a credential store. Nothing legitimate reaches these guards with a
 * `.git` path — both providers already drop the entry from directory listings.
 *
 * Folded, because '.GIT' opens `.git` on APFS and NTFS and the walk hands this
 * gate the caller's spelling. The FINAL component counts too: a FILE named
 * `.git` is the "gitfile" pointer form (`gitdir: …`) and is equally a repository.
 *
 * TWIN: traversesGitDir in cmd/brain/fsguard.go and internal/bus/policy.go.
 */
export function traversesGitDir(canonicalTarget: string): boolean {
  return canonicalTarget.split(path.sep).some((c) => asciiLower(c) === GIT_METADATA_DIR);
}

/** The basename of git's per-user configuration file. Its own gate, not a
 *  SECRET_BASENAME, because the reason is different: this file is not a
 *  credential, it is a place to define a PROGRAM. */
export const GIT_GLOBAL_CONFIG_BASENAME = '.gitconfig';

/** `$XDG_CONFIG_HOME/git`, or `$HOME/.config/git` — git's other per-user
 *  configuration directory, holding `config`, `attributes` and `ignore`.
 *
 *  The `git` component is resolved WITH the base rather than appended to an
 *  already-resolved base. `~/.config/git -> ~/dotfiles/git` is the ordinary
 *  stow/chezmoi/yadm arrangement, and the target this is compared against has
 *  already been canonicalized per component — so appending an unresolved
 *  component compares a resolved path against an unresolved directory and can
 *  only miss, which took git's per-user config out of the gate entirely on every
 *  dotfiles host (basename `config`, no `.git` component, nowhere near the
 *  config dir: nothing else catches it).
 *
 *  TWIN: gitXdgConfigDir in cmd/brain/fsguard.go and internal/bus/policy.go. */
function gitXdgConfigDir(): string | null {
  const xdg = process.env.XDG_CONFIG_HOME;
  const base = xdg && path.isAbsolute(xdg) ? xdg : path.join(os.homedir(), '.config');
  return canonicalRoot(path.join(base, 'git'));
}

/**
 * True when an ALREADY-canonical path is one of git's per-user configuration
 * files — the OTHER place a namespaced exec driver can be defined.
 *
 * lib/gitExec.ts's third mechanism used to read: "Every one of those drivers has
 * to be DEFINED in a config file inside the repository's `.git` directory, and
 * [the guard] refuses every caller-supplied path that traverses a `.git`
 * component. The definition can never be written."
 *
 * That sentence was false. git reads `filter.<drv>.clean`, `diff.<drv>.textconv`,
 * `merge.<drv>.driver` and `trailer.<t>.command` out of `~/.gitconfig` and
 * `$XDG_CONFIG_HOME/git/config` exactly as it reads them out of `.git/config` —
 * and $HOME is an ordinary workspace root the moment any agent's cwd is $HOME,
 * which is not exotic: it is what `agents.spawn({})` produces, since
 * normalizeSpawnCwd('') returns the home directory. Neither file is a
 * SECRET_BASENAME, neither lives in the config dir, and neither carries a `.git`
 * component, so `fs.write` allowed both. The `* filter=drv` half of the chain is
 * an ordinary `.gitattributes` that nothing refuses (nor should it — see the
 * corpus case), so the definition site was the only thing left to close.
 *
 * The `-c` prefix cannot close it from the other side: `-c` can only SET keys,
 * and the driver name belongs to whoever wrote the file. Nor can
 * GIT_CONFIG_GLOBAL be neutralized wholesale — `core.excludesFile` in the user's
 * own global config is a legitimate part of the ignore answer that
 * `check-ignore`, `status` and `add` all depend on.
 *
 * THREE clauses, because the file moves:
 *   1. the BASENAME anywhere — `~/.gitconfig` however it is reached, folded for
 *      the same reason the credential basenames are;
 *   2. whatever `<home>/.gitconfig` RESOLVES to — a global config symlinked into
 *      a dotfiles repo (an extremely ordinary arrangement) has a canonical path
 *      that clause 1 does not match, and that repo may itself be an agent cwd;
 *   3. anything at or inside the resolved `$XDG_CONFIG_HOME/git` — `config`,
 *      `attributes` and `ignore` alike.
 *
 * Reads go with writes, on the same footing as `.git/config`: a global config
 * routinely carries credential-helper settings and `url.<base>.insteadOf`
 * rewrites with tokens in them.
 *
 * TWIN: pathIsGitGlobalConfig in cmd/brain/fsguard.go and internal/bus/policy.go.
 */
export function isGitGlobalConfigPath(canonicalTarget: string): boolean {
  if (asciiLower(canonicalBasename(canonicalTarget)) === GIT_GLOBAL_CONFIG_BASENAME) return true;
  const home = os.homedir();
  if (home) {
    try {
      if (canonicalizePath(path.join(home, GIT_GLOBAL_CONFIG_BASENAME)) === canonicalTarget) {
        return true;
      }
    } catch {
      // Unresolvable home config: nothing to compare against. The basename and
      // XDG clauses still apply.
    }
  }
  const xdgGit = gitXdgConfigDir();
  return xdgGit !== null && containsCanonicalFolded(xdgGit, canonicalTarget);
}

/**
 * Second gate, applied to every guarded path after the roots check — reads AND
 * writes, because handing a token out is a privilege promotion and overwriting
 * one is a denial of service on the whole bus.
 *
 * Narrowing the config root is not enough on its own: an agent cwd is a root
 * too, so a user who spawns an agent in `$HOME` (or `~/.config`) re-admits the
 * entire config dir through THAT root. Anything landing in the config dir
 * outside library/ layouts/ sessions/ is therefore refused here regardless of
 * which root allowed it — config.yaml, remote-token, tokens.json,
 * remote-server.json, vapid.json, workspacer.db, the Electron cookie jar,
 * plugins/** and the dir itself. The Go twin (fsguard.go) denies the same whole
 * remainder; the two are supposed to stay word for word.
 *
 * Order matters: the basename check runs FIRST and unconditionally, so a
 * credential name inside a store carve-out is still refused. The two git gates
 * (`.git/**` and the per-user global config) run next, for the same reason and
 * with the same unconditional reach: both are places to define a program git
 * will execute, and both are reachable through an ordinary agent cwd.
 */
export function isSecretPath(canonicalTarget: string): boolean {
  // Folded: '.BUS-TOKEN' opens .bus-token on macOS and Windows, and the walk
  // hands this gate the caller's spelling. See containsCanonicalFolded.
  if (SECRET_BASENAMES.has(asciiLower(canonicalBasename(canonicalTarget)))) return true;
  if (traversesGitDir(canonicalTarget)) return true;
  if (isGitGlobalConfigPath(canonicalTarget)) return true;
  // The same unconditional reach, for the same reason, aimed at the other
  // programs this host runs: a provider CLI's hooks/permissions/MCP files.
  if (isAgentInterpretedConfigPath(canonicalTarget)) return true;
  const cfg = canonicalRoot(getConfigDir());
  // An unverifiable config dir means we cannot prove the target is outside it.
  if (cfg === null) return true;
  if (!containsCanonicalFolded(cfg, canonicalTarget)) return false;
  for (const store of configStoreRoots()) {
    const sr = canonicalRoot(store);
    if (sr === null) continue;
    // A carve-out only ever NARROWS the gate, so this arm stays byte-exact
    // (folding would exempt <configDir>/LIBRARY) and the resolved carve-out must
    // still be STRICTLY inside the resolved config dir. Without that, a symlink
    // at <configDir>/library aimed at its own parent — and that is the one
    // directory a remote caller can fs.write into — resolved the carve-out to
    // the config dir itself, which contains everything in it: one symlink
    // disarmed the whole gate and handed out remote-token.
    if (sr === cfg || !containsCanonical(cfg, sr)) continue;
    if (containsCanonical(sr, canonicalTarget)) return false;
  }
  return true;
}

/**
 * True when a path a capability is ABOUT TO RETURN BYTES FROM must be dropped,
 * because `fs.read` would refuse it.
 *
 * `assertPathAllowed` answers about a path the CALLER named. This answers about
 * a path the HOST discovered while serving a call whose only caller-supplied
 * coordinate was a directory — `search.project`, which hands its cwd to ripgrep
 * and returns matching lines out of whatever the walker chose to open.
 *
 * That distinction is the whole composition. search.project applies
 * assertPathAllowed to its cwd and to nothing else, and delegates per-file
 * exclusion to ripgrep's hidden/ignore walker — whose policy is A FILE INSIDE
 * THE SEARCHED DIRECTORY. `.ignore` is an ordinary dotfile: not a credential
 * basename, no `.git` component, inside the root, so `fs.write` accepts it. Two
 * calls, each correctly confined:
 *
 *     fs.write       <root>/.ignore   with "!*\n!**\/*\n"
 *     search.project { cwd: <root> }
 *
 * and the second returns matching lines out of `<root>/.git/config` and
 * `<root>/.settings.json` — the two files the secret gate exists to refuse.
 * Bytes written as DATA by one confined call became the READ POLICY of the next.
 *
 * The durable invariant is not "make ripgrep ignore `.ignore`" (its walker has
 * several such files and their precedence is its business): it is that the set
 * of files a capability can return CONTENT from may not exceed `fs.read`'s. So
 * the gate is applied per result path, here, by the same predicate.
 *
 * Unverifiable → drop, the same posture as the guard: a path we cannot resolve
 * is a path we cannot prove is allowed.
 *
 * TWIN: resultPathIsSecret in cmd/brain/search.go.
 */
export function isSecretResultPath(target: string): boolean {
  try {
    return isSecretPath(canonicalizePath(target));
  } catch {
    return true;
  }
}

/**
 * Reject a call whose path escapes the allowed roots or lands on a credential
 * file; otherwise return the CANONICAL path, which the caller must be the one to
 * hand to the filesystem operation (re-passing the caller's raw string is the
 * check-path/opened-path split this whole module exists to close).
 *
 * One message for all three refusal reasons, matching the brain word for word:
 * it goes to a remote caller, and confirming where a denied path landed — or
 * that it hit something worth protecting — is a probe primitive.
 */
export function assertPathAllowed(cap: string, target: string, roots: string[]): string {
  const refuse = (): never => {
    throw new Error(`${cap}: path is outside the allowed workspace (agent cwds + config stores)`);
  };
  let canonical: string;
  try {
    // Exactly one resolution per request: the roots test and the secret gate
    // both read this value.
    canonical = canonicalizePath(target);
  } catch {
    return refuse(); // unverifiable → deny
  }
  if (!pathWithinRoots(roots, canonical)) return refuse();
  if (isSecretPath(canonical)) return refuse();
  return canonical;
}
