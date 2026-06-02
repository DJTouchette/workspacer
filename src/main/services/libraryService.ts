/**
 * The library: a store of reusable prompts + skills as plain markdown files
 * with YAML frontmatter, decoupled from Claude Code's own skills dir.
 *
 *   Global:  <configDir>/library/*.md          (e.g. ~/.config/workspacer/library)
 *   Project: <cwd>/.workspacer/library/*.md     (per repo, committable)
 *
 * Items are merged with PROJECT WINNING over global on id collision (id = the
 * filename slug). The service reads/writes the files, watches both dirs for
 * live edits, and pushes a `library:changed` event to the renderer.
 */
import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import type { BrowserWindow } from 'electron';
import { getConfigDir } from './configService';

export type LibraryScope = 'global' | 'project';
export type LibraryKind = 'prompt' | 'skill';
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
      const kind: LibraryKind = data.kind === 'skill' ? 'skill' : 'prompt';
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

  /** Merged item list, project winning over global on id collision. */
  list(cwd?: string): LibraryItem[] {
    const byId = new Map<string, LibraryItem>();
    for (const it of readDir(globalDir(), 'global')) byId.set(it.id, it);
    if (cwd) {
      for (const it of readDir(projectDir(cwd), 'project')) byId.set(it.id, it);
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

  remove(scope: LibraryScope, id: string, cwd?: string): void {
    const dir = scope === 'project' ? projectDir(cwd || process.cwd()) : globalDir();
    try { fs.unlinkSync(path.join(dir, `${slug(id)}.md`)); } catch { /* already gone */ }
  }

  // ── watching ──────────────────────────────────────────────────────────────

  private ensureProjectWatch(cwd: string): void {
    if (cwd === this.watchedProjectCwd) return;
    // Drop the old project watcher (keep the global one).
    const old = projectDir(this.watchedProjectCwd);
    const w = this.watchers.get(old);
    if (w && this.watchedProjectCwd) { w.close(); this.watchers.delete(old); }
    this.watchedProjectCwd = cwd;
    this.watch(projectDir(cwd));
  }

  private watch(dir: string): void {
    if (this.watchers.has(dir)) return;
    try {
      fs.mkdirSync(dir, { recursive: true });
      const w = fs.watch(dir, () => this.notifyChanged());
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
