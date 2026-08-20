/**
 * PaneHints — tmux display-panes for the command layer (`prefix d`).
 *
 * Big numbered badges over each pane of the active tab; App owns the
 * transient state and the digit listener (the next 1–9 focuses that pane,
 * anything else dismisses). Rendered as a fixed overlay measured off the live
 * `[data-pane-id]` wrappers, so no pane-tree component needs to know hints
 * exist; the state lives a keystroke, so a one-shot measure is enough.
 */
import React, { useMemo } from 'react';

const PaneHints: React.FC<{ paneIds: string[] }> = ({ paneIds }) => {
  const spots = useMemo(
    () =>
      paneIds.flatMap((id, i) => {
        const el = document.querySelector(`[data-pane-id="${id}"]`);
        if (!(el instanceof HTMLElement) || el.offsetParent === null) return [];
        const r = el.getBoundingClientRect();
        if (r.width < 40 || r.height < 40) return [];
        return [{ id, n: i + 1, left: r.left + r.width / 2, top: r.top + r.height / 2 }];
      }),
    [paneIds],
  );
  if (spots.length === 0) return null;
  return (
    <div aria-hidden style={{ position: 'fixed', inset: 0, zIndex: 400, pointerEvents: 'none' }}>
      {spots.map((s) => (
        <div
          key={s.id}
          style={{
            position: 'fixed',
            left: s.left,
            top: s.top,
            transform: 'translate(-50%, -50%)',
            minWidth: 54,
            padding: '6px 16px',
            textAlign: 'center',
            fontFamily: 'var(--wks-font-mono)',
            fontSize: '1.6rem',
            fontWeight: 700,
            color: 'var(--wks-text-on-accent)',
            backgroundColor: 'var(--wks-accent)',
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 'var(--wks-radius-md)',
            boxShadow: '0 12px 36px var(--wks-glass-shadow)',
          }}
        >
          {s.n}
        </div>
      ))}
    </div>
  );
};

export default PaneHints;
