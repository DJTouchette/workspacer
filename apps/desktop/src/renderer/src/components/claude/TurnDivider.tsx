import React from 'react';
import { claudeColors as colors } from '../claude-shared';

/** Hairline between turns. `label={null}` renders a plain line (used above
 *  user messages, where "Response" would be wrong). */
export const TurnDivider: React.FC<{ label?: string | null }> = ({ label = 'Response' }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      margin: '16px 0 12px 0',
    }}
  >
    <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
    {label && (
      <>
        <span
          style={{
            fontSize: '0.64rem',
            color: colors.mutedDim,
            fontWeight: 500,
            textTransform: 'uppercase',
            letterSpacing: '0.05em',
          }}
        >
          {label}
        </span>
        <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
      </>
    )}
  </div>
);
