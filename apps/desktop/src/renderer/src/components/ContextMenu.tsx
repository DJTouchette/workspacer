import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Injected-once CSS for the context menu. The menu portals to <body>, which
 * lives OUTSIDE .app-root — so it never inherits the app's UI font or our
 * hover transitions. We set both here explicitly: a pop-in entrance and an
 * accent hover that nudges the item rightward (the "jazz"), an accent bar that
 * grows in on the left, and a brighter accent-tinted fill. The highlight also
 * applies on keyboard focus (:focus-visible) so arrow/tab navigation shows it.
 */
const CTX_STYLE_ID = 'wks-context-menu-styles';
function ensureContextMenuStyles(): void {
  if (typeof document === 'undefined' || document.getElementById(CTX_STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = CTX_STYLE_ID;
  style.textContent = `
@keyframes wks-ctx-pop { from { opacity: 0; transform: translateY(-6px) scale(0.97); } to { opacity: 1; transform: none; } }
.wks-ctx-menu {
  font-family: "Hanken Grotesk", "Inter", system-ui, -apple-system, sans-serif;
  animation: wks-ctx-pop 0.14s cubic-bezier(0.16, 1, 0.3, 1);
  transform-origin: top left;
}
.wks-ctx-item {
  position: relative;
  transition: background 0.12s ease, color 0.1s ease, padding-left 0.14s cubic-bezier(0.16, 1, 0.3, 1);
}
/* The accent bar — grows from the vertical center on hover/focus. */
.wks-ctx-item::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 50%;
  width: 3px;
  height: 0;
  transform: translateY(-50%);
  border-radius: 2px;
  background: var(--wks-accent);
  transition: height 0.16s cubic-bezier(0.16, 1, 0.3, 1);
  pointer-events: none;
}
.wks-ctx-item:hover:not(:disabled),
.wks-ctx-item:focus-visible:not(:disabled) {
  background: color-mix(in srgb, var(--wks-accent) 16%, transparent);
  color: var(--wks-text-primary);
  padding-left: 18px;
  outline: none;
}
.wks-ctx-item:hover:not(:disabled)::before,
.wks-ctx-item:focus-visible:not(:disabled)::before { height: 58%; }
.wks-ctx-item.wks-ctx-danger:hover:not(:disabled),
.wks-ctx-item.wks-ctx-danger:focus-visible:not(:disabled) {
  background: color-mix(in srgb, var(--wks-error) 15%, transparent);
  color: var(--wks-error);
}
.wks-ctx-item.wks-ctx-danger:hover:not(:disabled)::before,
.wks-ctx-item.wks-ctx-danger:focus-visible:not(:disabled)::before { background: var(--wks-error); }
`;
  document.head.appendChild(style);
}

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

export function ContextMenu({
  x,
  y,
  onClose,
  children,
  minWidth = 150,
  zIndex = 10000,
}: ContextMenuProps) {
  ensureContextMenuStyles();
  const ref = useRef<HTMLDivElement>(null);
  // Start at the raw anchor; clamp once we know our measured size.
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: x, top: y });
  // Keep hidden for the first paint so the unclamped position never flashes.
  const [measured, setMeasured] = useState(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;

    const clamp = () => {
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
    };

    clamp();
    // Re-clamp when the menu's size changes after mount — e.g. the model menu
    // opens as a one-row "Loading…" and grows once the list arrives. Without
    // this the grown menu keeps the small-menu position and spills downward
    // off-screen instead of flipping above the anchor.
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(clamp);
    ro.observe(el);
    return () => ro.disconnect();
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
      className="wks-ctx-menu"
      style={{
        position: 'fixed',
        left: pos.left,
        top: pos.top,
        minWidth,
        zIndex,
        padding: '5px',
        borderRadius: 'var(--wks-radius-lg, 12px)',
        background: 'var(--wks-glass-strong, var(--wks-bg-elevated))',
        backdropFilter: 'blur(var(--wks-glass-blur, 12px)) saturate(160%)',
        WebkitBackdropFilter: 'blur(var(--wks-glass-blur, 12px)) saturate(160%)',
        border: '1px solid var(--wks-glass-border, var(--wks-border-input))',
        boxShadow:
          '0 12px 34px var(--wks-glass-shadow, var(--wks-shadow)), inset 0 1px 0 var(--wks-glass-highlight)',
        fontSize: '0.76rem',
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
      className={`wks-ctx-item${danger ? ' wks-ctx-danger' : ''}`}
      disabled={disabled}
      onClick={onClick}
      style={{
        display: 'block',
        width: '100%',
        textAlign: 'left',
        padding: '6px 11px',
        borderRadius: 'var(--wks-radius-md, 7px)',
        border: 'none',
        background: 'transparent',
        color: disabled
          ? 'var(--wks-text-disabled)'
          : danger
            ? 'var(--wks-error)'
            : 'var(--wks-text-primary)',
        cursor: disabled ? 'default' : 'pointer',
        font: 'inherit',
        fontWeight: 500,
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </button>
  );
}

export function ContextMenuSeparator() {
  return (
    <div
      style={{
        height: 1,
        background: 'var(--wks-glass-border, var(--wks-border))',
        margin: '5px 7px',
      }}
    />
  );
}

export function ContextMenuLabel({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: '3px 11px 4px',
        fontFamily: 'var(--wks-font-mono, monospace)',
        fontSize: '0.55rem',
        color: 'var(--wks-text-faint)',
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
      }}
    >
      {children}
    </div>
  );
}
