/**
 * The library: a store of reusable prompts + skills as plain markdown files
 * with YAML frontmatter, decoupled from Claude Code's own skills dir.
 *
 *   Global:  <configDir>/library/*.md          (e.g. ~/.config/workspacer/library)
 *   Project: <cwd>/.workspacer/library/*.md     (per repo, committable)
 *
 * It ALSO surfaces the project's Claude Code assets (scope 'claude') so they
 * can be browsed/edited from the same pane, in their native on-disk format:
 *
 *   Skills: <cwd>/.claude/skills/<id>/SKILL.md  (frontmatter: name, description, ...)
 *   Agents: <cwd>/.claude/agents/<id>.md        (frontmatter: name, description, tools, model, ...)
 *
 * Edits to claude-scoped items write back in place, preserving any frontmatter
 * keys we don't model (tools, model, metadata, ...).
 *
 * Items are merged with PROJECT WINNING over global on id collision (id = the
 * filename slug); claude items are namespaced separately and never collide.
 * The service reads/writes the files, watches the dirs for live edits, and
 * pushes a `library:changed` event to the renderer.
 */
import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import type { BrowserWindow } from 'electron';
import { getConfigDir } from './configService';
import { slugLibrary } from '../lib/fileUtils';
import { byteCompare, trimSuffixFold } from '../lib/providerParity';
import { publishToHub } from './hubClient';

