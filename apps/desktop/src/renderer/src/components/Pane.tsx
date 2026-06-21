import React, { useCallback, useRef, useState, useEffect } from 'react';
import { PaneType } from '../types/pane';
import { useConfig } from '../hooks/useConfig';
import { PaneIcon } from './icons';

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
  children,
}) => {
  const { config } = useConfig();
  const headerHeight = config.ui.paneHeaderHeight || 22;
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;

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

  const handleRenameKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      handleFinishRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setIsEditing(false);
    }
    e.stopPropagation();
  }, [handleFinishRename]);

  const handleHeaderMouseDown = useCallback((e: React.MouseEvent) => {
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
  }, [id]);

  return (
    <div
      className="pane-wrapper"
      data-pane-id={id}
      style={{
        flex: 1,
        minWidth: 0,
        height: flush ? '100%' : 'calc(100% - 8px)',
        display: 'flex',
        flexDirection: 'column',
        borderRadius: flush ? 0 : 'var(--wks-radius-lg)',
        overflow: 'hidden',
        margin: flush ? 0 : '4px 8px',
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
          backgroundColor: isActive
            ? 'var(--wks-bg-header)'
            : 'var(--wks-bg-elevated)',
          borderBottom: '1px solid var(--wks-glass-border)',
          cursor: onMove ? 'grab' : 'default',
          userSelect: 'none',
          position: 'relative',
          zIndex: 1001,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', overflow: 'hidden' }}>
          <span style={{ display: 'flex', alignItems: 'center', opacity: 0.75, color: 'var(--wks-text-tertiary)' }}>
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
            lineHeight: '1',
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
