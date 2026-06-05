import React from 'react';
import { claudeColors as colors } from '../claude-shared';

export const TurnDivider: React.FC<{ label?: string }> = ({ label = 'Response' }) => (
  <div style={{
    display: 'flex',
    alignItems: 'center',
    gap: 10,
    margin: '16px 0 12px 0',
  }}>
    <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
    <span style={{ fontSize: '0.6rem', color: colors.mutedDim, fontWeight: 500, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
      {label}
    </span>
    <div style={{ flex: 1, height: 1, backgroundColor: colors.divider }} />
  </div>
);
