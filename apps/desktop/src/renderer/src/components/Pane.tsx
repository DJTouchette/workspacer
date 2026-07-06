import React, { useCallback, useRef, useState, useEffect } from 'react';
import { Columns2 } from 'lucide-react';
import { PaneType } from '../types/pane';
import { useConfig } from '../hooks/useConfig';
import { PaneIcon } from './icons';

// Pane types offered by the in-pane split button. Mirrors the palette's
// built-in actions (minus the special Library/Editor entries) so "split into…"
// covers the same core set everywhere.
const SPLIT_TYPES: { type: PaneType; label: string }[] = [
  { type: 'claude', label: 'Claude Code' },
  { type: 'terminal', label: 'Terminal' },
  { type: 'browser', label: 'Browser' },
  { type: 'notes', label: 'Notes' },
  { type: 'review', label: 'Review' },
];

interface PaneProps {
  id: string;
  type: PaneType;
  title: string;
  isActive: boolean;
  onClose: (id: string) => void;
  onFocus: (id: string) => void;
  onMove?: (id: string, delta: number) => void;
  onRename?: (id: string, title: string) => void;
  renameSignal?: number;
  /** Omit the header bar (title + close button). Used for single-pane tabs
   *  where the tab itself already carries the label. */
  hideHeader?: boolean;
  /** Don't show the focused/accent border — used when a tab has a single pane,
   *  where indicating "which pane is focused" is meaningless. */
  hideActiveBorder?: boolean;
  /** Full-bleed: drop the card margin/radius/border/shadow so the pane sits
   *  flush edge-to-edge under the tab bar (matches the mockup). Used for
   *  single-pane tabs; split panes keep their card framing. */
  flush?: boolean;
  /** Split the tab by adding a pane of the chosen type. When wired, a small
   *  split button appears on the pane (in the header, or floating when flush). */
  onSplit?: (type: PaneType) => void;
  children: React.ReactNode;
}

