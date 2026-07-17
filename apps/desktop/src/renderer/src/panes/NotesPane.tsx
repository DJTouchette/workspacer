/**
 * NotesPane — a per-agent markdown scratchpad.
 *
 * Content is held in the pane's own React state for snappy typing and pushed to
 * the owning PaneConfig (debounced) via `onNotesChange`, so it persists with the
 * session like a browser pane's URL. Write / Split / Preview views, reusing the
 * shared lightweight markdown renderer — no external editor deps.
 */
import React, { useState, useRef, useEffect, useMemo } from 'react';
import { parseMarkdownBlocks } from '../components/markdown';

type View = 'write' | 'split' | 'preview';

const VIEW_KEY = 'wksNotesView';
const PERSIST_DEBOUNCE_MS = 400;

interface NotesPaneProps {
  title: string;
  /** Initial content, restored from the persisted PaneConfig. */
  notes?: string;
  /** Persist content back onto the PaneConfig (debounced by the pane). */
  onNotesChange?: (notes: string) => void;
}

const NotesPane: React.FC<NotesPaneProps> = ({ notes, onNotesChange }) => {
  const [text, setText] = useState(notes ?? '');
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

  // Persist (debounced) without re-rendering on every keystroke from the parent.
  const onChangeRef = useRef(onNotesChange);
  onChangeRef.current = onNotesChange;
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const latest = useRef(text);

  const update = (v: string) => {
    setText(v);
    latest.current = v;
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => onChangeRef.current?.(v), PERSIST_DEBOUNCE_MS);
  };

  // Flush any pending edit on unmount (agent switch / close) so nothing is lost.
  useEffect(
    () => () => {
      if (timer.current) {
        clearTimeout(timer.current);
        onChangeRef.current?.(latest.current);
      }
    },
    [],
  );

  const preview = useMemo(() => parseMarkdownBlocks(text), [text]);

  // Tab inserts two spaces instead of moving focus out of the editor.
  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== 'Tab') return;
    e.preventDefault();
    const ta = e.currentTarget;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    update(text.slice(0, start) + '  ' + text.slice(end));
    requestAnimationFrame(() => {
      ta.selectionStart = ta.selectionEnd = start + 2;
    });
  };

  const showWrite = view !== 'preview';
  const showPreview = view !== 'write';

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        background: 'var(--wks-bg-base)',
      }}
    >
      {/* Toolbar */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 2,
          padding: '4px 6px',
          flex: '0 0 auto',
          background: 'var(--wks-bg-raised)',
          borderBottom: '1px solid var(--wks-border-subtle)',
        }}
      >
        {(['write', 'split', 'preview'] as const).map((v) => (
          <button
            key={v}
            type="button"
            onClick={() => setView(v)}
            style={{
              fontSize: '0.66rem',
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
        <div style={{ flex: 1 }} />
        <span
          style={{ fontSize: '0.62rem', color: 'var(--wks-text-disabled)', userSelect: 'none' }}
        >
          **bold** *italic* `code` # heading - list
        </span>
      </div>

      {/* Editor / preview row */}
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {showWrite && (
          <textarea
            value={text}
            onChange={(e) => update(e.target.value)}
            onKeyDown={handleKeyDown}
            spellCheck={false}
            placeholder="Jot notes for this agent… (markdown, saved with the session)"
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
            {text.trim() ? (
              preview
            ) : (
              <div style={{ color: 'var(--wks-text-disabled)', fontStyle: 'italic', marginTop: 6 }}>
                Nothing to preview yet.
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default NotesPane;
