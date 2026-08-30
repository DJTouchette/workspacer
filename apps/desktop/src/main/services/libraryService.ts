/**
 * The library: a store of reusable prompts + skills as plain markdown files
 * with YAML frontmatter, decoupled from Claude Code's own skills dir.
 *
 *   Global:  <configDir>/library/*.md          (e.g. ~/.config/workspacer/library)
 *   Project: <cwd>/.workspacer/library/*.md     (per repo, committable)
 *
 * It ALSO surfaces Claude Code's own assets (scope 'claude') so they can be
 * browsed/edited from the same pane, in their native on-disk format:
 *
 *   Skills: <root>/skills/<id>/SKILL.md  (frontmatter: name, description, ...)
 *   Agents: <root>/agents/<id>.md        (frontmatter: name, description, tools, model, ...)
 *   Commands: <root>/commands/<id>.md
 *
 * …for each <root> in ORIGIN order (see CLAUDE_ORIGINS), which is where the
 * bug was: only `<cwd>/.claude` was ever read. A repo with no `.claude/skills`
 * of its own therefore showed ZERO Claude items while the session had a full
 * complement of them, because the user's are in `~/.claude` and plugins ship
 * their own — none of which this ever looked at.
 *
 * Edits to claude-scoped items write back in place, preserving any frontmatter
 * keys we don't model (tools, model, metadata, ...). A plugin's assets are
 * read-only: they belong to the installed package, and rewriting one is undone
 * by the next plugin update.
 *
 * Items are merged with PROJECT WINNING over global on id collision (id = the
 * filename slug); claude items are namespaced separately and never collide.
 * The service reads/writes the files, watches the dirs for live edits, and
 * pushes a `library:changed` event to the renderer.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as yaml from 'js-yaml';
import type { BrowserWindow } from 'electron';
import { getConfigDir } from './configService';
import { slugLibrary } from '../lib/fileUtils';
import { byteCompare, trimSuffixFold } from '../lib/providerParity';
import { hasNonBlankText } from '../lib/asciiWhitespace';
import { publishToHub } from './hubClient';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { LIBRARY_KINDS, type LibraryKind } from '../shared/libraryKinds';
import { dispatchTemplateParams, type DispatchTemplateParam } from '../lib/dispatchTemplate';

export type LibraryScope = 'global' | 'project' | 'claude';
/** 'dispatch' is a Fleet Manager dispatch template: template TEXT with named
 *  placeholders plus an optional default resultSchema, rendered host-side at
 *  spawn (agents.spawn {template, templateParams} → lib/dispatchTemplate.ts).
 *  DELIBERATELY nothing else — no toolScope/cwd/model/worktree/skipPermissions
 *  fields exist on the kind, so a template is pure text with no trust boundary:
 *  every spawn argument still comes from the CALLER and passes the caller's
 *  clamps, and a template file can never smuggle one. */
export { LIBRARY_KINDS };
export type { LibraryKind };
export type LibraryAction = 'insert' | 'spawn' | 'copy';

/**
 * An MCP server definition, in Claude Code's `mcpServers` shape. A `stdio`
 * server launches a local process (`command`/`args`/`env`); an `http`/`sse`
 * server connects to a URL (`url`/`headers`). Stored in an item's `mcp:`
 * frontmatter block when kind === 'mcp'.
 */
export interface McpServerConfig {
  type?: 'stdio' | 'http' | 'sse';
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
}

/**
 * Which root a claude-scoped item lives under. Also the PRECEDENCE order Claude
 * Code itself resolves a name in, so when the same skill name exists twice the
 * library shows the copy that is actually live.
 *
 * 'plugin:<name>' items are read-only — see saveClaude/remove.
 */
export type ClaudeOrigin = 'project' | 'user' | `plugin:${string}`;

export interface LibraryItem {
  id: string; // filename slug (no extension)
  scope: LibraryScope;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  /** Default action when the item is picked. */
  action?: LibraryAction;
  /** MCP server config — present only when kind === 'mcp'. */
  mcp?: McpServerConfig;
  /** Default structured-result contract (a JSON Schema object) — present only
   *  when kind === 'dispatch'. Applied to a template spawn unless the call
   *  passes its own resultSchema. */
  resultSchema?: Record<string, unknown>;
  /** DERIVED, never read from the file: the template's placeholders parsed out
   *  of `body` — present only when kind === 'dispatch'.
   *
   *  It exists so learning what a template wants is a schema read instead of a
   *  prose read: before this, the only way to discover `{{task}}` was to fetch
   *  the WHOLE listing (every item, every body) and read the description or
   *  eyeball the markdown. Parsed by lib/dispatchTemplate's dispatchTemplateParams
   *  — the SAME parser the spawn path then enforces, so what is advertised and
   *  what is required cannot drift. Auto-filled vars ({{cwd}}) are excluded:
   *  this is what a CALLER must/may pass, not every token in the file. */
  params?: DispatchTemplateParam[];
  /** Which root a claude-scoped item came from. Absent for global/project. */
  origin?: ClaudeOrigin;
  /** False when the item's file belongs to something else (a plugin package). */
  editable?: boolean;
  body: string; // the prompt/skill text (may contain {{templates}})
  path: string; // absolute file path
}

const slug = slugLibrary;

/**
 * Validates ONE DERIVED library file — never the caller's cwd — and returns the
 * canonical path to open, or null to skip it.
 *
 * `list` and `remove` are bus-reachable, and confining their `cwd` is not the
 * same thing as confining what they touch: every read is
 * `<cwd>/.workspacer/library/<name>.md` or `<cwd>/.claude/skills/<id>/SKILL.md`,
 * composed AFTER the check and handed straight to readFileSync/rmSync. One
 * symlink planted inside the (allowed) project — writing it is an ordinary
 * permitted `fs.write` — therefore read `~/.config/workspacer/remote-token`
 * through `library.list`, which `fs.read` of the identical symlink refuses, and
 * `rm -rf`'d the config dir through `library.remove`.
 *
 * The local IPC path passes no guard: that is the desktop user working in their
 * own repos, exactly as `save` has always been. The Go twin (cmd/brain
 * library.go `libraryFileGuard`) has the same shape.
 */
export type LibraryFileGuard = (filePath: string) => string | null;

/**
 * Optional narrowing for a listing. Both fields are ANDed, and both are exact
 * matches — this is a cheap fetch-one door for a caller that already knows what
 * it wants (a Fleet Manager reading ONE dispatch template's params), not a
 * search surface.
 *
 * Applied AFTER the merge and the sort, never during it, so a filtered listing
 * is a subset of the unfiltered one — filtering must not change which item wins
 * a project-over-global id collision.
 *
 * TWIN: services/hub/cmd/brain/library.go `libraryFilter`.
 */
export interface LibraryListFilter {
  /** Exact kind match, e.g. 'dispatch'. */
  kind?: LibraryKind;
  /** Exact id match (the filename slug). Claude items are namespaced in the
   *  merge map but carry their bare id on the item, which is what matches. */
  id?: string;
}

/** Apply a LibraryListFilter. Absent/empty filter returns the list untouched,
 *  so every existing caller keeps today's answer. */
