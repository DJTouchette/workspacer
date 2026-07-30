import React, { useEffect, useState } from 'react';
import { claudeColors as colors } from '../claude-shared';
import { fmtDuration } from './agentUtils';

/**
 * "Working for 1m 04s" — the elapsed-time label beside the streaming spinner.
 *
 * It owns its own 1s interval on purpose: the pane it lives in re-renders the
 * whole transcript, so ticking a clock from up there would repaint everything
 * once a second for one changing string. Here only this leaf re-renders.
 */
export const WorkingTimer: React.FC<{
  /** Epoch ms the current run started (the message you sent, normally). */
  since: number;
  /** Leading word. 'Working' unless the caller has better wording. */
  verb?: string;
}> = ({ since, verb = 'Working' }) => {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    // Re-sync immediately on a new run so a remounted/reused instance can't
    // show the previous run's elapsed value for up to a second.
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [since]);

  // A clock skew (or a start stamped by the daemon on another machine) must not
  // render "-3s"; the floor is 0.
  const elapsed = Math.max(0, now - since);

  return (
    <span
      // Polite, not assertive: a screen reader shouldn't announce every tick.
      aria-live="off"
      style={{
        fontSize: '0.66rem',
        color: colors.muted,
        // Tabular digits so the label doesn't jitter as the seconds roll over.
        fontVariantNumeric: 'tabular-nums',
        userSelect: 'none',
        whiteSpace: 'nowrap',
      }}
    >
      {verb} for {fmtDuration(elapsed)}
    </span>
  );
};
