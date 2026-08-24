import React from 'react';
import { Unlink2 } from 'lucide-react';

/**
 * Marks a worker whose dispatcher card is gone — a dangling `parentId` (see
 * AgentWorkspace.parentId / SideBar's `rootOf`). The worker itself is fine;
 * nobody just happens to be listening for its result. Never a click target —
 * adoption is a manager tool call (`adopt_workers`), not a UI button, because
 * picking the wrong predecessor silently re-points a live worker's wakes.
 *
 * `confirmed` distinguishes a proven Fleet Manager dispatcher (recorded at
 * adopt time via `dispatchedByManager`) from a merely-gone parent that was
 * never confirmed to be a manager at all — same chip, honest tooltip.
 */
export const OrphanChip: React.FC<{ confirmed: boolean; style?: React.CSSProperties }> = ({
  confirmed,
  style,
}) => (
  <span
    title={
      confirmed
        ? "Its manager session ended — nobody is watching for this worker's result. " +
          'A new manager can adopt it (list_orphans / adopt_workers).'
        : 'The agent that dispatched this worker is no longer here — it may be unwatched. ' +
          'If it was dispatched by a manager, adopt_workers can re-attach it.'
    }
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 4,
      flexShrink: 0,
      padding: '1px 6px',
      borderRadius: 'var(--wks-radius-pill)',
      fontFamily: 'var(--wks-font-mono)',
      fontSize: '0.6rem',
      lineHeight: 1.6,
      whiteSpace: 'nowrap',
      color: 'var(--wks-text-tertiary)',
      background: 'color-mix(in srgb, var(--wks-text-tertiary) 10%, transparent)',
      ...style,
    }}
  >
    <Unlink2 size={10} strokeWidth={2} style={{ flexShrink: 0 }} />
    Unwatched
  </span>
);
