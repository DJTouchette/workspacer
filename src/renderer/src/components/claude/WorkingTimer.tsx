import React, { useEffect, useState } from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';

// ── Working Timer ──

export const WorkingTimer: React.FC<{ session: ClaudeSessionSnapshot | null }> = ({ session }) => {
  const [elapsed, setElapsed] = useState(0);
  const isWorking = session?.ambientState === 'thinking' || session?.ambientState === 'streaming';

  useEffect(() => {
    if (!isWorking || !session) {
      setElapsed(0);
      return;
    }
    const start = session.lastActivity;
    const tick = () => setElapsed(Math.floor((Date.now() - start) / 1000));
    tick();
    const iv = setInterval(tick, 1000);
    return () => clearInterval(iv);
  }, [isWorking, session?.lastActivity]);

  if (!isWorking) return null;

  const fmt = elapsed < 60 ? `${elapsed}s` : `${Math.floor(elapsed / 60)}m ${elapsed % 60}s`;

  return (
    <span style={{ fontSize: '0.55rem', color: colors.muted, fontVariantNumeric: 'tabular-nums' }}>
      Working... {fmt}
    </span>
  );
};