export type LibraryScope = 'global' | 'project' | 'claude';
export type LibraryKind = 'prompt' | 'skill' | 'agent' | 'mcp' | 'command';
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
function claudeSkillsDir(cwd: string): string {
  return path.join(cwd, '.claude', 'skills');
}
function claudeAgentsDir(cwd: string): string {
  return path.join(cwd, '.claude', 'agents');
}
function claudeCommandsDir(cwd: string): string {
  return path.join(cwd, '.claude', 'commands');
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

function serialize(
  item: Pick<LibraryItem, 'title' | 'kind' | 'description' | 'tags' | 'action' | 'mcp' | 'body'>,
): string {
  const fm: Record<string, any> = { title: item.title, kind: item.kind };
  if (item.description) fm.description = item.description;
  if (item.tags && item.tags.length) fm.tags = item.tags;
  if (item.action) fm.action = item.action;
  if (item.kind === 'mcp' && item.mcp) fm.mcp = cleanMcp(item.mcp);
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
        data.kind === 'skill' || data.kind === 'agent' || data.kind === 'mcp'
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
      items.push({
        id,
        scope,
        title: typeof data.title === 'string' && data.title.trim() ? data.title : id,
        kind,
        description: typeof data.description === 'string' ? data.description : undefined,
        tags: Array.isArray(data.tags) ? data.tags.map(String) : undefined,
        action,
        mcp,
        body: body.replace(/^\s*\n/, ''),
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
      title: typeof data.name === 'string' && data.name.trim() ? data.name : id,
      kind,
      description: typeof data.description === 'string' ? data.description : undefined,
      body: body.replace(/^\s*\n/, ''),
      path: full,
    };
  } catch {
    return null;
  }
}

function readClaudeItems(cwd: string, guard: LibraryFileGuard): LibraryItem[] {
  const items: LibraryItem[] = [];

  // The id for a claude item is its REAL on-disk basename (skill dir name, or
  // agent/command filename sans .md), NOT a slug of it. Slugging here loses the
  // 1:1 map back to disk: two names that slug to the same id would collide in
  // list()'s Map (dropping one), and save/remove re-slugging the id would miss
  // the real path. The basename is already unique per directory.

  // Skills: one directory per skill, content in SKILL.md
  try {
    for (const e of fs.readdirSync(claudeSkillsDir(cwd), { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      const it = claudeItem(
        path.join(claudeSkillsDir(cwd), e.name, 'SKILL.md'),
        e.name,
        'skill',
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no .claude/skills */
  }

  // Agents: flat markdown files
  try {
    for (const name of fs.readdirSync(claudeAgentsDir(cwd))) {
      if (!name.toLowerCase().endsWith('.md')) continue;
      const it = claudeItem(
        path.join(claudeAgentsDir(cwd), name),
        name.replace(/\.md$/i, ''),
        'agent',
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no .claude/agents */
  }

  // Custom slash commands: flat markdown files. Claude command frontmatter has
  // no `name` (the file's basename is the command), so claudeItem falls back to
  // the id for the title — which is exactly what the "/" picker shows after "/".
  try {
    for (const name of fs.readdirSync(claudeCommandsDir(cwd))) {
      if (!name.toLowerCase().endsWith('.md')) continue;
      const it = claudeItem(
        path.join(claudeCommandsDir(cwd), name),
        name.replace(/\.md$/i, ''),
        'command',
        guard,
      );
      if (it) items.push(it);
    }
  } catch {
    /* no .claude/commands */
  }

  return items;
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

class LibraryService {
  private win: BrowserWindow | null = null;
  private watchers = new Map<string, fs.FSWatcher>();
  private watchedProjectCwd = '';
  private debounce: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    this.seedGlobalIfEmpty();
  }

  setMainWindow(win: BrowserWindow): void {
    this.win = win;
    this.watch(globalDir());
  }

  /** Merged item list, project winning over global on id collision.
   *  Claude items (.claude/skills + .claude/agents) are namespaced separately. */
  list(cwd?: string, guard: LibraryFileGuard = allowAnyLibraryFile): LibraryItem[] {
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
      this.ensureProjectWatch(cwd, false, false);
    }
    // Byte-wise, not localeCompare: the Go twin sorts `out[i].Title < out[j].Title`
    // (raw bytes), so localeCompare made library.list come back in a different
    // order depending on which provider answered — every uppercase title before
    // every lowercase one on one side, case-insensitively interleaved on the
    // other. The ordering is what the picker shows and what "first" means in it.
    return Array.from(byId.values()).sort((a, b) => byteCompare(a.title, b.title));
  }

  save(
    input: {
      scope: LibraryScope;
      id?: string;
      title: string;
      kind: LibraryKind;
      description?: string;
      tags?: string[];
      action?: LibraryAction;
      mcp?: McpServerConfig;
      body: string;
      cwd?: string;
    },
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): LibraryItem {
    if (input.scope === 'claude') return this.saveClaude(input, guard);
    const id = slug(input.id || input.title);
    // Checked BEFORE mkdir, so a denied save leaves no directories behind, and
    // the directory is re-derived from the CANONICAL file or mkdir would
    // rebuild the unresolved one.
    const full = guardWriteTarget(
      guard,
      path.join(
        input.scope === 'project' ? projectDir(input.cwd || process.cwd()) : globalDir(),
        `${id}.md`,
      ),
    );
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, serialize(input), 'utf-8');
    return {
      id,
      scope: input.scope,
      title: input.title,
      kind: input.kind,
      description: input.description,
      tags: input.tags,
      action: input.action,
      mcp: input.kind === 'mcp' && input.mcp ? cleanMcp(input.mcp) : undefined,
      body: input.body,
      path: full,
    };
  }

  /** Write a claude-scoped item back in Claude Code's native format/location. */
  private saveClaude(
    input: {
      id?: string;
      title: string;
      kind: LibraryKind;
      description?: string;
      body: string;
      cwd?: string;
    },
    guard: LibraryFileGuard = allowAnyLibraryFile,
  ): LibraryItem {
    const cwd = input.cwd || process.cwd();
    const kind: 'skill' | 'agent' | 'command' =
      input.kind === 'agent' ? 'agent' : input.kind === 'command' ? 'command' : 'skill';
    // An existing item's id IS its real on-disk basename (see readClaudeItems),
    // so edit it in place; only slug when minting a brand-new item from a title.
    // A supplied id is still caller data, so it must look like a basename.
    const id = input.id ? assertPlainBasename(input.id) : slug(input.title);
    // A `.claude/skills` DIRECTORY symlink inside the (allowed) cwd is the same
    // escape library.remove had to close, in the write direction.
    const full = guardWriteTarget(
      guard,
      kind === 'skill'
        ? path.join(claudeSkillsDir(cwd), id, 'SKILL.md')
        : kind === 'command'
          ? path.join(claudeCommandsDir(cwd), `${id}.md`)
          : path.join(claudeAgentsDir(cwd), `${id}.md`),
    );
    fs.mkdirSync(path.dirname(full), { recursive: true });

    // Preserve frontmatter keys we don't model (tools, model, metadata, ...)
    let existing: Record<string, any> = {};
    try {
      existing = parseFrontmatter(fs.readFileSync(full, 'utf-8')).data;
    } catch {
      /* new file */
    }
    fs.writeFileSync(
      full,
      serializeClaude(existing, input.title, input.description, input.body),
      'utf-8',
    );
    this.ensureProjectWatch(cwd, true);
    return {
      id,
      scope: 'claude',
      title: input.title,
      kind,
      description: input.description,
      body: input.body,
      path: full,
    };
  }

  remove(
    scope: LibraryScope,
    id: string,
    cwd?: string,
    kind?: LibraryKind,
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
      const root = cwd || process.cwd();
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

  private ensureProjectWatch(cwd: string, force = false, mayCreate = true): void {
    if (cwd === this.watchedProjectCwd && !force) return;
    if (cwd !== this.watchedProjectCwd) {
      // Drop the old project's watchers (keep the global one).
      for (const dir of [
        projectDir(this.watchedProjectCwd),
        claudeSkillsDir(this.watchedProjectCwd),
        claudeAgentsDir(this.watchedProjectCwd),
        claudeCommandsDir(this.watchedProjectCwd),
      ]) {
        const w = this.watchers.get(dir);
        if (w && this.watchedProjectCwd) {
          w.close();
          this.watchers.delete(dir);
        }
      }
      this.watchedProjectCwd = cwd;
    }
    this.watch(projectDir(cwd), { createIfMissing: mayCreate });
    // Claude dirs: watch only if they exist — don't litter repos with empty
    // .claude/skills dirs. list()/save() re-call this, so a dir created later
    // gets picked up. Skills need recursive (SKILL.md is one level down).
    this.watch(claudeSkillsDir(cwd), { createIfMissing: false, recursive: true });
    this.watch(claudeAgentsDir(cwd), { createIfMissing: false });
    this.watch(claudeCommandsDir(cwd), { createIfMissing: false });
  }

  private watch(dir: string, opts: { createIfMissing?: boolean; recursive?: boolean } = {}): void {
    if (this.watchers.has(dir)) return;
    try {
      if (opts.createIfMissing === false) {
        if (!fs.existsSync(dir)) return;
      } else {
        fs.mkdirSync(dir, { recursive: true });
      }
      const w = fs.watch(dir, { recursive: opts.recursive ?? false }, () => this.notifyChanged());
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

  // ── first-run seed ──────────────────────────────────────────────────────────

  private seedGlobalIfEmpty(): void {
    const dir = globalDir();
    try {
      if (fs.existsSync(dir) && fs.readdirSync(dir).some((n) => n.toLowerCase().endsWith('.md')))
        return;
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(
        path.join(dir, 'summarize-and-plan.md'),
        serialize({
          title: 'Summarize & plan',
          kind: 'prompt',
          description: 'Have the agent summarize the codebase area and propose a plan.',
          tags: ['planning'],
          action: 'insert',
          body: 'Summarize how `{{cwd}}` is structured at a high level, then propose a step-by-step plan for: {{?What do you want to do?}}\n\nList the files you would touch and call out the riskiest step before writing any code.',
        }),
        'utf-8',
      );
      fs.writeFileSync(
        path.join(dir, 'careful-refactor.md'),
        serialize({
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
        }),
        'utf-8',
      );
      fs.writeFileSync(
        path.join(dir, 'context7-mcp.md'),
        serialize({
          title: 'Context7 (MCP)',
          kind: 'mcp',
          description:
            'Example MCP server — up-to-date library docs. Select it at spawn to expose its tools.',
          tags: ['docs', 'example'],
          mcp: { type: 'stdio', command: 'npx', args: ['-y', '@upstash/context7-mcp'] },
          body: 'An example MCP server entry. Edit the command/args (or switch to an http URL), then pick it in the spawn dialog to load it for a session.',
        }),
        'utf-8',
      );
      fs.writeFileSync(
        path.join(dir, 'make-workspacer-plugin.md'),
        serialize({
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
        }),
        'utf-8',
      );
    } catch {
      /* seeding is best-effort */
    }
  }
}

export const libraryService = new LibraryService();
