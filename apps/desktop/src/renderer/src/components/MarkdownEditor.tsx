/**
 * Markdown editor with live preview — used by the Library item editor.
 * Write / Split / Preview views; the chosen view persists across sessions.
 * No external editor deps: a mono textarea + the shared lightweight
 * markdown renderer (components/markdown.tsx).
 */
import React, { useMemo, useState } from 'react';
import { parseMarkdownBlocks } from './markdown';

type EditorView = 'write' | 'split' | 'preview';

const VIEW_KEY = 'wksMdEditorView';

/** Render {{cwd}} / {{?Question}} template tokens as inline-code chips in the
 *  preview so they stand out (they're host-side substitutions, not markdown). */
function chipTemplates(text: string): string {
  return text.replace(/(`?)(\{\{[^{}]+\}\})(`?)/g, (_m, pre, tok, post) =>
    pre || post ? `${pre}${tok}${post}` : `\`${tok}\``);
}

const MarkdownEditor: React.FC<{
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}> = ({ value, onChange, placeholder }) => {
  const [view, setViewState] = useState<EditorView>(() => {
    const saved = localStorage.getItem(VIEW_KEY);
    return saved === 'write' || saved === 'preview' ? saved : 'split';
  });
  const setView = (v: EditorView) => { setViewState(v); try { localStorage.setItem(VIEW_KEY, v); } catch {} };

  const preview = useMemo(
    () => parseMarkdownBlocks(chipTemplates(value)),
    [value],
  );

  // Tab inserts two spaces instead of leaving the editor
  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== 'Tab') return;
    e.preventDefault();
    const ta = e.currentTarget;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    onChange(value.slice(0, start) + '  ' + value.slice(end));
    requestAnimationFrame(() => { ta.selectionStart = ta.selectionEnd = start + 2; });
  };

  const showWrite = view !== 'preview';
  const showPreview = view !== 'write';

  return (
    <div style={{
      border: '1px solid var(--wks-border-input)',
      borderRadius: 6,
      overflow: 'hidden',
      background: 'var(--wks-bg-input)',
    }}>
      {/* Toolbar */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 2,
        padding: '4px 6px',
        background: 'var(--wks-bg-raised)',
        borderBottom: '1px solid var(--wks-border-subtle)',
      }}>
        {(['write', 'split', 'preview'] as const).map((v) => (
          <button
            key={v}
            type="button"
            onClick={() => setView(v)}
            style={{
              fontSize: '0.62rem', fontFamily: 'inherit', cursor: 'pointer',
              padding: '2px 8px', borderRadius: 4, textTransform: 'capitalize',
              border: '1px solid ' + (view === v ? 'var(--wks-accent)' : 'transparent'),
              background: view === v ? 'var(--wks-accent-bg)' : 'transparent',
              color: view === v ? 'var(--wks-accent-text)' : 'var(--wks-text-muted)',
            }}
          >
            {v}
          </button>
        ))}
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: '0.58rem', color: 'var(--wks-text-disabled)', userSelect: 'none' }}>
          **bold**  *italic*  `code`  # heading  - list
        </span>
      </div>

      {/* Editor / preview row — drag the bottom edge to resize */}
      <div style={{ display: 'flex', height: 340, resize: 'vertical', overflow: 'hidden', minHeight: 160 }}>
        {showWrite && (
          <textarea
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={handleKeyDown}
            spellCheck={false}
            placeholder={placeholder}
            style={{
              flex: 1, minWidth: 0, border: 'none', outline: 'none', resize: 'none',
              padding: '10px 12px', boxSizing: 'border-box',
              background: 'var(--wks-bg-input)', color: 'var(--wks-text-primary)',
              fontFamily: 'var(--wks-mono, ui-monospace, monospace)',
              fontSize: '0.74rem', lineHeight: 1.6,
            }}
          />
        )}
        {showPreview && (
          <div style={{
            flex: 1, minWidth: 0, overflowY: 'auto',
            padding: '6px 14px', boxSizing: 'border-box',
            borderLeft: showWrite ? '1px solid var(--wks-border-subtle)' : 'none',
            background: 'var(--wks-bg-base)',
            color: 'var(--wks-text-secondary)',
            fontSize: '0.78rem',
          }}>
            {value.trim()
              ? preview
              : <div style={{ color: 'var(--wks-text-disabled)', fontStyle: 'italic', marginTop: 6 }}>Nothing to preview yet.</div>}
          </div>
        )}
      </div>
    </div>
  );
};

export default MarkdownEditor;