function applyLibraryFilter(items: LibraryItem[], filter?: LibraryListFilter): LibraryItem[] {
  if (!filter || (!filter.kind && !filter.id)) return items;
  return items.filter(
    (it) => (!filter.kind || it.kind === filter.kind) && (!filter.id || it.id === filter.id),
  );
}

/** The identity guard — the trusted local IPC path, and the format unit tests. */
const allowAnyLibraryFile: LibraryFileGuard = (filePath) => filePath;

/**
 * The same guard applied to a WRITE target, where a refusal must fail the whole
 * call rather than skip an item: `save` returns the path it claims to have
 * written, so silently writing nothing would hand the caller a path that never
 * received the bytes.
 *
 * It runs BEFORE mkdir, and the canonical answer is what the directory is
 * created for and what is opened — `library.save` composed
 * `<cwd>/.workspacer/library/<slug>.md` and `<cwd>/.claude/skills/<id>/SKILL.md`
 * AFTER its cwd check and handed the raw string to writeFileSync, so a symlink
 * planted in the (allowed) project by an ordinary permitted fs.write overwrote
 * `<configDir>/config.yaml` with caller-controlled body content — and
 * `updates.channel` is concatenated into the electron-updater feed URL. The Go
 * twin (cmd/brain library.go saveLibrary / saveLibraryClaude) guards the derived
 * destination exactly here, and refused what this copy allowed.
 */
function guardWriteTarget(guard: LibraryFileGuard, target: string): string {
  const full = guard(target);
  if (full === null) {
    throw new Error(
      'library.save: path is outside the allowed workspace (agent cwds + config stores)',
    );
  }
  return full;
}

/**
 * Claude-scoped ids are real on-disk basenames rather than slugs (see
 * readClaudeItems — the 1:1 map back to disk is what makes in-place edits of a
 * `My.Skill` directory work), so they reach path.join unfiltered. That is a path
 * injection point now that `library.save` / `library.remove` are bus-reachable:
 * an id of `../../..` pointed saveClaude's write — and remove's recursive rmSync
 * — anywhere on disk. Slugging is not an option here without breaking
 * non-slug-stable names, so instead require what a basename actually is: one
 * path segment, neither `.` nor `..`, no separator of either flavour (a
 * backslash is only a separator on win32, but a Windows-shaped id is never a
 * legitimate item name on any platform).
 */
/**
 * Gate on the origin a WRITE (or delete) claims. A plugin's assets are owned by
 * the installed package — editing one in place is silently reverted by the next
 * plugin update, and deleting one corrupts the install rather than the user's
 * library. Unknown origins fall back to 'project', which is where a brand-new
 * item has always gone; only an explicit 'plugin:…' is refused.
 */
function assertWritableOrigin(origin?: ClaudeOrigin): 'project' | 'user' {
  if (typeof origin === 'string' && origin.startsWith('plugin:')) {
    throw new Error(
      `library: ${origin} items are read-only — copy it into the project or your user skills to edit it`,
    );
  }
  return origin === 'user' ? 'user' : 'project';
}

function assertPlainBasename(id: string): string {
  const name = String(id ?? '');
  if (!name || name === '.' || name === '..' || /[\\/]/.test(name) || path.isAbsolute(name)) {
    throw new Error(`invalid library item id: ${id}`);
  }
  return name;
}

function globalDir(): string {
  return path.join(getConfigDir(), 'library');
}
function projectDir(cwd: string): string {
  return path.join(cwd, '.workspacer', 'library');
}
/** A root that holds Claude assets, in the `skills/ agents/ commands/` layout
 *  shared by `.claude` directories and plugin packages. */
interface ClaudeRoot {
  origin: ClaudeOrigin;
  dir: string;
  /** Whether the pane may write into it. A plugin's files are the package's. */
  editable: boolean;
}

function claudeSkillsDir(root: string): string {
  return path.join(root, 'skills');
}
function claudeAgentsDir(root: string): string {
  return path.join(root, 'agents');
}
function claudeCommandsDir(root: string): string {
  return path.join(root, 'commands');
}
/** The project's own `.claude` — the one root every write goes to by default. */
function projectClaudeDir(cwd: string): string {
  return path.join(cwd, '.claude');
}
/**
 * Claude Code's own config root. `CLAUDE_CONFIG_DIR` relocates it — the CLI
 * honours that variable and so must we, or an install that sets it shows an
 * empty user scope while the session has a full complement of skills. Resolved
 * (not tilde-expanded: `~` is an ordinary filename, per the fs guards).
 */
function userClaudeDir(): string {
  const override = process.env.CLAUDE_CONFIG_DIR;
  return override && override.trim()
    ? path.resolve(override.trim())
    : path.join(os.homedir(), '.claude');
}

/** How many plugin directories to enumerate. A marketplace clone is a checkout
 *  of arbitrary size and this runs on every list(). */
const MAX_PLUGIN_ROOTS = 200;

/** Immediate subdirectories of `dir`, sorted for a stable order. Unreadable or
 *  missing reads as empty — root discovery is best-effort. */
function subdirs(dir: string): string[] {
  try {
    return fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => path.join(dir, e.name))
      .sort();
  } catch {
    return [];
  }
}

/**
 * Installed and marketplace plugin roots under `~/.claude/plugins`:
 *
 *   cache/<marketplace>/<plugin>/<version>/   (installed)
 *   marketplaces/<mp>/plugins/<plugin>/       (the clone, whose commands are
 *                                              live without being "installed")
 *
 * Mirrors `asset_roots` in claudemon's claude_stream.rs, which resolves the same
 * layout for the Context pane — the two must agree or the same skill is
 * "built-in" in one pane and a plugin file in the other.
 */
function pluginRoots(): ClaudeRoot[] {
  const base = path.join(userClaudeDir(), 'plugins');
  const out: ClaudeRoot[] = [];
  const seen = new Set<string>();
  const push = (name: string, dir: string): void => {
    if (out.length >= MAX_PLUGIN_ROOTS || !name || seen.has(dir)) return;
    seen.add(dir);
    out.push({ origin: `plugin:${name}`, dir, editable: false });
  };
  for (const marketplace of subdirs(path.join(base, 'cache'))) {
    for (const plugin of subdirs(marketplace)) {
      // The VERSION directory is the root, not the plugin directory.
      for (const version of subdirs(plugin)) push(path.basename(plugin), version);
    }
  }
  for (const marketplace of subdirs(path.join(base, 'marketplaces'))) {
    for (const plugin of subdirs(path.join(marketplace, 'plugins'))) {
      push(path.basename(plugin), plugin);
    }
  }
  return out;
}

/** Every root a claude-scoped item can live under, in precedence order. */
function claudeRoots(cwd: string): ClaudeRoot[] {
  const roots: ClaudeRoot[] = [{ origin: 'project', dir: projectClaudeDir(cwd), editable: true }];
  const user = userClaudeDir();
  // A cwd that IS the home directory would otherwise list every user asset
  // twice, once per origin (a bare `agents.spawn({})` produces exactly that cwd).
  if (path.resolve(user) !== path.resolve(projectClaudeDir(cwd))) {
    roots.push({ origin: 'user', dir: user, editable: true });
  }
  return [...roots, ...pluginRoots()];
}

