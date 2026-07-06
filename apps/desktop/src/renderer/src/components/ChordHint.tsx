import React from 'react';
import { formatBinding, buildChordTree, chordMenu, chordBreadcrumb } from '../lib/shortcuts';

interface ChordHintProps {
  /** Live chord path: null = idle (hidden), [] = root after prefix, ['t'] = in
   *  the Tab submenu, etc. */
  path: string[] | null;
  /** Workspace prefix combo, e.g. 'ctrl+space'. */
  prefix: string;
  /** Resolved shortcuts (config merged with defaults). */
  shortcuts: Record<string, string>;
  /** Expand into a which-key cheatsheet of the keys available at this level.
   *  When false, only the minimal prefix/breadcrumb indicator shows. */
  showOptions: boolean;
}

/**
 * Bottom-right indicator that appears the moment the workspace prefix is
 * pressed and follows you into submenus. With options enabled it's a which-key
 * style cheatsheet of the keys reachable at the current level.
 */
const ChordHint: React.FC<ChordHintProps> = ({ path, prefix, shortcuts, showOptions }) => {
  if (path === null) return null;

  const tree = buildChordTree(shortcuts);
  const items = showOptions ? chordMenu(tree, path) : [];
  const crumbs = chordBreadcrumb(path);

  return (
    <div
      style={{
        position: 'fixed',
        bottom: '16px',
        right: '16px',
        zIndex: 200,
        backgroundColor: 'var(--wks-glass-strong)',
        backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        border: '1px solid var(--wks-glass-border)',
        borderRadius: 'var(--wks-radius-md)',
        boxShadow:
          '0 12px 36px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
        padding: items.length ? '8px 10px' : '3px 9px',
        maxWidth: 'min(440px, 72vw)',
        fontFamily: 'var(--wks-font-mono)',
      }}
    >
      {/* Breadcrumb header: prefix chip › group › group … */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '5px',
          flexWrap: 'wrap',
          color: 'var(--wks-accent)',
          fontWeight: 700,
          fontSize: '0.7rem',
        }}
      >
        <span
          style={{
            padding: '1px 6px',
            borderRadius: '3px',
            backgroundColor: 'var(--wks-accent)',
            color: 'var(--wks-text-on-accent, #fff)',
          }}
        >
          {formatBinding(prefix)}
        </span>
        {crumbs.map((c, i) => (
          <React.Fragment key={i}>
            <span style={{ color: 'var(--wks-text-faint)', fontWeight: 400 }}>›</span>
            <span>{c}</span>
          </React.Fragment>
        ))}
        <span style={{ color: 'var(--wks-text-faint)', fontWeight: 400 }}>then…</span>
      </div>

      {items.length > 0 && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(2, minmax(150px, 1fr))',
            gap: '1px 16px',
            marginTop: '8px',
          }}
        >
          {items.map((it) => (
            <div
              key={it.step}
              style={{ display: 'flex', alignItems: 'baseline', gap: '7px', padding: '1px 0' }}
            >
              <kbd
                style={{
                  flexShrink: 0,
                  minWidth: '14px',
                  textAlign: 'center',
                  padding: '0 4px',
                  borderRadius: '3px',
                  border: '1px solid var(--wks-border-input)',
                  backgroundColor: 'var(--wks-bg-input)',
                  color: 'var(--wks-text-secondary)',
                  fontSize: '0.62rem',
                  fontWeight: 600,
                }}
              >
                {it.keyLabel}
              </kbd>
              <span
                style={{
                  color: it.isGroup ? 'var(--wks-text-secondary)' : 'var(--wks-text-muted)',
                  fontWeight: it.isGroup ? 600 : 400,
                  fontSize: '0.65rem',
                  whiteSpace: 'nowrap',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                }}
              >
                {it.label}
                {it.isGroup ? ' ▸' : ''}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Footer: navigation affordances once you've shown options. */}
      {items.length > 0 && (
        <div
          style={{
            marginTop: '7px',
            paddingTop: '6px',
            borderTop: '1px solid var(--wks-border)',
            fontSize: '0.58rem',
            color: 'var(--wks-text-faint)',
            display: 'flex',
            gap: '12px',
          }}
        >
          {path.length > 0 && <span>⌫ back</span>}
          <span>Esc cancel</span>
        </div>
      )}
    </div>
  );
};

export default ChordHint;
