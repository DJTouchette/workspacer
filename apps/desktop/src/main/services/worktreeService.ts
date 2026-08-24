/**
 * Git-worktree support for agent spawns: give each agent its own working tree
 * so parallel agents in one repo can't trample each other, and everything
 * scoped to the agent's cwd (plugins, watchers, checks) is confined to that
 * tree.
 *
 * Worktrees are created under `agents.worktreeRoot` (default
 * `~/.workspacer/worktrees`) as `<repoName>/<slug>` on a fresh `wks/<slug>`
 * branch cut from the repo's current HEAD. They are NEVER deleted
 * automatically — a worktree may hold uncommitted work; cleanup is a
 * deliberate `git worktree remove` (or `git worktree prune` after deleting
 * the folder).
 */

import { exec, execFile } from 'child_process';
import { gitArgs } from '../lib/gitExec';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

export interface WorktreeInfo {
  /** True when the directory is inside a git work tree. */
  isRepo: boolean;
  /** Repo top-level (the main working tree root), when isRepo. */
  root?: string;
  /** Current branch name (or short HEAD sha when detached), when isRepo. */
  branch?: string;
}

export interface WorktreeCreateResult {
  ok: boolean;
  /** Absolute path of the new worktree (the agent's cwd), when ok. */
  path?: string;
  /** The branch the worktree was created on, when ok. */
  branch?: string;
  /** What the project's configured setup commands did (absent when none). A
   *  setup failure does NOT fail creation — the worktree exists and is usable;
   *  callers surface `setup.failed` as a warning. */
  setup?: WorktreeSetupReport;
  error?: string;
}

/**
 * The slice of the app config the worktree setup hooks read. Structurally a
 * subset of configService's Config, so call sites just pass
 * `configService.getConfig()`; declared here so this service (and its tests)
 * don't depend on the config singleton.
 */
export interface WorktreeSetupSource {
  projects?: Record<string, { worktreeSetup?: string[] }>;
  scripts?: Record<string, Array<{ name: string; command: string }>>;
}

/** One worktreeSetup entry after `script:<name>` resolution. `error` set (and
 *  `command` absent) when the entry references a script that doesn't exist. */
export interface ResolvedSetupCommand {
  /** The entry exactly as configured (what reports name). */
  raw: string;
  /** The shell command to run, when resolvable. */
  command?: string;
  error?: string;
}

export interface WorktreeSetupReport {
  /** Entries that completed with exit 0, in the order they ran. */
  ran: string[];
  /** The first entry that failed (non-zero exit, timeout, unresolvable
   *  script ref), when any. Execution stops there — a failed step's
   *  successors likely depend on it, so skipping them is the deterministic
   *  choice. */
  failed?: { command: string; error: string };
  /** Entries not attempted because an earlier one failed. */
  skipped: string[];
}

export interface WorktreeRemoveResult {
  ok: boolean;
  /** The path removed, when ok. */
  removed?: string;
  /** Left in place on purpose: not an agent worktree, not a linked tree, or
   *  dirty/holding uncommitted work (git refused). Not an error to surface. */
  skipped?: boolean;
  error?: string;
}

function git(
  args: string[],
  cwd: string,
): Promise<{ ok: boolean; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    // gitArgs: see lib/gitExec.ts — .git/config is caller-writable data here.
    execFile('git', gitArgs(args), { cwd, timeout: 15_000 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: (stdout ?? '').trim(), stderr: (stderr ?? '').trim() });
    });
  });
}

/** Default parent directory for agent worktrees. */
export function defaultWorktreeRoot(): string {
  return path.join(os.homedir(), '.workspacer', 'worktrees');
}

