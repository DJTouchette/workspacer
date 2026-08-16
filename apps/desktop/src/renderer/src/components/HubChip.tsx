import React from 'react';
import { Server } from 'lucide-react';

/**
 * Small "lives on peer hub X" chip for federated agents — the one marker that
 * a card/row's agent runs on another machine. Quiet tint at rest; flips to the
 * warning tone when that peer's link is down (tombstone state). Local agents
 * never render it.
 */
export const HubChip: React.FC<{
  name: string;
  offline?: boolean;
  style?: React.CSSProperties;
}> = ({ name, offline, style }) => (
  <span
    title={offline ? `hub ${name} is offline` : `on hub ${name}`}
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
      color: offline ? 'var(--wks-warning)' : 'var(--wks-text-tertiary)',
      background: offline
        ? 'color-mix(in srgb, var(--wks-warning) 10%, transparent)'
        : 'color-mix(in srgb, var(--wks-text-tertiary) 10%, transparent)',
      ...style,
    }}
  >
    <Server size={10} strokeWidth={2} style={{ flexShrink: 0 }} />
    {name}
  </span>
);
