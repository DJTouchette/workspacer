/**
 * NotesPane — per-directory notes with tags.
 *
 * Pins: (1) only the pane's directory's notes are listed, (2) the tag filter
 * narrows the list, (3) legacy pane-config content migrates into the store
 * once and the pane copy is cleared.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import NotesPane from '../../src/panes/NotesPane';
import type { NoteRecord } from '../../src/types/electron';

const note = (over: Partial<NoteRecord>): NoteRecord => ({
  id: Math.random().toString(36).slice(2),
  cwd: '/proj',
  title: 'Untitled',
  content: '',
  tags: [],
  createdAt: '2026-07-17T00:00:00Z',
  updatedAt: '2026-07-17T00:00:00Z',
  ...over,
});

let store: NoteRecord[];
let notesSave: ReturnType<typeof vi.fn>;

beforeEach(() => {
  store = [];
  notesSave = vi.fn().mockImplementation(async (input: Partial<NoteRecord>) => {
    const rec = note({ ...input, id: input.id ?? `n${store.length + 1}` } as Partial<NoteRecord>);
    const i = store.findIndex((n) => n.id === rec.id);
    if (i >= 0) store[i] = rec;
    else store.push(rec);
    return rec;
  });
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    notesList: vi.fn().mockImplementation(async () => [...store]),
    notesSave,
    notesDelete: vi.fn().mockImplementation(async (id: string) => {
      store = store.filter((n) => n.id !== id);
    }),
  };
});

describe('NotesPane', () => {
  it('lists only the current directory’s notes', async () => {
    store = [
      note({ id: 'a', cwd: '/proj', title: 'Mine' }),
      note({ id: 'b', cwd: '/other', title: 'Elsewhere' }),
    ];
    render(<NotesPane title="Notes" cwd="/proj" />);
    await screen.findAllByText('Mine');
    expect(screen.queryByText('Elsewhere')).toBeNull();
  });

  it('tag chips filter the list', async () => {
    store = [
      note({ id: 'a', cwd: '/proj', title: 'Perf ideas', tags: ['perf'] }),
      note({ id: 'b', cwd: '/proj', title: 'UI polish', tags: ['ui'] }),
    ];
    render(<NotesPane title="Notes" cwd="/proj" />);
    await screen.findByText('UI polish');
    // Click the sidebar filter chip for 'perf' (first occurrence is the filter row).
    fireEvent.click(screen.getAllByText('perf')[0]);
    await waitFor(() => expect(screen.queryByText('UI polish')).toBeNull());
    expect(screen.getAllByText('Perf ideas').length).toBeGreaterThan(0);
  });

  it('migrates legacy pane content into a real note and clears the pane copy', async () => {
    const onNotesChange = vi.fn();
    render(
      <NotesPane
        title="Notes"
        cwd="/proj"
        notes="# legacy scratchpad"
        onNotesChange={onNotesChange}
      />,
    );
    await waitFor(() =>
      expect(notesSave).toHaveBeenCalledWith(
        expect.objectContaining({ cwd: '/proj', content: '# legacy scratchpad' }),
      ),
    );
    expect(onNotesChange).toHaveBeenCalledWith('');
  });
});
