import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Viewport-clamped context menu. Anchors at a point (typically the mouse
 * `clientX`/`clientY` of a right-click) and shifts/flips itself so it never
 * renders off-screen — the long-standing bug where menus opened near the
 * right or bottom edge appeared "far to the right" or not at all.
 *
 * Handles outside-click and Escape to close. Render whatever items you like
 * as children, or use the `ContextMenuItem` / `ContextMenuSeparator` /
 * `ContextMenuLabel` helpers for the standard look.
 */
export interface ContextMenuProps {
  /** Anchor X in viewport coords (e.g. mouse clientX). */
  x: number;
  /** Anchor Y in viewport coords (e.g. mouse clientY). */
  y: number;
  onClose: () => void;
  children: React.ReactNode;
  /** Min width of the menu surface. Default 150px. */
  minWidth?: number;
  /** zIndex. Default 10000. */
  zIndex?: number;
}

const VIEWPORT_MARGIN = 6;

export function ContextMenu({ x, y, onClose, children, minWidth = 150, zIndex = 10000 }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  // Start at the raw anchor; clamp once we know our measured size.
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: x, top: y });
  // Keep hidden for the first paint so the unclamped position never flashes.
  const [measured, setMeasured] = useState(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let left = x;
    let top = y;
    // Overflow right → flip to the left of the anchor (or clamp).
    if (left + rect.width + VIEWPORT_MARGIN > vw) {
      left = Math.max(VIEWPORT_MARGIN, x - rect.width);
      if (left + rect.width + VIEWPORT_MARGIN > vw) left = vw - rect.width - VIEWPORT_MARGIN;
    }
    // Overflow bottom → flip above the anchor (or clamp).
    if (top + rect.height + VIEWPORT_MARGIN > vh) {
      top = Math.max(VIEWPORT_MARGIN, y - rect.height);
      if (top + rect.height + VIEWPORT_MARGIN > vh) top = vh - rect.height - VIEWPORT_MARGIN;
    }
    left = Math.max(VIEWPORT_MARGIN, left);
    top = Math.max(VIEWPORT_MARGIN, top);

    setPos({ left, top });
    setMeasured(true);
  }, [x, y]);

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    // capture so we beat element-level handlers and close reliably
    window.addEventListener('mousedown', onPointerDown, true);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('mousedown', onPointerDown, true);
      window.removeEventListener('keydown', onKey, true);
    };
  }, [onClose]);

  // Portal to <body> so the menu escapes any ancestor stacking context or
  // `transform`/`backdrop-filter` containing block (the sidebar & navbar both
  // use frosted-glass blur). Without this, `position: fixed` is measured
  // relative to that ancestor — which slid the navbar's tab menu into the
  // middle of the screen — and the menu's z-index is trapped below sibling
  // panes instead of floating above everything.
  return createPortal(
    <div
      ref={ref}
      role="menu"
      style={{
        position: 'fixed',
        left: pos.left,
        top: pos.top,
        minWidth,
        zIndex,
        padding: '4px 0',
        borderRadius: 'var(--wks-radius-md, 6px)',
        background: 'var(--wks-bg-surface)',
        border: '1px solid var(--wks-border-input)',
        boxShadow: '0 6px 24px var(--wks-shadow)',
        fontSize: '0.74rem',
        // Hide until clamped so the off-screen first paint is never visible.
        visibility: measured ? 'visible' : 'hidden',
      }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {children}
    </div>,
    document.body,
  );
}

export function ContextMenuItem({
  label,
  onClick,
  danger,
  disabled,
}: {
  label: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      style={{
        display: 'block',
        width: '100%',
        textAlign: 'left',
        padding: '5px 12px',
        border: 'none',
        background: 'transparent',
        color: disabled
          ? 'var(--wks-text-disabled)'
          : danger
            ? 'var(--wks-error)'
            : 'var(--wks-text-primary)',
        cursor: disabled ? 'default' : 'pointer',
        font: 'inherit',
        whiteSpace: 'nowrap',
      }}
      onMouseEnter={(e) => {
        if (!disabled) (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.background = 'transparent';
      }}
    >
      {label}
    </button>
  );
}

export function ContextMenuSeparator() {
  return <div style={{ height: 1, background: 'var(--wks-border)', margin: '4px 0' }} />;
}

export function ContextMenuLabel({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: '2px 12px',
        fontSize: '0.55rem',
        color: 'var(--wks-text-disabled)',
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
      }}
    >
      {children}
    </div>
  );
}
