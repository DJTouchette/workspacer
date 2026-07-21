import React from 'react';
import { ChevronDown } from 'lucide-react';
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
        fontSize: '0.68rem',
        fontWeight: 500,
        padding: '4px 14px',
        borderRadius: 'var(--wks-radius-pill)',
        border: `1px solid ${colors.border}`,
        backgroundColor: 'var(--wks-glass-bg)',
        color: colors.muted,
        cursor: 'pointer',
        backdropFilter: 'blur(8px)',
        display: 'flex',
        alignItems: 'center',
        gap: 4,
      }}
    >
      <ChevronDown size={12} strokeWidth={2} />
      Scroll to bottom
    </button>
  </div>
);