/** Where a NEW (or relocated) claude item of this origin gets written. */
function writableRootDir(origin: ClaudeOrigin | undefined, cwd: string): string {
  return origin === 'user' ? userClaudeDir() : projectClaudeDir(cwd);
}

/** Split a markdown file into its YAML frontmatter + body. */
function parseFrontmatter(raw: string): { data: Record<string, any>; body: string } {
  const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (m) {
    try {
      return { data: (yaml.load(m[1]) as Record<string, any>) ?? {}, body: m[2] };
    } catch {
      /* malformed frontmatter — treat the whole file as body */
    }
  }
  return { data: {}, body: raw };
}

/** Strip empty/undefined keys so the persisted `mcp:` block stays tidy. */
function cleanMcp(cfg: McpServerConfig): McpServerConfig {
  const out: McpServerConfig = {};
  if (cfg.type) out.type = cfg.type;
  if (cfg.command && cfg.command.trim()) out.command = cfg.command.trim();
  if (Array.isArray(cfg.args) && cfg.args.length) out.args = cfg.args.map(String);
  if (cfg.env && Object.keys(cfg.env).length) out.env = cfg.env;
  if (cfg.url && cfg.url.trim()) out.url = cfg.url.trim();
  if (cfg.headers && Object.keys(cfg.headers).length) out.headers = cfg.headers;
  return out;
}

/**
 * What a stored secret reads as once it leaves this process. Same literal as
 * the plugin-settings placeholder (`SecretPlaceholder` in
 * services/hub/internal/plugin/settings.go, `SECRET_PLACEHOLDER` in
 * renderer/src/types/plugin.ts) so workspacer has ONE convention across both of
 * its credential stores.
 */
export const SECRET_PLACEHOLDER = '__WKS_SECRET__';

/**
 * The MCP fields whose PURPOSE is credentials: `env` (a stdio server's
 * `JIRA_API_TOKEN=…`) and `headers` (an http server's `Authorization: Bearer …`).
 * Both are typed into the Library pane's MCP editor by hand and then written
 * PLAINTEXT into markdown frontmatter — under `<cwd>/.workspacer/library/` for
 * project scope, a directory this service's own header calls "per repo,
 * committable". So the risk needs no attacker: `git add -A` is enough.
 *
 * `url` is deliberately NOT redacted — it is an endpoint, it is the only thing
 * that identifies an http server in the list UI, and masking it would leave the
 * user unable to tell two servers apart. A credential belongs in `headers`, not
 * in a query string.
 */
function redactMcp(cfg: McpServerConfig): McpServerConfig {
  const mask = (rec?: Record<string, string>): Record<string, string> | undefined => {
    if (!rec) return rec;
    const out: Record<string, string> = {};
    // Keys stay visible — which variables a server needs is configuration, not
    // a secret, and the UI has to render the row to let the user replace it.
    for (const [k, v] of Object.entries(rec)) out[k] = v ? SECRET_PLACEHOLDER : v;
    return out;
  };
  return { ...cfg, env: mask(cfg.env), headers: mask(cfg.headers) };
}

/** An item as it may leave the process: MCP credentials masked, rest verbatim. */
function redactItem(item: LibraryItem): LibraryItem {
  if (item.kind !== 'mcp' || !item.mcp) return item;
  return { ...item, mcp: redactMcp(item.mcp) };
}

/**
 * Put back the real value wherever the caller echoed the placeholder — the
 * write half of the masked, write-only UI. Without this a round-trip through
 * the Library pane (open an MCP item, change its title, save) would persist the
 * literal `__WKS_SECRET__` as the token and silently break the server.
 *
 * A placeholder with nothing stored behind it is DROPPED rather than written,
 * so a caller cannot inject the sentinel as a real value.
 */
function restoreSecrets(next: McpServerConfig, stored?: McpServerConfig): McpServerConfig {
  const merge = (
    incoming?: Record<string, string>,
    prev?: Record<string, string>,
  ): Record<string, string> | undefined => {
    if (!incoming) return incoming;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(incoming)) {
      if (v !== SECRET_PLACEHOLDER) {
        out[k] = v;
        continue;
      }
      const kept = prev?.[k];
      if (kept !== undefined && kept !== SECRET_PLACEHOLDER) out[k] = kept;
    }
    return out;
  };
  return {
    ...next,
    env: merge(next.env, stored?.env),
    headers: merge(next.headers, stored?.headers),
  };
}

/** The stored MCP config at `full`, if that file exists and holds one. */
function storedMcpAt(full: string): McpServerConfig | undefined {
  try {
    const { data } = parseFrontmatter(fs.readFileSync(full, 'utf-8'));
    return data.mcp && typeof data.mcp === 'object' ? (data.mcp as McpServerConfig) : undefined;
  } catch {
    return undefined; // new file, or unreadable — nothing to preserve
  }
}

function serialize(
  item: Pick<
    LibraryItem,
    'title' | 'kind' | 'description' | 'tags' | 'action' | 'mcp' | 'resultSchema' | 'body'
  >,
): string {
  const fm: Record<string, any> = { title: item.title, kind: item.kind };
  if (item.description) fm.description = item.description;
  if (item.tags && item.tags.length) fm.tags = item.tags;
  if (item.action) fm.action = item.action;
  if (item.kind === 'mcp' && item.mcp) fm.mcp = cleanMcp(item.mcp);
  if (item.kind === 'dispatch' && item.resultSchema && Object.keys(item.resultSchema).length) {
    fm.resultSchema = item.resultSchema;
  }
  const head = yaml.dump(fm, { lineWidth: -1 }).trimEnd();
  return `---\n${head}\n---\n\n${item.body.replace(/\s+$/, '')}\n`;
}

function readDir(dir: string, scope: LibraryScope, guard: LibraryFileGuard): LibraryItem[] {
  let names: string[];
  try {
    names = fs.readdirSync(dir).filter((n) => n.toLowerCase().endsWith('.md'));
  } catch {
    return []; // dir doesn't exist yet
  }
  const items: LibraryItem[] = [];
  for (const name of names) {
    const full = guard(path.join(dir, name));
    if (full === null) continue; // a symlink out of the roots, or onto a credential
    try {
      const raw = fs.readFileSync(full, 'utf-8');
      const { data, body } = parseFrontmatter(raw);
      const id = slug(trimSuffixFold(name, '.md'));
      const kind: LibraryKind =
        data.kind === 'skill' ||
        data.kind === 'agent' ||
        data.kind === 'mcp' ||
        data.kind === 'dispatch'
          ? data.kind
          : 'prompt';
      const action: LibraryAction | undefined =
        data.action === 'insert' || data.action === 'spawn' || data.action === 'copy'
          ? data.action
          : undefined;
      const mcp =
        kind === 'mcp' && data.mcp && typeof data.mcp === 'object'
          ? cleanMcp(data.mcp as McpServerConfig)
          : undefined;
      // A dispatch item is TEXT plus this one default: any other frontmatter a
      // template file carries (a toolScope, a cwd, a model, a skipPermissions…)
      // is deliberately NOT modelled and never leaves this parser — spawn
      // arguments have no field to ride in, so a template cannot smuggle them.
      const text = body.replace(/^\s*\n/, '');
      const resultSchema =
        kind === 'dispatch' &&
        data.resultSchema &&
        typeof data.resultSchema === 'object' &&
        !Array.isArray(data.resultSchema)
          ? (data.resultSchema as Record<string, unknown>)
          : undefined;
      items.push({
        id,
        scope,
        title: typeof data.title === 'string' && hasNonBlankText(data.title) ? data.title : id,
        kind,
        description: typeof data.description === 'string' ? data.description : undefined,
        tags: Array.isArray(data.tags) ? data.tags.map(String) : undefined,
        action,
        mcp,
        resultSchema,
        params: kind === 'dispatch' ? dispatchTemplateParams(text) : undefined,
        body: text,
        path: full,
      });
    } catch {
      /* skip unreadable file */
    }
  }
  return items;
}

