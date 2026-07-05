import React from 'react';
import type { PendingApproval } from '../../types/claudeSession';
import { claudeColors as colors, approvalBtnStyle } from '../claude-shared';
import { IconApprove, IconReject } from '../wksIcons';

export const ApprovalPrompt: React.FC<{
  approval: PendingApproval;
  onRespond: (response: 'yes' | 'no') => void;
}> = ({ approval, onRespond }) => (
  <div
    style={{
      padding: '12px 14px',
      margin: '8px 0',
      borderRadius: 10,
      backgroundColor: 'rgba(248, 113, 113, 0.06)',
      border: `1px solid rgba(248, 113, 113, 0.2)`,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}
  >
    <div style={{ fontSize: '0.75rem', color: colors.error, fontWeight: 600, marginBottom: 6 }}>
      Permission Required: {approval.toolName}
    </div>
    <pre
      style={{
        fontSize: '0.7rem',
        color: 'rgb(180, 180, 200)',
        margin: '4px 0 8px 0',
        padding: 8,
        backgroundColor: 'rgba(0, 0, 0, 0.3)',
        borderRadius: 6,
        maxHeight: 120,
        overflow: 'auto',
        whiteSpace: 'pre-wrap',
        fontFamily: 'var(--claude-mono-font, monospace)',
        border: `1px solid ${colors.border}`,
      }}
    >
      {JSON.stringify(approval.toolInput, null, 2)}
    </pre>
    <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
      <button
        style={{
          ...approvalBtnStyle(colors.success),
          position: 'relative',
          zIndex: 10,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
        }}
        onClick={(e) => {
          e.stopPropagation();
          onRespond('yes');
        }}
      >
        <IconApprove size={14} strokeWidth={2.4} />
        Allow
      </button>
      <button
        style={{
          ...approvalBtnStyle(colors.error),
          position: 'relative',
          zIndex: 10,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
        }}
        onClick={(e) => {
          e.stopPropagation();
          onRespond('no');
        }}
      >
        <IconReject size={14} strokeWidth={2.4} />
        Deny
      </button>
    </div>
  </div>
);