/** Is `cwd` a git repo, and if so where/what branch? Never throws. */
export async function worktreeInfo(cwd: string): Promise<WorktreeInfo> {
  if (!cwd || !fs.existsSync(cwd)) return { isRepo: false };
  const top = await git(['rev-parse', '--show-toplevel'], cwd);
  if (!top.ok || !top.stdout) return { isRepo: false };
  const branch = await git(['rev-parse', '--abbrev-ref', 'HEAD'], cwd);
  let name = branch.ok ? branch.stdout : '';
  if (name === 'HEAD') {
    const sha = await git(['rev-parse', '--short', 'HEAD'], cwd);
    name = sha.ok ? sha.stdout : '';
  }
  return { isRepo: true, root: top.stdout, branch: name || undefined };
}

/**
 * Relative paths (posix-joined with path.join, so platform separators) of
 * `node_modules` directories anywhere under `srcRoot`, at any depth (e.g. this
 * repo's own `apps/desktop/src/renderer/node_modules`, 4 levels down — an
 * earlier depth-≤2 cap missed it and left it to be hand-symlinked). Never
 * descends into a `node_modules` directory itself (so a nested dependency's
 * own `node_modules` is never walked) or into dot-directories. Never throws.
 */
export async function discoverNodeModules(srcRoot: string): Promise<string[]> {
  const subdirs = async (dir: string): Promise<string[]> => {
    try {
      const entries = await fs.promises.readdir(dir, { withFileTypes: true });
      return entries
        .filter((e) => e.isDirectory() && e.name !== 'node_modules' && !e.name.startsWith('.'))
        .map((e) => e.name);
    } catch {
      return [];
    }
  };
  const parents: string[] = [];
  const walk = async (rel: string): Promise<void> => {
    parents.push(rel);
    for (const name of await subdirs(path.join(srcRoot, rel))) {
      await walk(rel ? path.join(rel, name) : name);
    }
  };
  await walk('');
  const found: string[] = [];
  for (const parent of parents) {
    const rel = path.join(parent, 'node_modules');
    try {
      const st = await fs.promises.stat(path.join(srcRoot, rel));
      if (st.isDirectory()) found.push(rel);
    } catch {
      /* absent — skip */
    }
  }
  return found;
}

/**
 * Best-effort: symlink the source checkout's node_modules dirs (any depth)
 * into a fresh worktree so agents come up with dependencies installed instead
 * of an `npm install`-less tree. Fail-safe on tree cleanliness: a link is kept
 * only if git ignores it IN THE WORKTREE — an untracked entry would both dirty
 * the tree and make `git worktree remove` refuse at teardown. (Note the
 * classic gotcha: a dir-only pattern like `node_modules/` does NOT match a
 * symlink; such links are rolled back here.) Returns the relative paths
 * linked. Never throws.
 */
export async function linkNodeModules(srcRoot: string, wtPath: string): Promise<string[]> {
  const linked: string[] = [];
  for (const rel of await discoverNodeModules(srcRoot)) {
    const linkPath = path.join(wtPath, rel);
    try {
      // Skip if anything already occupies the path (e.g. a committed dir).
      if (fs.existsSync(linkPath) || (await lexists(linkPath))) continue;
      // Parent must already exist in the worktree (it's a tracked dir in any
      // repo where this rel exists in HEAD); don't mkdir into a clean tree.
      if (!fs.existsSync(path.dirname(linkPath))) continue;
      const target = path.join(srcRoot, rel);
      // 'junction' only matters on Windows (no-op elsewhere): dir links
      // without elevation; target is already absolute as junctions require.
      await fs.promises.symlink(target, linkPath, 'junction');
      const ignored = await git(['check-ignore', '-q', rel.split(path.sep).join('/')], wtPath);
      if (!ignored.ok) {
        // Not gitignored → would show untracked and block teardown; undo.
        await fs.promises.unlink(linkPath).catch(() => {});
        console.log(`[worktree] not linking ${rel}: not gitignored in worktree`);
        continue;
      }
      console.log(`[worktree] linked ${rel} -> ${target}`);
      linked.push(rel);
    } catch (err) {
      console.log(`[worktree] link ${rel} failed: ${(err as Error).message}`);
    }
  }
  return linked;
}

