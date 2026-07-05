import React from 'react';
import { claudeColors as colors } from '../claude-shared';

export const ScrollToBottomButton: React.FC<{ onClick: () => void }> = ({ onClick }) => (
  <div
    style={{
      position: 'absolute',
      bottom: 12,
      left: '50%',
      transform: 'translateX(-50%)',
      animation: 'claudeScrollBtn 0.2s ease-out',
      zIndex: 10,
    }}
  >
    <button
      onClick={onClick}
      style={{
        fontSize: '0.65rem',
        fontWeight: 500,
        padding: '4px 14px',
        borderRadius: 20,
        border: `1px solid ${colors.border}`,
        backgroundColor: 'rgba(13, 13, 16, 0.9)',
        color: colors.muted,
        cursor: 'pointer',
        backdropFilter: 'blur(8px)',
        display: 'flex',
        alignItems: 'center',
        gap: 4,
      }}
    >
      <span style={{ fontSize: '0.7rem' }}>{'↓'}</span>
      Scroll to bottom
    </button>
  </div>
);
