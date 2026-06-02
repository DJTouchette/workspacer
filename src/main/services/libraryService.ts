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

export type LibraryScope = 'global' | 'project' | 'claude';
export type LibraryKind = 'prompt' | 'skill' | 'agent';
export type LibraryAction = 'insert' | 'spawn' | 'copy';

export interface LibraryItem {
  id: string;            // filename slug (no extension)
  scope: LibraryScope;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  /** Default action when the item is picked. */
  action?: LibraryAction;
  body: string;          // the prompt/skill text (may contain {{templates}})
  path: string;          // absolute file path
}

function slug(s: string): string {
  const out = (s || '').toLowerCase().replace(/[^a-z0-9-_]+/g, '-').replace(/^-+|-+$/g, '');
  return out || 'item';
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

function serialize(item: Pick<LibraryItem, 'title' | 'kind' | 'description' | 'tags' | 'action' | 'body'>): string {
  const fm: Record<string, any> = { title: item.title, kind: item.kind };
  if (item.description) fm.description = item.description;
  if (item.tags && item.tags.length) fm.tags = item.tags;
  if (item.action) fm.action = item.action;
  const head = yaml.dump(fm, { lineWidth: -1 }).trimEnd();
  return `---\n${head}\n---\n\n${item.body.replace(/\s+$/, '')}\n`;
}

function readDir(dir: string, scope: LibraryScope): LibraryItem[] {
  let names: string[];
  try {
    names = fs.readdirSync(dir).filter((n) => n.toLowerCase().endsWith('.md'));
  } catch {
    return []; // dir doesn't exist yet
  }
  const items: LibraryItem[] = [];
  for (const name of names) {
    const full = path.join(dir, name);
    try {
      const raw = fs.readFileSync(full, 'utf-8');
      const { data, body } = parseFrontmatter(raw);
      const id = slug(name.replace(/\.md$/i, ''));
      const kind: LibraryKind = data.kind === 'skill' || data.kind === 'agent' ? data.kind : 'prompt';
      const action: LibraryAction | undefined =
        data.action === 'insert' || data.action === 'spawn' || data.action === 'copy' ? data.action : undefined;
      items.push({
        id,
        scope,
        title: typeof data.title === 'string' && data.title.trim() ? data.title : id,
        kind,
        description: typeof data.description === 'string' ? data.description : undefined,
        tags: Array.isArray(data.tags) ? data.tags.map(String) : undefined,
        action,
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
function claudeItem(full: string, id: string, kind: 'skill' | 'agent'): LibraryItem | null {
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

function readClaudeItems(cwd: string): LibraryItem[] {
  const items: LibraryItem[] = [];

  // Skills: one directory per skill, content in SKILL.md
  try {
    for (const e of fs.readdirSync(claudeSkillsDir(cwd), { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      const it = claudeItem(path.join(claudeSkillsDir(cwd), e.name, 'SKILL.md'), slug(e.name), 'skill');
      if (it) items.push(it);
    }
  } catch { /* no .claude/skills */ }

  // Agents: flat markdown files
  try {
    for (const name of fs.readdirSync(claudeAgentsDir(cwd))) {
      if (!name.toLowerCase().endsWith('.md')) continue;
      const it = claudeItem(path.join(claudeAgentsDir(cwd), name), slug(name.replace(/\.md$/i, '')), 'agent');
      if (it) items.push(it);
    }
  } catch { /* no .claude/agents */ }

  return items;
}

/**
 * Serialize in Claude Code's frontmatter format (name + description first),
 * preserving any pre-existing keys we don't model (tools, model, metadata...).
 */
function serializeClaude(existing: Record<string, any>, title: string, description: string | undefined, body: string): string {
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
  list(cwd?: string): LibraryItem[] {
    const byId = new Map<string, LibraryItem>();
    for (const it of readDir(globalDir(), 'global')) byId.set(it.id, it);
    if (cwd) {
      for (const it of readDir(projectDir(cwd), 'project')) byId.set(it.id, it);
      for (const it of readClaudeItems(cwd)) byId.set(`claude:${it.kind}:${it.id}`, it);
      this.ensureProjectWatch(cwd);
    }
    return Array.from(byId.values()).sort((a, b) => a.title.localeCompare(b.title));
  }

  save(input: {
    scope: LibraryScope;
    id?: string;
    title: string;
    kind: LibraryKind;
    description?: string;
    tags?: string[];
    action?: LibraryAction;
    body: string;
    cwd?: string;
  }): LibraryItem {
    if (input.scope === 'claude') return this.saveClaude(input);
    const dir = input.scope === 'project'
      ? projectDir(input.cwd || process.cwd())
      : globalDir();
    fs.mkdirSync(dir, { recursive: true });
    const id = slug(input.id || input.title);
    const full = path.join(dir, `${id}.md`);
    fs.writeFileSync(full, serialize(input), 'utf-8');
    return {
      id, scope: input.scope, title: input.title, kind: input.kind,
      description: input.description, tags: input.tags, action: input.action,
      body: input.body, path: full,
    };
  }

  /** Write a claude-scoped item back in Claude Code's native format/location. */
  private saveClaude(input: {
    id?: string; title: string; kind: LibraryKind;
    description?: string; body: string; cwd?: string;
  }): LibraryItem {
    const cwd = input.cwd || process.cwd();
    const kind: 'skill' | 'agent' = input.kind === 'agent' ? 'agent' : 'skill';
    const id = slug(input.id || input.title);
    const full = kind === 'skill'
      ? path.join(claudeSkillsDir(cwd), id, 'SKILL.md')
      : path.join(claudeAgentsDir(cwd), `${id}.md`);
    fs.mkdirSync(path.dirname(full), { recursive: true });

    // Preserve frontmatter keys we don't model (tools, model, metadata, ...)
    let existing: Record<string, any> = {};
    try { existing = parseFrontmatter(fs.readFileSync(full, 'utf-8')).data; } catch { /* new file */ }
    fs.writeFileSync(full, serializeClaude(existing, input.title, input.description, input.body), 'utf-8');
    this.ensureProjectWatch(cwd, true);
    return {
      id, scope: 'claude', title: input.title, kind,
      description: input.description, body: input.body, path: full,
    };
  }

  remove(scope: LibraryScope, id: string, cwd?: string, kind?: LibraryKind): void {
    if (scope === 'claude') {
      const root = cwd || process.cwd();
      if (kind === 'agent') {
        try { fs.unlinkSync(path.join(claudeAgentsDir(root), `${slug(id)}.md`)); } catch { /* already gone */ }
      } else {
        // A skill is a directory (SKILL.md + optional resources)
        try { fs.rmSync(path.join(claudeSkillsDir(root), slug(id)), { recursive: true, force: true }); } catch { /* already gone */ }
      }
      return;
    }
    const dir = scope === 'project' ? projectDir(cwd || process.cwd()) : globalDir();
    try { fs.unlinkSync(path.join(dir, `${slug(id)}.md`)); } catch { /* already gone */ }
  }

  // ── watching ──────────────────────────────────────────────────────────────

  private ensureProjectWatch(cwd: string, force = false): void {
    if (cwd === this.watchedProjectCwd && !force) return;
    if (cwd !== this.watchedProjectCwd) {
      // Drop the old project's watchers (keep the global one).
      for (const dir of [projectDir(this.watchedProjectCwd), claudeSkillsDir(this.watchedProjectCwd), claudeAgentsDir(this.watchedProjectCwd)]) {
        const w = this.watchers.get(dir);
        if (w && this.watchedProjectCwd) { w.close(); this.watchers.delete(dir); }
      }
      this.watchedProjectCwd = cwd;
    }
    this.watch(projectDir(cwd));
    // Claude dirs: watch only if they exist — don't litter repos with empty
    // .claude/skills dirs. list()/save() re-call this, so a dir created later
    // gets picked up. Skills need recursive (SKILL.md is one level down).
    this.watch(claudeSkillsDir(cwd), { createIfMissing: false, recursive: true });
    this.watch(claudeAgentsDir(cwd), { createIfMissing: false });
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
      try { this.win?.webContents.send('library:changed'); } catch { /* window gone */ }
    }, 150);
  }

  // ── first-run seed ──────────────────────────────────────────────────────────

  private seedGlobalIfEmpty(): void {
    const dir = globalDir();
    try {
      if (fs.existsSync(dir) && fs.readdirSync(dir).some((n) => n.toLowerCase().endsWith('.md'))) return;
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(path.join(dir, 'summarize-and-plan.md'), serialize({
        title: 'Summarize & plan',
        kind: 'prompt',
        description: 'Have the agent summarize the codebase area and propose a plan.',
        tags: ['planning'],
        action: 'insert',
        body: 'Summarize how `{{cwd}}` is structured at a high level, then propose a step-by-step plan for: {{?What do you want to do?}}\n\nList the files you would touch and call out the riskiest step before writing any code.',
      }), 'utf-8');
      fs.writeFileSync(path.join(dir, 'careful-refactor.md'), serialize({
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
      }), 'utf-8');
    } catch {
      /* seeding is best-effort */
    }
  }
}

export const libraryService = new LibraryService();
