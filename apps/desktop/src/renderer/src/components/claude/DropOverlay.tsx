import React from 'react';
import { claudeColors as colors } from '../claude-shared';

export const DropOverlay: React.FC = () => (
  <div
    style={{
      position: 'absolute',
      inset: 0,
      zIndex: 50,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: 'rgba(0, 0, 0, 0.6)',
      backdropFilter: 'blur(4px)',
      border: `2px dashed ${colors.accent}`,
      borderRadius: 'var(--wks-radius-md)',
      pointerEvents: 'none',
    }}
  >
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: '1.5rem', marginBottom: 8, opacity: 0.7 }}>{'📎'}</div>
      <div style={{ fontSize: '0.8rem', color: colors.accent, fontWeight: 600 }}>
        Drop files here
      </div>
      <div style={{ fontSize: '0.65rem', color: colors.muted, marginTop: 4 }}>
        Images, code, PDFs — any file Claude can read
      </div>
    </div>
  </div>
);
