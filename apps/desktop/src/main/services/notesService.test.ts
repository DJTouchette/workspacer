import { describe, it, expect, beforeEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { notesService } from './notesService';

// getConfigDir honors XDG_CONFIG_HOME — point it at a fresh temp dir per test.
beforeEach(() => {
  process.env.XDG_CONFIG_HOME = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-notes-'));
  notesService._resetCache();
});

describe('notesService', () => {
  it('creates, updates, and deletes notes with normalized tags and cwd', () => {
    const created = notesService.saveNote({
      cwd: '/work/proj/',
      title: '  Ideas  ',
      content: '# hello',
      tags: [' Perf ', 'perf', '', 'UI'],
    });
    expect(created.cwd).toBe('/work/proj'); // trailing separator stripped
    expect(created.title).toBe('Ideas');
    expect(created.tags).toEqual(['perf', 'ui']); // trimmed, lowered, deduped

    const updated = notesService.saveNote({ id: created.id, content: 'changed' });
    expect(updated.id).toBe(created.id);
    expect(updated.content).toBe('changed');
    expect(updated.title).toBe('Ideas'); // untouched fields survive partial saves
    expect(updated.createdAt).toBe(created.createdAt);

    notesService.deleteNote(created.id);
    expect(notesService.listNotes()).toEqual([]);
  });

  it('persists to disk — a cold cache reads the same notes back', () => {
    const a = notesService.saveNote({ cwd: '/p', title: 'A', content: 'x' });
    notesService._resetCache();
    const list = notesService.listNotes();
    expect(list.map((n) => n.id)).toEqual([a.id]);
  });

  it('an empty title falls back to Untitled', () => {
    expect(notesService.saveNote({ cwd: '', title: '   ' }).title).toBe('Untitled');
  });
});