const Pane: React.FC<PaneProps> = ({
  id,
  type,
  title,
  isActive,
  onClose,
  onFocus,
  onMove,
  onRename,
  renameSignal,
  hideHeader,
  hideActiveBorder,
  flush,
  onSplit,
  children,
}) => {
  const { config } = useConfig();
  const headerHeight = config.ui.paneHeaderHeight || 22;
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;

  const [splitMenuOpen, setSplitMenuOpen] = useState(false);
  const splitRef = useRef<HTMLDivElement>(null);

  // Close the split menu on outside-click / Escape.
  useEffect(() => {
    if (!splitMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (splitRef.current && !splitRef.current.contains(e.target as Node)) setSplitMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSplitMenuOpen(false);
    };
    document.addEventListener('mousedown', onDown, true);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('mousedown', onDown, true);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [splitMenuOpen]);

  // The split control: a button that toggles a "split into…" type picker.
  // Rendered inside the header when there is one, else floated over a flush
  // (single) pane. Anchored in `splitRef` so outside-click detection covers
  // both the button and its menu.
  const splitControl = onSplit ? (
    <div ref={splitRef} style={{ position: 'relative' }}>
      <button
        className="pane-split-control"
        onClick={(e) => {
          e.stopPropagation();
          setSplitMenuOpen((v) => !v);
        }}
        title="Split pane"
        style={{
          background: 'none',
          border: 'none',
          color: splitMenuOpen ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
          cursor: 'pointer',
          padding: '2px',
          margin: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          borderRadius: '3px',
          opacity: splitMenuOpen ? 1 : undefined,
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
        }}
      >
        <Columns2 size={13} strokeWidth={1.75} />
      </button>
      {splitMenuOpen && (
        <div
          role="menu"
          style={{
            position: 'absolute',
            top: 'calc(100% + 4px)',
            right: 0,
            zIndex: 1100,
            minWidth: '140px',
            padding: '4px',
            background: 'var(--wks-bg-elevated)',
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 'var(--wks-radius-md)',
            boxShadow: '0 8px 28px var(--wks-glass-shadow)',
          }}
        >
          <div
            style={{
              fontSize: '0.55rem',
              textTransform: 'uppercase',
              letterSpacing: '0.05em',
              color: 'var(--wks-text-faint)',
              padding: '2px 8px 4px',
            }}
          >
            Split into
          </div>
          {SPLIT_TYPES.map(({ type, label }) => (
            <button
              key={type}
              onClick={(e) => {
                e.stopPropagation();
                setSplitMenuOpen(false);
                onSplit(type);
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                width: '100%',
                padding: '5px 8px',
                background: 'none',
                border: 'none',
                borderRadius: '4px',
                color: 'var(--wks-text-secondary)',
                fontSize: '0.7rem',
                cursor: 'pointer',
                textAlign: 'left',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
              }}
            >
              <PaneIcon type={type} size={13} />
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  ) : null;

  const [isEditing, setIsEditing] = useState(false);
  const [editValue, setEditValue] = useState(title);
  const inputRef = useRef<HTMLInputElement>(null);
  const prevSignalRef = useRef(renameSignal);

  // Trigger rename only when F2 signal actually increments
  useEffect(() => {
    if (renameSignal && renameSignal > 0 && renameSignal !== prevSignalRef.current && onRename) {
      setEditValue(title);
      setIsEditing(true);
      setTimeout(() => inputRef.current?.select(), 0);
    }
    prevSignalRef.current = renameSignal;
  }, [renameSignal]);

  const handleStartRename = useCallback(() => {
    if (!onRename) return;
    setEditValue(title);
    setIsEditing(true);
    setTimeout(() => inputRef.current?.select(), 0);
  }, [title, onRename]);

  const handleFinishRename = useCallback(() => {
    setIsEditing(false);
    const trimmed = editValue.trim();
    if (trimmed && trimmed !== title && onRename) {
      onRename(id, trimmed);
    }
  }, [editValue, title, id, onRename]);

  const handleRenameKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        handleFinishRename();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setIsEditing(false);
      }
      e.stopPropagation();
    },
    [handleFinishRename],
  );

  const handleHeaderMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (!onMoveRef.current) return;
      if (e.button !== 0) return;
      if ((e.target as HTMLElement).closest('button')) return;
      if ((e.target as HTMLElement).closest('input')) return;

      e.preventDefault();
      e.stopPropagation();

      let lastX = e.clientX;
      const moveThreshold = 60;

      document.body.style.cursor = 'grabbing';
      document.body.style.userSelect = 'none';

      const handleMouseMove = (moveEvent: MouseEvent) => {
        const delta = moveEvent.clientX - lastX;
        if (Math.abs(delta) > moveThreshold) {
          const direction = delta > 0 ? 1 : -1;
          onMoveRef.current?.(id, direction);
          lastX = moveEvent.clientX;
        }
      };

      const handleMouseUp = () => {
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        document.removeEventListener('mousemove', handleMouseMove);
        document.removeEventListener('mouseup', handleMouseUp);
      };

      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    },
    [id],
  );

  return (
    <div
      className="pane-wrapper"
      data-pane-id={id}
      style={{
        flex: 1,
        minWidth: 0,
        height: flush ? '100%' : 'calc(100% - 2px)',
        display: 'flex',
        flexDirection: 'column',
        borderRadius: flush ? 0 : 'var(--wks-radius-md)',
        overflow: 'hidden',
        // Split panes sit almost flush: a 1px margin keeps adjacent cards from
        // overlapping their borders without opening a visible gutter.
        margin: flush ? 0 : '1px',
        border: flush
          ? 'none'
          : isActive && !hideActiveBorder
            ? '1px solid var(--wks-border-active)'
            : '1px solid var(--wks-glass-border)',
        boxShadow: flush
          ? 'none'
          : isActive && !hideActiveBorder
            ? 'inset 0 0 0 1.5px var(--wks-glass-highlight), 0 0 0 1px var(--wks-accent-glow), 0 10px 34px var(--wks-glass-shadow)'
            : 'inset 0 0 0 1.5px var(--wks-glass-highlight), 0 6px 22px var(--wks-glass-shadow)',
        transition: 'none',
        flexShrink: 0,
        position: 'relative',
        zIndex: 0,
      }}
      onClick={() => onFocus(id)}
    >
      {/* Header bar — hidden for single-pane tabs (the tab label suffices). */}
      {!hideHeader && (
        <div
          className="pane-header"
          onMouseDown={handleHeaderMouseDown}
          style={{
            height: `${headerHeight}px`,
            minHeight: `${headerHeight}px`,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '0 8px',
            // Solid-ish tint, no backdrop-filter: the header sits over the static
            // app background (blur added nothing visible) and a backdrop-filter
            // inside the pane's clipped region couples it to heavy repaints from
            // dynamic panes (streaming Claude transcript / WebGL terminal), which
            // caused transient compositing garble. See bg-header / bg-elevated.
            backgroundColor: isActive ? 'var(--wks-bg-header)' : 'var(--wks-bg-elevated)',
            borderBottom: '1px solid var(--wks-glass-border)',
            cursor: onMove ? 'grab' : 'default',
            userSelect: 'none',
            position: 'relative',
            zIndex: 1001,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: '6px', overflow: 'hidden' }}>
            <span
              style={{
                display: 'flex',
                alignItems: 'center',
                opacity: 0.75,
                color: 'var(--wks-text-tertiary)',
              }}
            >
              <PaneIcon type={type} size={12} />
            </span>
            {isEditing ? (
              <input
                ref={inputRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onBlur={handleFinishRename}
                onKeyDown={handleRenameKeyDown}
                style={{
                  fontSize: '0.6rem',
                  color: 'var(--wks-text-primary)',
                  fontWeight: 500,
                  backgroundColor: 'var(--wks-bg-input)',
                  border: '1px solid var(--wks-accent)',
                  borderRadius: '2px',
                  padding: '0 4px',
                  height: '16px',
                  width: '120px',
                  outline: 'none',
                  fontFamily: 'inherit',
                }}
              />
            ) : (
              <span
                onDoubleClick={handleStartRename}
                style={{
                  fontSize: '0.6rem',
                  color: 'var(--wks-text-secondary)',
                  fontWeight: 500,
                  cursor: onRename ? 'text' : 'default',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
                title="Double-click to rename"
              >
                {title}
              </span>
            )}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '2px' }}>
            {splitControl}
            <button
              onClick={(e) => {
                e.stopPropagation();
                onClose(id);
              }}
              style={{
                background: 'none',
                border: 'none',
                color: 'var(--wks-text-muted)',
                cursor: 'pointer',
                fontSize: '0.85rem',
                padding: '0 4px',
                margin: 0,
                width: 'auto',
                height: 'auto',
                lineHeight: 1,
                borderRadius: '3px',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
              onMouseEnter={(e) => {
                (e.target as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                (e.target as HTMLElement).style.color = 'var(--wks-text-primary)';
              }}
              onMouseLeave={(e) => {
                (e.target as HTMLElement).style.backgroundColor = 'transparent';
                (e.target as HTMLElement).style.color = 'var(--wks-text-muted)';
              }}
              title="Close pane"
            >
              &#x2715;
            </button>
          </div>
        </div>
      )}

      {/* Flush (single-pane) tabs have no header, so float the split control in
          the top-right corner — hover-revealed via .pane-wrapper:hover. */}
      {hideHeader && splitControl && (
        <div
          className="pane-split-floating"
          style={{
            position: 'absolute',
            top: '4px',
            right: '4px',
            zIndex: 1002,
            background: 'var(--wks-bg-elevated)',
            borderRadius: '4px',
            boxShadow: '0 2px 8px var(--wks-glass-shadow)',
            // Keep it visible while the menu is open even if the cursor moves
            // off the pane onto the popover.
            opacity: splitMenuOpen ? 1 : undefined,
          }}
        >
          {splitControl}
        </div>
      )}

      <div
        className="pane-content"
        style={{
          flex: 1,
          overflow: 'hidden',
          backgroundColor: 'var(--wks-bg-surface)',
          isolation: 'isolate',
          position: 'relative',
          zIndex: 0,
        }}
      >
        {children}
      </div>
    </div>
  );
};

export default Pane;