/** Does the path exist as a directory entry (including a dangling symlink)? */
async function lexists(p: string): Promise<boolean> {
  try {
    await fs.promises.lstat(p);
    return true;
  } catch {
    return false;
  }
}

/** Per-command ceiling for worktree setup commands (an `npm ci` fits; a hung
 *  server does not get to stall agent spawns forever). */
const SETUP_COMMAND_TIMEOUT_MS = 5 * 60_000;

/**
 * The config key for a directory — mirrors the renderer's lib/projectKey
 * (backslashes normalized, trailing separators stripped; normalization, not
 * canonicalization). Duplicated because main and renderer are separate
 * compilation units with no shared lib for this yet.
 */
function projectConfigKey(cwd: string): string {
  return cwd.replace(/\\/g, '/').replace(/\/+$/, '');
}

/** Exact-key lookup with a case-insensitive fallback on case-insensitive
 *  filesystems (Windows/macOS) — the main-process twin of resolveProjectKey. */
function lookupByDir<T>(map: Record<string, T> | undefined, dir: string): T | undefined {
  if (!map) return undefined;
  const key = projectConfigKey(dir);
  if (Object.prototype.hasOwnProperty.call(map, key)) return map[key];
  if (process.platform !== 'win32' && process.platform !== 'darwin') return undefined;
  const lowered = key.toLowerCase();
  for (const existing of Object.keys(map)) {
    if (existing.toLowerCase() === lowered) return map[existing];
  }
  return undefined;
}

/**
 * The project's configured worktree setup commands, resolved. Looked up under
 * each of `dirs` in order (the spawn cwd first, then the repo root — a project
 * is usually keyed by the directory the user opens, which is usually but not
 * always the repo root); first hit wins. An entry `script:<name>` references
 * this project's `scripts` (the top-bar script buttons, keyed identically) by
 * name; an unresolvable reference is kept as an erroring entry rather than
 * silently dropped, so the miss is surfaced at run time instead of skipped.
 */
export function resolveWorktreeSetup(
  source: WorktreeSetupSource | undefined,
  dirs: string[],
): ResolvedSetupCommand[] {
  if (!source) return [];
  for (const dir of dirs) {
    const commands = lookupByDir(source.projects, dir)?.worktreeSetup;
    if (!commands?.length) continue;
    const scripts = lookupByDir(source.scripts, dir) ?? [];
    return commands
      .map((raw) => raw.trim())
      .filter(Boolean)
      .map((raw) => {
        const ref = /^script:(.+)$/.exec(raw);
        if (!ref) return { raw, command: raw };
        const script = scripts.find((s) => s.name === ref[1].trim());
        return script
          ? { raw, command: script.command }
          : { raw, error: `no script named "${ref[1].trim()}" for this project` };
      });
  }
  return [];
}

/** `$SOURCE`/`$WORKTREE` (and `${...}`) replaced textually, so the vars work
 *  even under shells that don't expand `$VAR` (cmd.exe). They are ALSO in the
 *  environment, for scripts the command invokes. */
function substituteSetupVars(command: string, ctx: { source: string; worktree: string }): string {
  return command.replace(/\$\{?SOURCE\}?/g, ctx.source).replace(/\$\{?WORKTREE\}?/g, ctx.worktree);
}

/**
 * Run a project's worktree setup commands in the new worktree: sequentially in
 * configured order, each through the platform shell with cwd = the worktree
 * root and SOURCE/WORKTREE in the environment, each output-captured to the app
 * log, each under a per-command timeout. First failure stops the remaining
 * commands (recorded as skipped) but never throws — the caller decides what a
 * failure means. Returns undefined when there is nothing to run.
 */