// ── Claude Code project assets (.claude/skills, .claude/agents) ──────────────

/** Build a LibraryItem from a Claude-format markdown file (name/description frontmatter). */
function claudeItem(
  fullPath: string,
  id: string,
  kind: 'skill' | 'agent' | 'command',
  root: ClaudeRoot,
  guard: LibraryFileGuard,
): LibraryItem | null {
  const full = guard(fullPath);
  if (full === null) return null;
  try {
    const raw = fs.readFileSync(full, 'utf-8');
    const { data, body } = parseFrontmatter(raw);
    return {
      id,
      scope: 'claude',
      title: typeof data.name === 'string' && hasNonBlankText(data.name) ? data.name : id,
      kind,
      description: typeof data.description === 'string' ? data.description : undefined,
      origin: root.origin,
      editable: root.editable,
      body: body.replace(/^\s*\n/, ''),
      path: full,
    };
  } catch {
    return null;
  }
}

/** The claude assets under ONE root. */
function readClaudeRoot(root: ClaudeRoot, guard: LibraryFileGuard): LibraryItem[] {
  const items: LibraryItem[] = [];

  // The id for a claude item is its REAL on-disk basename (skill dir name, or
  // agent/command filename sans .md), NOT a slug of it. Slugging here loses the
  // 1:1 map back to disk: two names that slug to the same id would collide in
  // list()'s Map (dropping one), and save/remove re-slugging the id would miss
  // the real path. The basename is already unique per directory.

  // Skills: one directory per skill, content in SKILL.md
  try {
    for (const e of fs.readdirSync(claudeSkillsDir(root.dir), { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      const it = claudeItem(
        path.join(claudeSkillsDir(root.dir), e.name, 'SKILL.md'),
        e.name,
        'skill',
        root,
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no skills/ under this root */
  }

  // Agents: flat markdown files
  try {
    for (const name of fs.readdirSync(claudeAgentsDir(root.dir))) {
      if (!name.toLowerCase().endsWith('.md')) continue;
      const it = claudeItem(
        path.join(claudeAgentsDir(root.dir), name),
        name.replace(/\.md$/i, ''),
        'agent',
        root,
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no agents/ under this root */
  }

  // Custom slash commands: flat markdown files. Claude command frontmatter has
  // no `name` (the file's basename is the command), so claudeItem falls back to
  // the id for the title — which is exactly what the "/" picker shows after "/".
  try {
    for (const name of fs.readdirSync(claudeCommandsDir(root.dir))) {
      if (!name.toLowerCase().endsWith('.md')) continue;
      const it = claudeItem(
        path.join(claudeCommandsDir(root.dir), name),
        name.replace(/\.md$/i, ''),
        'command',
        root,
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no commands/ under this root */
  }

  return items;
}

/**
 * Every claude asset visible to a session in `cwd`, deduped by kind+id with the
 * FIRST root winning — [`claudeRoots`] is in precedence order, so the copy the
 * library shows is the copy Claude Code would actually run.
 *
 * Only the project root is reachable from the hub bus: `guard` confines library
 * files to the caller's project plus the global store (hubCapabilities'
 * `libraryItemRoots`), so a remote caller's user and plugin items are skipped
 * here rather than read. That divergence from the local desktop path is
 * DELIBERATE — widening the item roots to `~/.claude` to close it would put
 * `~/.claude/.credentials.json` one planted symlink away from `library.list`,
 * which returns file bodies. The Go twin (cmd/brain library.go) is the same.
 */
function readClaudeItems(cwd: string, guard: LibraryFileGuard): LibraryItem[] {
  const byKey = new Map<string, LibraryItem>();
  for (const root of claudeRoots(cwd)) {
    for (const it of readClaudeRoot(root, guard)) {
      const key = `${it.kind}:${it.id}`;
      if (!byKey.has(key)) byKey.set(key, it);
    }
  }
  return Array.from(byKey.values());
}

/**
 * Serialize in Claude Code's frontmatter format (name + description first),
 * preserving any pre-existing keys we don't model (tools, model, metadata...).
 */
function serializeClaude(
  existing: Record<string, any>,
  title: string,
  description: string | undefined,
  body: string,
): string {
  const { name: _n, description: _d, ...rest } = existing;
  const fm: Record<string, any> = { name: title };
  if (description) fm.description = description;
  Object.assign(fm, rest);
  const head = yaml.dump(fm, { lineWidth: -1 }).trimEnd();
  return `---\n${head}\n---\n\n${body.replace(/\s+$/, '')}\n`;
}

/** The frontmatter+body shape [`serialize`] persists — what a starter item is. */
type SerializableItem = Parameters<typeof serialize>[0];

/**
 * The starters that shipped BEFORE `library-seeded.json` existed, and the only
 * reason that file needs a bootstrap rule at all.
 *
 * An install predating the marker has demonstrably been offered these four (the
 * old all-or-nothing seeder wrote them on its first run or not at all), so one
 * of them missing from a non-empty library means the user deleted it — and the
 * seeder must not put it back. A starter NOT in this list postdates the marker,
 * so it has never been offered to such an install and its absence means nothing.
 *
 * Frozen by definition: never add to it. A new starter belongs in `starters()`
 * only, which is exactly what makes it seed for existing users.
 * The Go twin's `preMarkerStarterIDs` is the same list.
 */
const PRE_MARKER_STARTER_IDS = [
  'summarize-and-plan',
  'careful-refactor',
  'context7-mcp',
  'make-workspacer-plugin',
];

class LibraryService {
  private win: BrowserWindow | null = null;
  private watchers = new Map<string, fs.FSWatcher>();
  private watchedProjectCwd = '';
  private debounce: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    this.seedGlobalStarters();
  }

  setMainWindow(win: BrowserWindow): void {
    this.win = win;
    this.watch(globalDir());
    this.watchUserClaude();
  }

  /**
   * The merged item list AS IT MAY LEAVE THIS PROCESS — MCP `env`/`headers`
   * values masked to [`SECRET_PLACEHOLDER`].
   *
   * Redaction is the DEFAULT, and `listWithSecrets` is the opt-in, because the
   * failure directions are not symmetric: a reader that should have been
   * redacted leaks a live API token, while a consumer that should have had
   * secrets gets a visibly broken `__WKS_SECRET__` and a bug report. A new
   * caller that thinks about neither lands on the safe one.
   *
   * Only the two spawn paths legitimately need the real values, and neither
   * routes them through the renderer: `claudeSpawn`/`managedSpawn` receive
   * `mcpItemIds` (names only) and resolve the configs here in main.
   */
  list(
    cwd?: string,
    guard: LibraryFileGuard = allowAnyLibraryFile,
    filter?: LibraryListFilter,
  ): LibraryItem[] {
    return this.listWithSecrets(cwd, guard, filter).map(redactItem);
  }

  /** Merged item list, project winning over global on id collision.
   *  Claude items (.claude/skills + .claude/agents) are namespaced separately.
   *
   *  CREDENTIALS IN THE CLEAR — for the spawn paths, which write them into a
   *  session's `--mcp-config` file. Never hand this to the renderer or the bus;
   *  see `list()`. */
  listWithSecrets(
    cwd?: string,
    guard: LibraryFileGuard = allowAnyLibraryFile,
    filter?: LibraryListFilter,
  ): LibraryItem[] {
    const byId = new Map<string, LibraryItem>();
    // The GLOBAL dir is guarded too: <configDir>/library is the one directory a
    // remote caller can write into, so a symlink planted there aimed at the
    // sibling remote-token is the shortest version of the same attack.
    for (const it of readDir(globalDir(), 'global', guard)) byId.set(it.id, it);
    if (cwd) {
      for (const it of readDir(projectDir(cwd), 'project', guard)) byId.set(it.id, it);
      for (const it of readClaudeItems(cwd, guard)) byId.set(`claude:${it.kind}:${it.id}`, it);
      // createIfMissing: false. list() is READ-ONLY by contract — it is given the
      // BROWSE roots (the whole home tree) for exactly that reason — and
      // ensureProjectWatch's default branch ran fs.mkdirSync(projectDir(cwd)),
      // composed after the cwd check and never resolved. So the one capability
      // deliberately handed the widest root set performed an unguarded derived
      // write, and a symlinked `.workspacer` component put that directory
      // outside every allowed root, in a process where fs.write to the same
      // location is refused. The Go twin (listLibrary) is ReadDir-only.
      this.ensureProjectWatch(cwd, false, false, guard);
    }
    // Byte-wise, not localeCompare: the Go twin sorts `out[i].Title < out[j].Title`
    // (raw bytes), so localeCompare made library.list come back in a different
    // order depending on which provider answered — every uppercase title before
    // every lowercase one on one side, case-insensitively interleaved on the
    // other. The ordering is what the picker shows and what "first" means in it.
    return applyLibraryFilter(
      Array.from(byId.values()).sort((a, b) => byteCompare(a.title, b.title)),
      filter,
    );
  }

  save(
    input0: {
      scope: LibraryScope;
      id?: string;
      title: string;
      kind: LibraryKind;
      description?: string;
      tags?: string[];
      action?: LibraryAction;
      mcp?: McpServerConfig;
      resultSchema?: Record<string, unknown>;
      /** Claude scope only — which root to write into ('project' | 'user'). */
      origin?: ClaudeOrigin;
      body: string;
      cwd?: string;
    },
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): LibraryItem {
    if (input0.scope === 'claude') return this.saveClaude(input0, guard);
    const id = slug(input0.id || input0.title);
    // Checked BEFORE mkdir, so a denied save leaves no directories behind, and
    // the directory is re-derived from the CANONICAL file or mkdir would
    // rebuild the unresolved one.
    const full = guardWriteTarget(
      guard,
      path.join(
        input0.scope === 'project' ? projectDir(input0.cwd || process.cwd()) : globalDir(),
        `${id}.md`,
      ),
    );
    // Resolve echoed placeholders against what is already on disk BEFORE the
    // mkdir/write, so a save that only touched the title keeps the token.
    const input =
      input0.kind === 'mcp' && input0.mcp
        ? { ...input0, mcp: restoreSecrets(input0.mcp, storedMcpAt(full)) }
        : input0;
    fs.mkdirSync(path.dirname(full), { recursive: true });
    // Atomic (temp file in the same dir + rename), like every other file-backed
    // store here and like the Go twin (cmd/brain/library.go writeFileAtomic).
    // A plain writeFileSync opens the EXISTING target with O_TRUNC, so a write
    // that dies partway — ENOSPC/EDQUOT in the field — leaves the user's saved
    // prompt truncated with the original bytes already destroyed.
    atomicWriteFileSync(full, serialize(input));
    // Redacted on the way back out, like list(): save's return value goes
    // straight to the renderer / the bus caller, so echoing the resolved token
    // would undo the masking for anyone who just saved.
    return redactItem({
      id,
      scope: input.scope,
      title: input.title,
      kind: input.kind,
      description: input.description,
      tags: input.tags,
      action: input.action,
      mcp: input.kind === 'mcp' && input.mcp ? cleanMcp(input.mcp) : undefined,
      resultSchema: input.kind === 'dispatch' ? input.resultSchema : undefined,
      // Derived here too, so the item a caller gets BACK from a save carries the
      // same params the next list() will report — an echo that disagreed with
      // the store would be worse than no echo.
      params: input.kind === 'dispatch' ? dispatchTemplateParams(input.body) : undefined,
      body: input.body,
      path: full,
    });
  }

  /** Write a claude-scoped item back in Claude Code's native format/location. */
  private saveClaude(
    input: {
      id?: string;
      title: string;
      kind: LibraryKind;
      description?: string;
      origin?: ClaudeOrigin;
      body: string;
      cwd?: string;
    },
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): LibraryItem {
    const cwd = input.cwd || process.cwd();
    const kind: 'skill' | 'agent' | 'command' =
      input.kind === 'agent' ? 'agent' : input.kind === 'command' ? 'command' : 'skill';
    // A plugin's assets are the installed package's, and the next plugin update
    // overwrites whatever we wrote. Refuse rather than write-and-lose; the pane
    // offers "copy to project" instead. Checked before the id is even derived,
    // so a plugin origin can never reach a write target.
    const origin = assertWritableOrigin(input.origin);
    // An existing item's id IS its real on-disk basename (see readClaudeItems),
    // so edit it in place; only slug when minting a brand-new item from a title.
    // A supplied id is still caller data, so it must look like a basename.
    const id = input.id ? assertPlainBasename(input.id) : slug(input.title);
    // A `.claude/skills` DIRECTORY symlink inside the (allowed) cwd is the same
    // escape library.remove had to close, in the write direction.
    const root = writableRootDir(origin, cwd);
    const full = guardWriteTarget(
      guard,
      kind === 'skill'
        ? path.join(claudeSkillsDir(root), id, 'SKILL.md')
        : kind === 'command'
          ? path.join(claudeCommandsDir(root), `${id}.md`)
          : path.join(claudeAgentsDir(root), `${id}.md`),
    );
    fs.mkdirSync(path.dirname(full), { recursive: true });

    // Preserve frontmatter keys we don't model (tools, model, metadata, ...)
    let existing: Record<string, any> = {};
    try {
      existing = parseFrontmatter(fs.readFileSync(full, 'utf-8')).data;
    } catch {
      /* new file */
    }
    // Atomic for the same reason as save() above — and it matters more here:
    // these are the user's OWN `.claude/` agents and skills, files Workspacer
    // did not author.
    atomicWriteFileSync(
      full,
      serializeClaude(existing, input.title, input.description, input.body),
    );
    // mayCreate: false — the same reason list() passes it, and the leg the fix
    // for list() missed. saveClaude writes into `.claude/…` and nothing else, so
    // it has no business mkdir'ing `<cwd>/.workspacer/library`: that path is
    // DERIVED after the guard and never resolved, so a symlinked `.workspacer`
    // component (an ordinary permitted fs.write inside the allowed cwd) put the
    // created directory — and the fs.watch installed on it, a `library:changed`
    // activity oracle — outside every allowed root, in a process where fs.write
    // to the same location is refused. The Go twin (saveLibraryClaude) creates
    // no watch directory at all.
    this.ensureProjectWatch(cwd, true, false, guard);
    return {
      id,
      scope: 'claude',
      title: input.title,
      kind,
      description: input.description,
      origin,
      editable: true,
      body: input.body,
      path: full,
    };
  }

  remove(
    scope: LibraryScope,
    id: string,
    cwd?: string,
    kind?: LibraryKind,
    origin?: ClaudeOrigin,
    // Last, like list()/save(): the guard-coverage sweeps read it positionally
    // off the end of the call, and it is the argument every one of these legs
    // has in common.
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): void {
    // The DELETE TARGET goes through the guard, not the cwd it was composed
    // from: `<cwd>/.claude/skills` is a caller-writable location inside an
    // allowed root, so a directory symlink there turned a remove of id
    // 'remote-token' into an rm -rf of the bus credential while the cwd the
    // guard saw was impeccable. rmSync does not follow the FINAL symlink but it
    // does traverse symlinked parents, so the whole derived path has to be
    // canonical before anything is unlinked.
    const unlink = (target: string, recursive: boolean): void => {
      const full = guard(target);
      if (full === null) return;
      try {
        if (recursive) fs.rmSync(full, { recursive: true, force: true });
        else fs.unlinkSync(full);
      } catch {
        /* already gone */
      }
    };
    if (scope === 'claude') {
      // Refused for a plugin origin before anything is derived — the skill
      // branch below is a recursive, force rmSync, and a plugin's skill
      // directory is part of an installed package, not the user's library.
      const root = writableRootDir(assertWritableOrigin(origin), cwd || process.cwd());
      // The id is the item's real on-disk basename (from list()); use it verbatim
      // rather than re-slugging, or a non-slug-stable name unlinks nothing — but
      // verbatim means it must first be proven to BE a basename, because the
      // skill branch below is a recursive, force rmSync.
      const name = assertPlainBasename(id);
      if (kind === 'agent') {
        unlink(path.join(claudeAgentsDir(root), `${name}.md`), false);
      } else if (kind === 'command') {
        unlink(path.join(claudeCommandsDir(root), `${name}.md`), false);
      } else {
        // A skill is a directory (SKILL.md + optional resources)
        unlink(path.join(claudeSkillsDir(root), name), true);
      }
      return;
    }
    const dir = scope === 'project' ? projectDir(cwd || process.cwd()) : globalDir();
    unlink(path.join(dir, `${slug(id)}.md`), false);
  }

  // ── watching ──────────────────────────────────────────────────────────────

  /**
   * `guard` is the SAME per-file guard the read/write legs take, and it has to
   * reach here too: every directory below is DERIVED from the caller's cwd after
   * the cwd check, and `fs.watch` follows symlinks. A `<cwd>/.claude/agents`
   * pointing anywhere on the host — an ordinary permitted write inside the
   * allowed root — turned that target into a bus-visible change ORACLE: every
   * write to it publishes `{type:'library.changed'}`, so aiming it at
   * ~/.config/workspacer tells a remote caller exactly when remote-token,
   * tokens.json and config.yaml are written, and the skills watch is recursive so
   * on macOS it covers a whole subtree. The previous pass closed the mkdir leg
   * (mayCreate:false) and left the watch on the same unresolved path.
   */
  private ensureProjectWatch(
    cwd: string,
    force = false,
    mayCreate = true,
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): void {
    if (cwd === this.watchedProjectCwd && !force) return;
    if (cwd !== this.watchedProjectCwd) {
      // Drop the old project's watchers (keep the global and user ones).
      const old = projectClaudeDir(this.watchedProjectCwd);
      for (const dir of [
        projectDir(this.watchedProjectCwd),
        claudeSkillsDir(old),
        claudeAgentsDir(old),
        claudeCommandsDir(old),
      ]) {
        const w = this.watchers.get(dir);
        if (w && this.watchedProjectCwd) {
          w.close();
          this.watchers.delete(dir);
        }
      }
      this.watchedProjectCwd = cwd;
    }
    this.watch(projectDir(cwd), { createIfMissing: mayCreate }, guard);
    // Claude dirs: watch only if they exist — don't litter repos with empty
    // .claude/skills dirs. list()/save() re-call this, so a dir created later
    // gets picked up. Skills need recursive (SKILL.md is one level down).
    const claude = projectClaudeDir(cwd);
    this.watch(claudeSkillsDir(claude), { createIfMissing: false, recursive: true }, guard);
    this.watch(claudeAgentsDir(claude), { createIfMissing: false }, guard);
    this.watch(claudeCommandsDir(claude), { createIfMissing: false }, guard);
  }

  /**
   * Watch the USER's `~/.claude` assets. Installed once, never torn down: it is
   * not derived from any caller's cwd (so it takes no guard and is no oracle),
   * and it doesn't change when the active project does. Without it, editing
   * `~/.claude/skills/foo/SKILL.md` outside the app left the pane stale — the
   * failure mode the project watch already existed to prevent.
   */
  private watchUserClaude(): void {
    const user = userClaudeDir();
    this.watch(claudeSkillsDir(user), { createIfMissing: false, recursive: true });
    this.watch(claudeAgentsDir(user), { createIfMissing: false });
    this.watch(claudeCommandsDir(user), { createIfMissing: false });
  }

  private watch(
    dir: string,
    opts: { createIfMissing?: boolean; recursive?: boolean } = {},
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): void {
    if (this.watchers.has(dir)) return;
    // BINDING DECISION 2, on a sink that is not a read or a write: resolve the
    // derived directory, refuse it if it leaves the item roots, and hand the
    // RESOLVED string to mkdir/existsSync/fs.watch. The map stays keyed by the
    // derived name so the teardown loop above (which recomputes the same derived
    // names for the previous cwd) still finds the watcher.
    const resolved = guard(dir);
    if (resolved === null) return;
    try {
      if (opts.createIfMissing === false) {
        if (!fs.existsSync(resolved)) return;
      } else {
        fs.mkdirSync(resolved, { recursive: true });
      }
      const w = fs.watch(resolved, { recursive: opts.recursive ?? false }, () =>
        this.notifyChanged(),
      );
      this.watchers.set(dir, w);
    } catch {
      /* watching is best-effort */
    }
  }

  private notifyChanged(): void {
    if (this.debounce) clearTimeout(this.debounce);
    this.debounce = setTimeout(() => {
      try {
        this.win?.webContents.send('library:changed');
      } catch {
        /* window gone */
      }
      // Mirror onto the hub bus so the web/remote client auto-refreshes too (the
      // same both-transports pattern as the fs.changed watch sink in ipc.ts).
      // No-op when remote sharing is off.
      publishToHub({ type: 'library.changed' });
    }, 150);
  }

  // ── starter items + additive seeding ────────────────────────────────────────

  /**
   * The starter library, by file id (`<id>.md`). The Go twin ships the same set
   * in the same order (cmd/brain library.go `starterItems`); the seed-count
   * tests on both sides pin that they agree.
   */
  private starters(): Array<{ id: string; item: SerializableItem }> {
    return [
      {
        id: 'summarize-and-plan',
        item: {
          title: 'Summarize & plan',
          kind: 'prompt',
          description: 'Have the agent summarize the codebase area and propose a plan.',
          tags: ['planning'],
          action: 'insert',
          body: 'Summarize how `{{cwd}}` is structured at a high level, then propose a step-by-step plan for: {{?What do you want to do?}}\n\nList the files you would touch and call out the riskiest step before writing any code.',
        },
      },
      {
        id: 'careful-refactor',
        item: {
          title: 'Careful refactor (skill)',
          kind: 'skill',
          description: 'A disciplined refactor workflow: small steps, tests between each.',
          tags: ['refactor', 'tests'],
          action: 'insert',
          body: [
            'When refactoring, follow this workflow strictly:',
            '',
            '1. First, identify the smallest safe unit to change and state it.',
            '2. Make ONE change, then run the relevant tests/build.',
            '3. Only proceed to the next change once green. Never batch unrelated edits.',
            '4. Preserve public behavior; if a signature must change, note every caller.',
            '5. At the end, summarize what changed and what you verified.',
            '',
            'Begin by mapping the change surface for: {{?Target to refactor?}}',
          ].join('\n'),
        },
      },
      {
        id: 'context7-mcp',
        item: {
          title: 'Context7 (MCP)',
          kind: 'mcp',
          description:
            'Example MCP server — up-to-date library docs. Select it at spawn to expose its tools.',
          tags: ['docs', 'example'],
          mcp: { type: 'stdio', command: 'npx', args: ['-y', '@upstash/context7-mcp'] },
          body: 'An example MCP server entry. Edit the command/args (or switch to an http URL), then pick it in the spawn dialog to load it for a session.',
        },
      },
      {
        id: 'make-workspacer-plugin',
        item: {
          title: 'Make a workspacer plugin (skill)',
          kind: 'skill',
          description:
            'Scaffold and implement a workspacer plugin (webview or sidecar) that talks the hub bus.',
          tags: ['plugin', 'dev'],
          action: 'insert',
          body: [
            'Build a workspacer plugin that talks the hub bus. Pick one kind:',
            '',
            '- webview: a pane served from ui/index.html; may use ${agentCwd}-scoped capabilities.',
            '- sidecar: a zero-dependency Node process (server.js); Node >=22 built-ins only.',
            '',
            '1) plugin.json - apiVersion MUST be exactly "1"; id is "owner.name".',
            '',
            'Sidecar:',
            '{',
            '  "id": "you.my-plugin", "name": "My Plugin", "apiVersion": "1",',
            '  "server": { "command": "node", "args": ["server.js"], "port": 9300, "health": "/health" },',
            '  "capabilities": ["agents.sendMessage", "notifications.post"],',
            '  "consumes": ["agent.state_changed"]',
            '}',
            '',
            'Webview (omit server; set ui + a pane):',
            '{',
            '  "id": "you.my-plugin", "name": "My Plugin", "apiVersion": "1", "ui": "ui",',
            '  "panes": [{ "type": "you.my-plugin", "title": "My Plugin", "scope": "both", "path": "/" }],',
            '  "capabilities": ["agents.list"], "consumes": ["agent.state_changed"]',
            '}',
            '',
            'Rules (fail-closed; undeclared is silently denied):',
            '- Only call methods in capabilities, publish types in emits, receive types in consumes.',
            '- fs.* and search.project need object form: { "method": "fs.read", "paths": ["${pluginDir}"] }.',
            '  ${agentCwd} resolves only for per-pane webview tokens; a sidecar watches files locally via Node fs.',
            '- Never hand-write .bus-token/.settings.json/.install-source/.disabled; gitignore them.',
            '',
            '2) Talk to the bus.',
            'Webview: the host auto-injects window.workspacer (no bus boilerplate). Use:',
            '  await workspacer.ready',
            '  workspacer.on(type, (data) => {})     receives only your declared consumes types',
            '  await workspacer.call(method, params)     only your declared capabilities',
            '  workspacer.publish(type, data)',
            '  workspacer.settings                      live; workspacer.onSettings(cb) for changes',
            'Sidecar: connect to ws://127.0.0.1:7895/bus?token=<t> and speak JSON frames:',
            '- {op:"subscribe", topics:[...]}          (topics allow ns.* and *)',
            '- {op:"call", id, method, params}  ->  {op:"result", id, result} or {op:"error", id, error}',
            '- {op:"publish", event:{type, source, data}}    inbound: {op:"event", event}',
            'Token: a sidecar reads env HUB_TOKEN; a webview needs no token (the SDK is wired).',
            '',
            '3) Develop with hot-reload:',
            '    workspacer plugin dev <plugin-dir>',
            'boots the backend against just this plugin and reloads it on every save.',
            '',
            'Common capabilities: agents.list, agents.sendMessage, notifications.post (params in',
            'apps/desktop/src/main/services/hubCapabilities.ts). Common events: agent.state_changed',
            '{sessionId,mode,cwd}, agent.snapshot, workflow.completed, fs.changed (after fs.watch).',
            '',
            'Full guide: the "build a plugin" page on the landing site (build-plugin.html and build-plugin.md).',
            'Working examples: the workspacer-plugins catalog (test-on-save = sidecar, cost-hud = webview).',
            '',
            'Tell me the plugin name and what it should do, and I will scaffold and implement it: {{?What should the plugin do?}}',
          ].join('\n'),
        },
      },
      // ── Dispatch templates (kind 'dispatch') — the Fleet Manager's reusable
      // dispatch boilerplate, rendered host-side at spawn via
      // agents.spawn {template, templateParams} (lib/dispatchTemplate.ts).
      // {{task}} is REQUIRED on purpose: the manager must write the
      // task-specific reasoning itself; only the framing is canned.
      {
        id: 'ship-task',
        item: {
          title: 'Ship task (dispatch)',
          kind: 'dispatch',
          description:
            'Delivery-mode boilerplate + reporting contract for a worker that changes code. Fill {{task}}; {{delivery}} defaults to opening a PR.',
          tags: ['dispatch', 'ship'],
          resultSchema: {
            type: 'object',
            required: ['commit'],
            properties: {
              commit: { type: 'string' },
              filesChanged: { type: 'array', items: { type: 'string' } },
              checksRun: { type: 'array', items: { type: 'string' } },
              caveats: { type: 'string' },
            },
          },
          body: [
            'SHIP TASK in {{cwd}}.',
            '',
            '{{task}}',
            '',
            'Ground rules:',
            '- Work only inside this repo. Never push unless the task above says to.',
            '- Deliver the result this way: {{delivery:open a pull request for the user to review; do not merge it yourself}}.',
            '- Run the project’s own checks (build, tests, lint) on what you changed before reporting, and use the repo’s code-intelligence tools (CLAUDE.md / AGENTS.md names them) instead of blind grep.',
            '- Read what a check actually printed rather than trusting its exit code, and keep track of which conclusions you proved by RUNNING something and which you only read.',
            '',
            'Do not judge whether your own work is correct. A separate reviewer does that, in a fresh session that never saw how you got here, and your last act is to hand it what it needs.',
            '',
            'So end your turn with a HANDOFF, short and factual: the task and the acceptance criteria you worked to, the architectural constraints you had to respect, the commit id and the final diff, the files a reviewer should read first, the checks you ran and what their output said, and anything you could not prove. Leave your plan and your reasoning out of it; they would anchor the reviewer on the thinking that produced the code. That final message reaches your manager automatically; do not try to message anyone, just finish.',
          ].join('\n'),
        },
      },
      {
        id: 'scout-task',
        item: {
          title: 'Scout task (dispatch)',
          kind: 'dispatch',
          description:
            'Read-only investigation framing + report-to-file contract. Fill {{task}}; {{reportPath}} defaults to a dated file under .workspacer/reports/.',
          tags: ['dispatch', 'scout'],
          resultSchema: {
            type: 'object',
            required: ['findings'],
            properties: {
              findings: { type: 'string' },
              reportPath: { type: 'string' },
              followUps: { type: 'array', items: { type: 'string' } },
            },
          },
          body: [
            'SCOUT TASK in {{cwd}} — investigate only. Do not edit source, run builds that write artifacts into the repo, or push anything.',
            '',
            '{{task}}',
            '',
            'Write your full findings to {{reportPath:.workspacer/reports/<YYYY-MM-DD>-<topic>.md}} so they outlive this session, then end your turn with a short summary: the answer, the report path, and any follow-ups you would dispatch. Your final message reaches your manager automatically; just finish.',
          ].join('\n'),
        },
      },
      {
        id: 'two-explanations',
        item: {
          title: 'Two explanations (dispatch)',
          kind: 'dispatch',
          description:
            'Diagnose-before-fixing scaffold: name two opposite explanations for a symptom and make the worker establish which holds before changing anything.',
          tags: ['dispatch', 'diagnose'],
          resultSchema: {
            type: 'object',
            required: ['verdict', 'evidence'],
            properties: {
              verdict: { type: 'string' },
              evidence: { type: 'string' },
              fix: { type: 'string' },
              caveats: { type: 'string' },
            },
          },
          body: [
            'DIAGNOSE BEFORE FIXING, in {{cwd}}.',
            '',
            'The symptom: {{symptom}}',
            '',
            'There are two opposite explanations, with opposite fixes:',
            '(A) {{explanationA}}',
            '(B) {{explanationB}}',
            '',
            'Establish WHICH ONE holds before you change anything, and say what evidence settled it. If the evidence shows neither holds, that is a SUCCESS, not a failure: report what you found and stop rather than forcing a fix. Only then apply the fix that matches the verdict: {{fixInstruction:apply the smallest fix that matches the verdict, run the relevant checks, and report}}.',
            '',
            'End your turn with the verdict, the evidence, and what you did about it. If you applied a fix, do not rule on whether it is right; hand a reviewer the diff, the files to read first and the test results, and let it decide.',
          ].join('\n'),
        },
      },
    ];
  }

  /**
   * Where "we have already offered this starter" is recorded: a small JSON file
   * beside the library dir, in the same shape as the config store's other
   * sidecars (peers.json, claude-profiles.json, tui-pins.json).
   *
   * It exists because the only other available signal — is the file on disk? —
   * cannot tell "you have never been offered this" apart from "I deleted it on
   * purpose", and the seeder must never undo the second one. The Go twin
   * (cmd/brain library.go `librarySeedStatePath`) reads and writes this same
   * file with the same key, so whichever process runs first records for both.
   */
  private seedStatePath(): string {
    return path.join(getConfigDir(), 'library-seeded.json');
  }

  /** The ids ever seeded, or null when the marker has never been written.
   *  Unreadable or malformed reads as null: re-offering the post-marker
   *  starters is recoverable, and the bootstrap below still protects the
   *  pre-marker four from being resurrected. */
  private readSeedState(): Set<string> | null {
    try {
      const raw = JSON.parse(fs.readFileSync(this.seedStatePath(), 'utf-8')) as {
        seeded?: unknown;
      };
      if (!Array.isArray(raw.seeded)) return null;
      return new Set(raw.seeded.map(String));
    } catch {
      return null;
    }
  }

  /**
   * Seed every starter that has never been seeded and is not already on disk.
   *
   * This was `seedGlobalIfEmpty`, which returned the moment the global dir held
   * ANY .md — so a starter added after a user's first run (the three dispatch
   * templates, most recently) stayed invisible forever to every existing
   * install, which is the entire installed base. Seeding is per-ITEM now.
   *
   * Two rules it must not break:
   *  - Never overwrite a file that exists — the user may have edited it.
   *  - Never resurrect one the user DELETED. That is what the marker buys: an
   *    id recorded there is never written again, however absent it is.
   *
   * A genuinely empty dir still gets the whole set, exactly as before.
   */
  private seedGlobalStarters(): void {
    const dir = globalDir();
    try {
      const recorded = this.readSeedState();
      // No marker yet: a NON-EMPTY library is a pre-marker install, so treat the
      // starters that shipped before the marker as already offered (see
      // PRE_MARKER_STARTER_IDS). An empty one is a true first run.
      const seeded =
        recorded ??
        new Set<string>(
          fs.existsSync(dir) && fs.readdirSync(dir).some((n) => n.toLowerCase().endsWith('.md'))
            ? PRE_MARKER_STARTER_IDS
            : [],
        );
      const fresh = this.starters().filter((s) => !seeded.has(s.id));
      // Idempotent fast path: every run after the first has nothing to seed and
      // nothing to record, and touches no files at all.
      if (!fresh.length && recorded) return;
      fs.mkdirSync(dir, { recursive: true });
      for (const s of fresh) {
        const full = path.join(dir, `${s.id}.md`);
        // An existing file is the user's, even when the marker has never seen
        // it. It is still RECORDED below — just never written over.
        if (fs.existsSync(full)) continue;
        fs.writeFileSync(full, serialize(s.item), 'utf-8');
      }
      // Record everything offered on this pass, written or skipped, so that
      // deleting it afterwards keeps it gone.
      for (const s of fresh) seeded.add(s.id);
      atomicWriteFileSync(
        this.seedStatePath(),
        `${JSON.stringify({ seeded: Array.from(seeded).sort() }, null, 2)}\n`,
      );
    } catch {
      /* seeding is best-effort */
    }
  }
}

export const libraryService = new LibraryService();
