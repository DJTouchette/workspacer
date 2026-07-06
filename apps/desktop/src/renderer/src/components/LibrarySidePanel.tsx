import React, { useState, useMemo, useEffect } from 'react';
import { useLibrary } from '../hooks/useLibrary';
import { runLibraryItem } from '../lib/libraryBus';
import { Zap } from './icons';
import type { LibraryItem, LibraryScope, LibraryKind } from '../types/library';
import { captionInsetTop } from '../lib/layoutUtils';

interface Props {
  visible: boolean;
  onClose: () => void;
  /** Project root for project-scoped items (falls back to the app cwd). */
  cwd?: string;
}

const PANEL_W = 380;

/**
 * A right-side slide-in panel for browsing the Library (prompts & skills) while
 * an agent is open. "→ Agent" drops the item's text into the active Claude
 * pane's input (via the library:run → library:insert bus); "Copy" copies it.
 * Toggled with the library shortcut (Ctrl+L). No backdrop, so the agent stays
 * usable alongside it.
 */
const LibrarySidePanel: React.FC<Props> = ({ visible, onClose, cwd }) => {
  const { items } = useLibrary(cwd);
  const [query, setQuery] = useState('');
  const [scopeFilter, setScopeFilter] = useState<'all' | LibraryScope>('all');
  const [flashId, setFlashId] = useState<string | null>(null);

  // Escape closes the panel.
  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [visible, onClose]);

  const filtered = useMemo(() => {
    const q = query.toLowerCase();
    return items.filter(
      (it) =>
        (scopeFilter === 'all' || it.scope === scopeFilter) &&
        (q === '' ||
          it.title.toLowerCase().includes(q) ||
          (it.description ?? '').toLowerCase().includes(q) ||
          (it.tags ?? []).some((t) => t.toLowerCase().includes(q)) ||
          it.body.toLowerCase().includes(q)),
    );
  }, [items, query, scopeFilter]);

  const sendToAgent = (it: LibraryItem) => {
    runLibraryItem(it, 'insert');
    setFlashId(it.id);
    setTimeout(() => setFlashId((cur) => (cur === it.id ? null : cur)), 900);
  };

  return (
    <div
      aria-hidden={!visible}
      style={{
        position: 'fixed',
        top: 0,
        right: 0,
        bottom: 0,
        width: PANEL_W,
        maxWidth: '92vw',
        zIndex: 1800,
        display: 'flex',
        flexDirection: 'column',
        // Opaque background (no backdrop blur): a live blur recomputes every
        // frame as the panel slides, which is what made it feel laggy. A solid
        // surface lets the transform animate on the GPU compositor.
        backgroundColor: 'var(--wks-bg-surface)',
        borderLeft: '1px solid var(--wks-glass-border)',
        // Respect the theme's corner setting (0px when square) on the inner edge.
        borderTopLeftRadius: 'var(--wks-radius-lg)',
        borderBottomLeftRadius: 'var(--wks-radius-lg)',
        boxShadow: '-8px 0 28px var(--wks-shadow)',
        transform: visible ? 'translateX(0)' : 'translateX(100%)',
        transition: 'transform 0.22s cubic-bezier(0.32, 0.72, 0, 1)',
        willChange: 'transform',
        // Don't intercept clicks while hidden (it sits off-screen anyway).
        pointerEvents: visible ? 'auto' : 'none',
        overflow: 'hidden',
      }}
    >
      {/* Header — clear the Windows caption buttons (titleBarOverlay). */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: `${12 + captionInsetTop()}px 14px 12px`,
          borderBottom: '1px solid var(--wks-glass-border)',
        }}
      >
        <Zap size={15} strokeWidth={1.9} />
        <span style={{ fontWeight: 600, fontSize: '0.85rem', color: 'var(--wks-text-primary)' }}>
          Library
        </span>
        <div style={{ flex: 1 }} />
        <button onClick={onClose} title="Close (Esc)" style={closeBtn}>
          ✕
        </button>
      </div>

      {/* Search + scope filter */}
      <div
        style={{
          padding: '10px 14px',
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          borderBottom: '1px solid var(--wks-glass-border)',
        }}
      >
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search prompts & skills…"
          spellCheck={false}
          autoFocus
          style={{
            height: 30,
            padding: '0 10px',
            fontSize: '0.78rem',
            fontFamily: 'inherit',
            color: 'var(--wks-text-primary)',
            background: 'var(--wks-bg-base)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 6,
            outline: 'none',
          }}
        />
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'global', 'project', 'claude'] as const).map((s) => (
            <button key={s} onClick={() => setScopeFilter(s)} style={chip(scopeFilter === s)}>
              {s}
            </button>
          ))}
        </div>
      </div>

      {/* List */}
      <div
        style={{
          flex: 1,
          overflowY: 'auto',
          padding: 12,
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
        }}
      >
        {filtered.length === 0 && (
          <div
            style={{
              textAlign: 'center',
              color: 'var(--wks-text-faint)',
              padding: 36,
              fontSize: '0.76rem',
            }}
          >
            {items.length === 0 ? 'No library items yet.' : 'No matches.'}
          </div>
        )}
        {filtered.map((it) => (
          <div key={`${it.scope}:${it.id}`} style={card(flashId === it.id)}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <span
                style={{
                  fontWeight: 600,
                  fontSize: '0.78rem',
                  color: 'var(--wks-text-primary)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {it.title}
              </span>
              <span style={kindBadge(it.kind)}>{it.kind}</span>
            </div>
            {it.description && (
              <div
                style={{ fontSize: '0.66rem', color: 'var(--wks-text-secondary)', marginTop: 3 }}
              >
                {it.description}
              </div>
            )}
            <div
              style={{
                fontSize: '0.62rem',
                color: 'var(--wks-text-faint)',
                marginTop: 4,
                maxHeight: 30,
                overflow: 'hidden',
                fontFamily: 'var(--wks-mono, ui-monospace, monospace)',
                lineHeight: 1.4,
              }}
            >
              {it.body.replace(/\s+/g, ' ').slice(0, 140)}
            </div>
            <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
              <button
                onClick={() => sendToAgent(it)}
                style={primaryBtn}
                title="Insert into the active agent's input"
              >
                {flashId === it.id ? '✓ Added' : '→ Agent'}
              </button>
              <button
                onClick={() => runLibraryItem(it, 'copy')}
                style={ghostBtn}
                title="Copy to clipboard"
              >
                Copy
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

const closeBtn: React.CSSProperties = {
  width: 24,
  height: 24,
  padding: 0,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  fontSize: '0.85rem',
  color: 'var(--wks-text-faint)',
  background: 'transparent',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm)',
  cursor: 'pointer',
};

const chip = (active: boolean): React.CSSProperties => ({
  flex: 1,
  padding: '4px 0',
  fontSize: '0.62rem',
  fontWeight: 600,
  textTransform: 'capitalize',
  color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-faint)',
  background: active ? 'var(--wks-bg-selected)' : 'transparent',
  border: '1px solid ' + (active ? 'var(--wks-accent)' : 'var(--wks-border)'),
  borderRadius: 'var(--wks-radius-pill)',
  cursor: 'pointer',
  fontFamily: 'inherit',
});

const card = (flash: boolean): React.CSSProperties => ({
  padding: 10,
  borderRadius: 'var(--wks-radius-md)',
  background: 'var(--wks-bg-surface)',
  border: '1px solid ' + (flash ? 'var(--wks-accent)' : 'var(--wks-glass-border)'),
  transition: 'border-color 0.2s',
});

const kindBadge = (kind: LibraryKind): React.CSSProperties => ({
  fontSize: '0.55rem',
  fontWeight: 700,
  textTransform: 'uppercase',
  letterSpacing: '0.04em',
  padding: '1px 6px',
  borderRadius: 'var(--wks-radius-pill)',
  color: 'var(--wks-text-muted)',
  background: 'var(--wks-bg-elevated)',
  flexShrink: 0,
});

const primaryBtn: React.CSSProperties = {
  flex: 1,
  height: 26,
  fontSize: '0.7rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  color: '#fff',
  background: 'var(--wks-accent)',
  border: '1px solid var(--wks-accent)',
  borderRadius: 6,
  cursor: 'pointer',
};

const ghostBtn: React.CSSProperties = {
  flex: '0 0 auto',
  height: 26,
  padding: '0 12px',
  fontSize: '0.7rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  color: 'var(--wks-text-secondary)',
  background: 'var(--wks-bg-elevated)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 6,
  cursor: 'pointer',
};

export default LibrarySidePanel;