export async function runWorktreeSetup(
  commands: ResolvedSetupCommand[],
  ctx: { source: string; worktree: string },
  timeoutMs: number = SETUP_COMMAND_TIMEOUT_MS,
): Promise<WorktreeSetupReport | undefined> {
  if (!commands.length) return undefined;
  const ran: string[] = [];
  for (let i = 0; i < commands.length; i++) {
    const { raw, command, error } = commands[i];
    const tag = `[worktree] setup ${i + 1}/${commands.length}`;
    const fail = (why: string): WorktreeSetupReport => {
      console.error(`${tag} FAILED (${raw}): ${why} — skipping the rest`);
      return {
        ran,
        failed: { command: raw, error: why },
        skipped: commands.slice(i + 1).map((c) => c.raw),
      };
    };
    if (!command) return fail(error ?? 'unresolvable entry');
    const cmd = substituteSetupVars(command, ctx);
    console.log(`${tag}: ${cmd} (cwd ${ctx.worktree})`);
    const res = await new Promise<{ ok: boolean; error?: string; stdout: string; stderr: string }>(
      (resolve) => {
        exec(
          cmd,
          {
            cwd: ctx.worktree,
            timeout: timeoutMs,
            maxBuffer: 10 * 1024 * 1024,
            env: { ...process.env, SOURCE: ctx.source, WORKTREE: ctx.worktree },
          },
          (err, stdout, stderr) => {
            // exec's timeout kill surfaces as err.killed + the kill signal.
            const timedOut = !!err && (err as { killed?: boolean }).killed === true;
            resolve({
              ok: !err,
              error: err
                ? timedOut
                  ? `timed out after ${Math.round(timeoutMs / 1000)}s`
                  : `exit ${(err as { code?: number | string }).code ?? '?'}`
                : undefined,
              stdout: (stdout ?? '').trim(),
              stderr: (stderr ?? '').trim(),
            });
          },
        );
      },
    );
    if (res.stdout) console.log(`${tag} stdout: ${res.stdout}`);
    if (res.stderr) console.log(`${tag} stderr: ${res.stderr}`);
    if (!res.ok) return fail(res.error ?? 'failed');
    ran.push(raw);
  }
  console.log(`[worktree] setup done: ${ran.length} command(s)`);
  return { ran, skipped: [] };
}

/** Filesystem/branch-safe slug from an agent name. */
function slugify(name: string): string {
  const s = name
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^[-.]+|[-.]+$/g, '')
    .slice(0, 40);
  return s || 'agent';
}

/** A short random suffix so repeated spawns with the same name never collide. */
function shortId(): string {
  return Math.random().toString(36).slice(2, 6);
}

/**
 * Create a worktree for an agent spawn. `name` seeds the folder + branch slug
 * (typically the agent's display name); `rootOverride` is the configured
 * `agents.worktreeRoot` ('' = default); `config` supplies the project's
 * `worktreeSetup` commands (pass `configService.getConfig()`; absent = no
 * setup hooks).
 */
