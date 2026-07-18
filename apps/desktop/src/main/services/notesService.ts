// Per-directory notes store. Notes used to be a single markdown string glued
// to the pane config (persisted with the session layout), which meant they
// vanished with the pane and couldn't be organized. Now they're first-class
// records keyed by project directory, with tags, in one JSON document under
// the config dir — same persistence pattern as remoteTokens.
import * as fs from 'fs';
import * as path from 'path';
import { randomUUID } from 'crypto';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';

export interface NoteRecord {
  id: string;
  /** Project directory the note belongs to ('' = global, no directory). */
  cwd: string;
  title: string;
  /** Markdown body. */
  content: string;
  tags: string[];
  createdAt: string;
  updatedAt: string;
}

export interface NoteInput {
  id?: string;
  cwd?: string;
  title?: string;
  content?: string;
  tags?: string[];
}

function notesPath(): string {
  return path.join(getConfigDir(), 'notes.json');
}

let cache: NoteRecord[] | null = null;

function load(): NoteRecord[] {
  if (cache) return cache;
  try {
    const raw = fs.readFileSync(notesPath(), 'utf-8');
    const doc = JSON.parse(raw) as { notes?: NoteRecord[] };
    cache = Array.isArray(doc.notes) ? doc.notes.filter((n) => n && typeof n.id === 'string') : [];
  } catch {
    cache = []; // missing / malformed file — start empty, first save recreates it
  }
  return cache;
}

function persist(notes: NoteRecord[]): void {
  cache = notes;
  const dir = getConfigDir();
  fs.mkdirSync(dir, { recursive: true });
  atomicWriteFileSync(notesPath(), `${JSON.stringify({ version: 1, notes }, null, 2)}\n`);
}

/** Normalize tags: trimmed, lowercased, deduped, empty dropped. */
function cleanTags(tags: string[] | undefined): string[] {
  const seen = new Set<string>();
  for (const t of tags ?? []) {
    const c = t.trim().toLowerCase();
    if (c) seen.add(c);
  }
  return [...seen];
}

/** Strip a trailing path separator so '/a/b' and '/a/b/' scope together. */
function cleanCwd(cwd: string | undefined): string {
  return (cwd ?? '').replace(/[/\\]+$/, '');
}

export const notesService = {
  /** Every note, newest-updated first. The renderer filters by cwd/tag. */
  listNotes(): NoteRecord[] {
    return [...load()].sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1));
  },

  /** Upsert. Unknown/absent id creates; existing id updates + bumps updatedAt. */
  saveNote(input: NoteInput): NoteRecord {
    const notes = [...load()];
    const now = new Date().toISOString();
    const existing = input.id ? notes.find((n) => n.id === input.id) : undefined;
    const record: NoteRecord = {
      id: existing?.id ?? input.id ?? randomUUID(),
      cwd: cleanCwd(input.cwd ?? existing?.cwd),
      title: (input.title ?? existing?.title ?? '').trim() || 'Untitled',
      content: input.content ?? existing?.content ?? '',
      tags: cleanTags(input.tags ?? existing?.tags),
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    };
    const idx = notes.findIndex((n) => n.id === record.id);
    if (idx >= 0) notes[idx] = record;
    else notes.push(record);
    persist(notes);
    return record;
  },

  deleteNote(id: string): void {
    const notes = load().filter((n) => n.id !== id);
    persist(notes);
  },

  /** Test hook — drop the in-memory cache so the next read hits disk. */
  _resetCache(): void {
    cache = null;
  },
};
