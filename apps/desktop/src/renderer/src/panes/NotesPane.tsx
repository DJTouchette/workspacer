/**
 * NotesPane — per-directory markdown notes with tags.
 *
 * Notes are first-class records in the host store (main/notesService — one
 * JSON doc in the config dir), keyed by project directory, so they survive
 * pane/session/layout changes and every agent in the same project sees the
 * same notebook. The pane shows the current directory's notes with a list
 * sidebar (search + tag filter), and a Write / Split / Preview markdown
 * editor reusing the shared lightweight renderer.
 *
 * Legacy migration: notes used to be a single string on the PaneConfig
 * (persisted with the session). On first mount with legacy content, it's
 * folded into a real note for this directory and cleared from the pane.
 */
import React, { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { parseMarkdownBlocks } from '../components/markdown';
import type { NoteRecord } from '../types/electron';

type View = 'write' | 'split' | 'preview';

const VIEW_KEY = 'wksNotesView';
const PERSIST_DEBOUNCE_MS = 500;

interface NotesPaneProps {
  title: string;
  /** Project directory scoping this pane's notes ('' / undefined = global). */
  cwd?: string;
  /** Legacy pane-config content, migrated into the store on first mount. */
  notes?: string;
  /** Clears the legacy pane copy once migrated. */
  onNotesChange?: (notes: string) => void;
}

function relTime(iso: string): string {
  const ms = Date.now() - Date.parse(iso);
  const min = Math.floor(ms / 60000);
  if (min < 1) return 'now';
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** Tag chip; interactive when onClick is given (filter toggles). */
const TagChip: React.FC<{ tag: string; active?: boolean; onClick?: () => void }> = ({
  tag,
  active,
  onClick,
}) => (
  <span
    onClick={onClick}
    style={{
      fontSize: '0.62rem',
      fontWeight: 600,
      padding: '1px 7px',
      borderRadius: 'var(--wks-radius-pill, 999px)',
      cursor: onClick ? 'pointer' : 'default',
      whiteSpace: 'nowrap',
      color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-muted)',
      border: `1px solid ${
        active
          ? 'color-mix(in srgb, var(--wks-accent) 45%, transparent)'
          : 'var(--wks-border-subtle)'
      }`,
      background: active ? 'color-mix(in srgb, var(--wks-accent) 12%, transparent)' : 'transparent',
    }}
  >
    {tag}
  </span>
);

const NotesPane: React.FC<NotesPaneProps> = ({ cwd, notes: legacyNotes, onNotesChange }) => {
  const scope = (cwd ?? '').replace(/[/\\]+$/, '');
  const [all, setAll] = useState<NoteRecord[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [tagFilter, setTagFilter] = useState<string | null>(null);
  const [view, setViewState] = useState<View>(() => {
    const saved = localStorage.getItem(VIEW_KEY);
    return saved === 'write' || saved === 'preview' ? saved : 'split';
  });
  const setView = (v: View) => {
    setViewState(v);
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {}
  };

  // ── Draft state for the selected note (snappy typing; store writes debounced) ──
  const [draftTitle, setDraftTitle] = useState('');
  const [draftTags, setDraftTags] = useState('');
  const [draftContent, setDraftContent] = useState('');
  const saveTimer = useRef<ReturnType<typeof setTimeout>>();
  const draftRef = useRef({ id: null as string | null, title: '', tags: '', content: '' });

  const refresh = useCallback(async () => {
    try {
      const list = (await window.electronAPI.notesList?.()) ?? [];
      setAll(list);
      return list;
    } catch {
      setAll([]);
      return [] as NoteRecord[];
    }
  }, []);

  // Initial load + legacy pane-string migration (once).
  const migratedRef = useRef(false);
  useEffect(() => {
    void (async () => {
      await refresh();
      if (!migratedRef.current && legacyNotes?.trim() && window.electronAPI.notesSave) {
        migratedRef.current = true;
        // Fold the old pane-scoped scratchpad into a real note for this dir.
        await window.electronAPI.notesSave({
          cwd: scope,
          title: 'Notes',
          content: legacyNotes,
          tags: [],
        });
        onNotesChange?.(''); // clear the pane copy so it never re-migrates
        await refresh();
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const dirNotes = useMemo(() => (all ?? []).filter((n) => n.cwd === scope), [all, scope]);
  const allTags = useMemo(() => [...new Set(dirNotes.flatMap((n) => n.tags))].sort(), [dirNotes]);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return dirNotes.filter(
      (n) =>
        (!tagFilter || n.tags.includes(tagFilter)) &&
        (!q ||
          n.title.toLowerCase().includes(q) ||
          n.content.toLowerCase().includes(q) ||
          n.tags.some((t) => t.includes(q))),
    );
  }, [dirNotes, query, tagFilter]);

  const selected = useMemo(
    () => dirNotes.find((n) => n.id === selectedId) ?? null,
    [dirNotes, selectedId],
  );

  // Load the selected note into the drafts (and pick a default selection).
  useEffect(() => {
    if (selected) {
      if (draftRef.current.id !== selected.id) {
        draftRef.current = {
          id: selected.id,
          title: selected.title,
          tags: selected.tags.join(', '),
          content: selected.content,
        };
        setDraftTitle(selected.title);
        setDraftTags(selected.tags.join(', '));
        setDraftContent(selected.content);
      }
    } else if (dirNotes.length > 0 && selectedId === null) {
      setSelectedId(dirNotes[0].id);
    }
  }, [selected, dirNotes, selectedId]);

  const flushSave = useCallback(() => {
    const d = draftRef.current;
    if (!d.id) return;
    void window.electronAPI
      .notesSave?.({
        id: d.id,
        cwd: scope,
        title: d.title,
        content: d.content,
        tags: d.tags
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean),
      })
      .then(() => refresh());
  }, [scope, refresh]);

  const scheduleSave = useCallback(
    (patch: Partial<{ title: string; tags: string; content: string }>) => {
      draftRef.current = { ...draftRef.current, ...patch };
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(flushSave, PERSIST_DEBOUNCE_MS);
    },
    [flushSave],
  );

  // Flush pending edits on unmount so nothing is lost.
  useEffect(
    () => () => {
      if (saveTimer.current) {
        clearTimeout(saveTimer.current);
        flushSave();
      }
    },
    [flushSave],
  );

  const createNote = async () => {
    const rec = await window.electronAPI.notesSave?.({
      cwd: scope,
      title: 'Untitled',
      content: '',
      tags: tagFilter ? [tagFilter] : [],
    });
    await refresh();
    if (rec) setSelectedId(rec.id);
  };

  const deleteNote = async (id: string) => {
    await window.electronAPI.notesDelete?.(id);
    if (selectedId === id) {
      setSelectedId(null);
      draftRef.current = { id: null, title: '', tags: '', content: '' };
    }
    await refresh();
  };

  const preview = useMemo(() => parseMarkdownBlocks(draftContent), [draftContent]);

  // Tab inserts two spaces instead of moving focus out of the editor.
  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== 'Tab') return;
    e.preventDefault();
    const ta = e.currentTarget;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const next = draftContent.slice(0, start) + '  ' + draftContent.slice(end);
    setDraftContent(next);
    scheduleSave({ content: next });
    requestAnimationFrame(() => {
      ta.selectionStart = ta.selectionEnd = start + 2;
    });
  };

  const showWrite = view !== 'preview';
  const showPreview = view !== 'write';
  const dirLabel = scope ? scope.split(/[/\\]/).pop() : 'Global';

  return (
    <div style={{ display: 'flex', height: '100%', background: 'var(--wks-bg-base)' }}>
      {/* ── Note list sidebar ── */}
      <div
        style={{
          width: 220,
          flexShrink: 0,
          display: 'flex',
          flexDirection: 'column',
          borderRight: '1px solid var(--wks-border-subtle)',
          background: 'var(--wks-bg-raised)',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            padding: '8px 10px 4px',
          }}
        >
          <span
            title={scope || 'Notes without a project directory'}
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.72rem',
              fontWeight: 650,
              color: 'var(--wks-text-primary)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {dirLabel}
          </span>
          <button
            onClick={() => void createNote()}
            title="New note"
            style={{
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 700,
              cursor: 'pointer',
              padding: '1px 8px',
              borderRadius: 5,
              border: '1px solid var(--wks-border-subtle)',
              background: 'transparent',
              color: 'var(--wks-accent-text)',
            }}
          >
            ＋
          </button>
        </div>
        <div style={{ padding: '2px 10px 6px' }}>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search notes…"
            style={{
              width: '100%',
              boxSizing: 'border-box',
              padding: '4px 8px',
              fontSize: '0.7rem',
              fontFamily: 'inherit',
              backgroundColor: 'var(--wks-bg-input)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-subtle)',
              borderRadius: 5,
              outline: 'none',
            }}
          />
        </div>
        {allTags.length > 0 && (
          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: 4,
              padding: '0 10px 8px',
            }}
          >
            {allTags.map((t) => (
              <TagChip
                key={t}
                tag={t}
                active={tagFilter === t}
                onClick={() => setTagFilter((cur) => (cur === t ? null : t))}
              />
            ))}
          </div>
        )}
        <div style={{ flex: 1, overflowY: 'auto' }}>
          {all !== null && visible.length === 0 && (
            <div
              style={{
                padding: '14px 12px',
                fontSize: '0.68rem',
                color: 'var(--wks-text-faint)',
              }}
            >
              {dirNotes.length === 0
                ? 'No notes for this project yet.'
                : 'No notes match the filter.'}
            </div>
          )}
          {visible.map((n) => (
            <div
              key={n.id}
              onClick={() => setSelectedId(n.id)}
              style={{
                padding: '7px 10px',
                cursor: 'pointer',
                borderLeft: `2px solid ${n.id === selectedId ? 'var(--wks-accent)' : 'transparent'}`,
                background: n.id === selectedId ? 'var(--wks-bg-selected)' : 'transparent',
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 6,
                  fontSize: '0.72rem',
                  fontWeight: 600,
                  color: 'var(--wks-text-primary)',
                }}
              >
                <span
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {n.id === selectedId && draftTitle ? draftTitle : n.title}
                </span>
                <span
                  style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}
                >
                  {relTime(n.updatedAt)}
                </span>
              </div>
              {n.tags.length > 0 && (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 3, marginTop: 3 }}>
                  {n.tags.map((t) => (
                    <TagChip key={t} tag={t} />
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* ── Editor column ── */}
      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        {!selected ? (
          <div
            style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 8,
              color: 'var(--wks-text-faint)',
              fontSize: '0.75rem',
            }}
          >
            <span>No note selected.</span>
            <button
              onClick={() => void createNote()}
              style={{
                fontSize: '0.72rem',
                fontFamily: 'inherit',
                fontWeight: 600,
                cursor: 'pointer',
                padding: '5px 14px',
                borderRadius: 6,
                border: 'none',
                background: 'var(--wks-accent)',
                color: 'var(--wks-text-on-accent, #fff)',
              }}
            >
              ＋ New note
            </button>
          </div>
        ) : (
          <>
            {/* Title + tags + actions */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 10px',
                borderBottom: '1px solid var(--wks-border-subtle)',
                background: 'var(--wks-bg-raised)',
              }}
            >
              <input
                value={draftTitle}
                onChange={(e) => {
                  setDraftTitle(e.target.value);
                  scheduleSave({ title: e.target.value });
                }}
                placeholder="Title"
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontSize: '0.8rem',
                  fontWeight: 650,
                  fontFamily: 'inherit',
                  background: 'transparent',
                  color: 'var(--wks-text-primary)',
                  border: 'none',
                  outline: 'none',
                }}
              />
              <input
                value={draftTags}
                onChange={(e) => {
                  setDraftTags(e.target.value);
                  scheduleSave({ tags: e.target.value });
                }}
                placeholder="tags, comma, separated"
                title="Tags — comma separated"
                style={{
                  width: 180,
                  fontSize: '0.68rem',
                  fontFamily: 'var(--wks-font-mono, monospace)',
                  padding: '3px 8px',
                  backgroundColor: 'var(--wks-bg-input)',
                  color: 'var(--wks-text-secondary)',
                  border: '1px solid var(--wks-border-subtle)',
                  borderRadius: 'var(--wks-radius-pill, 999px)',
                  outline: 'none',
                }}
              />
              {(['write', 'split', 'preview'] as const).map((v) => (
                <button
                  key={v}
                  type="button"
                  onClick={() => setView(v)}
                  style={{
                    fontSize: '0.69rem',
                    fontFamily: 'inherit',
                    cursor: 'pointer',
                    padding: '2px 8px',
                    borderRadius: 4,
                    textTransform: 'capitalize',
                    border: '1px solid ' + (view === v ? 'var(--wks-accent)' : 'transparent'),
                    background: view === v ? 'var(--wks-accent-bg)' : 'transparent',
                    color: view === v ? 'var(--wks-accent-text)' : 'var(--wks-text-muted)',
                  }}
                >
                  {v}
                </button>
              ))}
              <button
                onClick={() => void deleteNote(selected.id)}
                title="Delete note"
                style={{
                  fontSize: '0.72rem',
                  fontFamily: 'inherit',
                  cursor: 'pointer',
                  padding: '2px 6px',
                  borderRadius: 4,
                  border: 'none',
                  background: 'transparent',
                  color: 'var(--wks-text-faint)',
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-error)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
                }}
              >
                ✕
              </button>
            </div>

            {/* Editor / preview row */}
            <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
              {showWrite && (
                <textarea
                  value={draftContent}
                  onChange={(e) => {
                    setDraftContent(e.target.value);
                    scheduleSave({ content: e.target.value });
                  }}
                  onKeyDown={handleKeyDown}
                  spellCheck={false}
                  placeholder="Write markdown… saved per project directory."
                  style={{
                    flex: 1,
                    minWidth: 0,
                    border: 'none',
                    outline: 'none',
                    resize: 'none',
                    padding: '10px 12px',
                    boxSizing: 'border-box',
                    background: 'var(--wks-bg-input)',
                    color: 'var(--wks-text-primary)',
                    fontFamily: 'var(--wks-mono, ui-monospace, monospace)',
                    fontSize: '0.74rem',
                    lineHeight: 1.6,
                  }}
                />
              )}
              {showPreview && (
                <div
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflowY: 'auto',
                    padding: '6px 14px',
                    boxSizing: 'border-box',
                    borderLeft: showWrite ? '1px solid var(--wks-border-subtle)' : 'none',
                    background: 'var(--wks-bg-base)',
                    color: 'var(--wks-text-secondary)',
                    fontSize: '0.78rem',
                  }}
                >
                  {draftContent.trim() ? (
                    preview
                  ) : (
                    <div
                      style={{
                        color: 'var(--wks-text-disabled)',
                        fontStyle: 'italic',
                        marginTop: 6,
                      }}
                    >
                      Nothing to preview yet.
                    </div>
                  )}
                </div>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default NotesPane;