export async function createWorktree(opts: {
  repoCwd: string;
  name?: string;
  rootOverride?: string;
  config?: WorktreeSetupSource;
}): Promise<WorktreeCreateResult> {
  const info = await worktreeInfo(opts.repoCwd);
  if (!info.isRepo || !info.root) {
    return { ok: false, error: `${opts.repoCwd} is not inside a git repository` };
  }

  const parent = path.join(
    opts.rootOverride?.trim() ? path.resolve(opts.rootOverride.trim()) : defaultWorktreeRoot(),
    path.basename(info.root),
  );

  const slug = slugify(opts.name ?? '');
  // Prefer the clean slug; disambiguate with a short suffix on collision (an
  // existing dir OR an existing branch — git refuses both).
  for (let attempt = 0; attempt < 3; attempt++) {
    const candidate = attempt === 0 ? slug : `${slug}-${shortId()}`;
    const wtPath = path.join(parent, candidate);
    const branch = `wks/${candidate}`;
    if (fs.existsSync(wtPath)) continue;
    try {
      await fs.promises.mkdir(parent, { recursive: true });
    } catch (err) {
      return { ok: false, error: `cannot create ${parent}: ${(err as Error).message}` };
    }
    const res = await git(['worktree', 'add', '-b', branch, wtPath], info.root);
    if (res.ok) {
      console.log(`[worktree] created ${wtPath} (${branch}) from ${info.root}`);
      // Give the agent working deps without an install step; best-effort.
      await linkNodeModules(info.root, wtPath);
      // Then the project's own setup commands (deterministic, configured),
      // AFTER the auto-link so a command can replace or build on the links.
      // A setup failure is surfaced in the result, not fatal: the worktree
      // itself is fine, and the spawn paths treat worktree trouble as
      // fall-back-and-warn, never refuse-the-dispatch.
      const setup = await runWorktreeSetup(
        resolveWorktreeSetup(opts.config, [opts.repoCwd, info.root]),
        { source: info.root, worktree: wtPath },
      );
      return { ok: true, path: wtPath, branch, setup };
    }
    // Branch collision → retry with a suffix; anything else is terminal.
    if (!/already exists/i.test(res.stderr)) {
      return { ok: false, error: res.stderr || 'git worktree add failed' };
    }
  }
  return { ok: false, error: 'could not find a free worktree name (tried 3 candidates)' };
}

/**
 * Tear down an AGENT worktree (a dispatched worker's disposable tree) when the
 * worker is gone. Fail-closed by construction and never destructive to real
 * work:
 *   - only ever touches a path UNDER the agent worktree root, so a real
 *     checkout can never be named here by accident;
 *   - only a LINKED worktree (git-dir ≠ common-dir), never a primary checkout;
 *   - `git worktree remove` WITHOUT --force, so git itself refuses a dirty tree
 *     (uncommitted work is left for the user), and the BRANCH is kept — the
 *     worker's committed work / open PR survives; only the working copy goes.
 * A non-agent path, a non-worktree, or a dirty tree returns `skipped`, not an
 * error.
 */
export async function removeAgentWorktree(opts: {
  cwd: string;
  rootOverride?: string;
}): Promise<WorktreeRemoveResult> {
  const cwd = opts.cwd;
  if (!cwd || !fs.existsSync(cwd)) return { ok: false, skipped: true };
  // Guard 1: must live under the agent worktree root (mirrors createWorktree).
  const root = opts.rootOverride?.trim()
    ? path.resolve(opts.rootOverride.trim())
    : defaultWorktreeRoot();
  const rel = path.relative(root, path.resolve(cwd));
  if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) {
    return { ok: false, skipped: true }; // not an agent worktree — never touch it
  }
  // Guard 2: must be a LINKED worktree, not the repo's primary checkout. For a
  // linked tree the per-worktree git-dir differs from the shared common-dir;
  // for the main tree they are equal.
  const common = await git(['rev-parse', '--path-format=absolute', '--git-common-dir'], cwd);
  const own = await git(['rev-parse', '--path-format=absolute', '--git-dir'], cwd);
  if (!common.ok || !own.ok || !common.stdout || common.stdout === own.stdout) {
    return { ok: false, skipped: true };
  }
  const mainRoot = path.dirname(common.stdout); // <repo>/.git → <repo>
  const res = await git(['worktree', 'remove', cwd], mainRoot);
  if (!res.ok) {
    // The common refusal is "contains modified or untracked files" — leave it.
    if (/modified|untracked|locked|dirty/i.test(res.stderr)) {
      return { ok: false, skipped: true, error: res.stderr };
    }
    return { ok: false, error: res.stderr || 'git worktree remove failed' };
  }
  await git(['worktree', 'prune'], mainRoot);
  console.log(`[worktree] removed agent worktree ${cwd}`);
  return { ok: true, removed: cwd };
}
