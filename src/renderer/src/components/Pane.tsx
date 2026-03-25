import React, { useCallback, useRef, useState, useEffect } from 'react';
import { PaneType } from '../types/pane';
import { useConfig } from '../hooks/useConfig';

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
  children: React.ReactNode;
}

const typeIndicators: Record<PaneType, string> = {
  terminal: '>_',
  browser: '\u{1F310}',
  notes: '\u{1F4DD}',
  agent: '\u{1F916}',
  settings: '\u2699',
};

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
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        borderRadius: '8px',
        overflow: 'hidden',
        margin: '0 8px',
        border: isActive
          ? '1px solid rgb(80, 120, 200)'
          : '1px solid rgb(50, 50, 55)',
        boxShadow: isActive
          ? '0 0 12px rgba(80, 120, 200, 0.15)'
          : 'none',
        transition: 'none',
        flexShrink: 0,
        position: 'relative',
      }}
      onClick={() => onFocus(id)}
    >
      {/* Header bar */}
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
          backgroundColor: isActive
            ? 'rgb(40, 42, 54)'
            : 'rgb(32, 32, 36)',
          borderBottom: '1px solid rgb(50, 50, 55)',
          cursor: onMove ? 'grab' : 'default',
          userSelect: 'none',
          position: 'relative',
          zIndex: 1001,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', overflow: 'hidden' }}>
          <span style={{ fontSize: '0.6rem', opacity: 0.7 }}>
            {typeIndicators[type]}
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
                color: 'rgb(220, 220, 235)',
                fontWeight: 500,
                backgroundColor: 'rgb(20, 20, 24)',
                border: '1px solid rgb(80, 120, 200)',
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
                color: 'rgb(200, 200, 210)',
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
            color: 'rgb(140, 140, 150)',
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
            (e.target as HTMLElement).style.backgroundColor = 'rgb(60, 60, 70)';
            (e.target as HTMLElement).style.color = 'rgb(220, 220, 230)';
          }}
          onMouseLeave={(e) => {
            (e.target as HTMLElement).style.backgroundColor = 'transparent';
            (e.target as HTMLElement).style.color = 'rgb(140, 140, 150)';
          }}
          title="Close pane"
        >
          &#x2715;
        </button>
      </div>

      <div
        className="pane-content"
        style={{
          flex: 1,
          overflow: 'hidden',
          backgroundColor: 'rgb(30, 30, 33)',
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
