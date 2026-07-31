import React, { useCallback, useEffect, useRef } from 'react';
import { clampSidebarWidth, SIDEBAR_DEFAULT_WIDTH } from '../lib/sidebarWidth';

interface Props {
  /** Current width — the drag starts from here. */
  width: number;
  /** Live width while dragging (every frame). Cheap: layout only. */
  onResize: (px: number) => void;
  /** Settled width, worth persisting. Fires on release / after a key repeat. */
  onCommit: (px: number) => void;
}

/** Keyboard nudge per arrow press (Shift = a bigger step). */
const KEY_STEP = 8;
const KEY_STEP_LARGE = 32;
/** How long a keyboard flurry is allowed to run before it's written to config. */
const KEY_COMMIT_DELAY = 400;

/**
 * The sidebar's right-edge drag handle. A 7px hit strip (wider than the 1px line
 * it paints on hover) sitting over the sidebar's border, driving width live and
 * committing once the gesture settles — dragging must not write config.yaml on
 * every mouse move.
 *
 * Pointer capture, not window listeners: the pointer keeps reporting to the
 * handle even when it outruns the cursor over a webview pane (a browser/plugin
 * pane would otherwise swallow the events and the drag would stick).
 */
export const SidebarResizeHandle: React.FC<Props> = ({ width, onResize, onCommit }) => {
  // Gesture state in refs — a drag must not re-render this component per frame.
  const dragRef = useRef<{ startX: number; startWidth: number; latest: number } | null>(null);
  const frameRef = useRef<number | null>(null);
  const keyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Latest width, so a keyboard nudge reads the current value without making
  // the key handler depend on (and re-bind to) every width change.
  const widthRef = useRef(width);
  widthRef.current = width;

  useEffect(
    () => () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      if (keyTimerRef.current) clearTimeout(keyTimerRef.current);
      // The drag writes document.body styles that only endDrag clears — and a
      // drag can outlive this component (collapsing the sidebar with Ctrl+B
      // mid-drag unmounts the handle, and pointer capture means no other
      // element sees the pointerup). Leaving them set makes the whole app
      // unselectable with a col-resize cursor until reload.
      if (dragRef.current) {
        document.body.style.userSelect = '';
        document.body.style.cursor = '';
      }
    },
    [],
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      // Left button / primary touch only — a right-click here belongs to the
      // sidebar's own context menu.
      if (e.button !== 0) return;
      e.preventDefault();
      dragRef.current = { startX: e.clientX, startWidth: width, latest: width };
      // jsdom has no pointer capture; the drag still works via the same handlers.
      e.currentTarget.setPointerCapture?.(e.pointerId);
      // Kill text selection + keep the resize cursor while the pointer roams
      // over panes that would otherwise assert their own.
      document.body.style.userSelect = 'none';
      document.body.style.cursor = 'col-resize';
    },
    [width],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag) return;
      drag.latest = clampSidebarWidth(
        drag.startWidth + (e.clientX - drag.startX),
        window.innerWidth,
      );
      // One layout write per frame, however fast the pointer streams events.
      if (frameRef.current !== null) return;
      frameRef.current = requestAnimationFrame(() => {
        frameRef.current = null;
        if (dragRef.current) onResize(dragRef.current.latest);
      });
    },
    [onResize],
  );

  const endDrag = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag) return;
      dragRef.current = null;
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      e.currentTarget.releasePointerCapture?.(e.pointerId);
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
      onResize(drag.latest);
      // Only the settled width reaches config — and only if it actually moved.
      if (drag.latest !== drag.startWidth) onCommit(drag.latest);
    },
    [onResize, onCommit],
  );

  const nudge = useCallback(
    (delta: number) => {
      const next = clampSidebarWidth(widthRef.current + delta, window.innerWidth);
      onResize(next);
      // Coalesce a held arrow key into one write once the flurry stops.
      if (keyTimerRef.current) clearTimeout(keyTimerRef.current);
      keyTimerRef.current = setTimeout(() => onCommit(next), KEY_COMMIT_DELAY);
    },
    [onResize, onCommit],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const step = e.shiftKey ? KEY_STEP_LARGE : KEY_STEP;
      if (e.key === 'ArrowLeft') nudge(-step);
      else if (e.key === 'ArrowRight') nudge(step);
      else if (e.key === 'Home' || e.key === 'Enter') {
        // Same escape hatch as the double-click: back to the shipped width.
        onResize(SIDEBAR_DEFAULT_WIDTH);
        onCommit(SIDEBAR_DEFAULT_WIDTH);
      } else return;
      e.preventDefault();
    },
    [nudge, onResize, onCommit],
  );

  return (
    <div
      className="wks-sidebar-resize"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      aria-valuenow={width}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onKeyDown}
      onDoubleClick={() => {
        onResize(SIDEBAR_DEFAULT_WIDTH);
        onCommit(SIDEBAR_DEFAULT_WIDTH);
      }}
      style={{ left: width - 3 }}
    >
      <span className="wks-sidebar-resize-line" />
    </div>
  );
};
