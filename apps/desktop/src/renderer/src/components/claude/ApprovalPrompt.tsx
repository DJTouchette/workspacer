import React from 'react';
import type { PendingApproval } from '../../types/claudeSession';
import { claudeColors as colors, approvalBtnStyle } from '../claude-shared';
import { IconApprove, IconReject } from '../wksIcons';

/**
 * Approval resolver. It is always rendered *inside* a card that already frames
 * it (AttentionCard's raised `Surface`, NeedsYouDock's panel), so this
 * component is deliberately NOT a surface of its own — no fill, no border, just
 * layout. The only nested surface it draws is the tool-input `<pre>`, which
 * takes the flat channel (border, no fill) per the border-or-fill rule in
 * `Surface.tsx`. That keeps the whole card two surfaces deep instead of three.
 */
export const ApprovalPrompt: React.FC<{
  approval: PendingApproval;
  onRespond: (response: 'yes' | 'no') => void;
}> = ({ approval, onRespond }) => (
  <div
    style={{
      margin: '4px 0 0',
      animation: 'claudeFadeIn 0.2s ease-out',
    }}
  >
    <div
      style={{
        fontSize: 'calc(0.8rem * var(--claude-gui-font-scale, 1))',
        color: colors.error,
        fontWeight: 600,
        marginBottom: 6,
      }}
    >
      Permission Required: {approval.toolName}
    </div>
    <pre
      style={{
        fontSize: 'calc(0.72rem * var(--claude-gui-font-scale, 1))',
        color: colors.text,
        margin: '4px 0 8px 0',
        padding: 8,
        // Flat nesting: the outline is this block's only separation channel —
        // a fill here would stack on the enclosing card's own.
        backgroundColor: 'transparent',
        border: `1px solid ${colors.borderSubtle}`,
        borderRadius: 'var(--wks-radius-sm)',
        maxHeight: 120,
        overflow: 'auto',
        whiteSpace: 'pre-wrap',
        fontFamily: 'var(--claude-mono-font, monospace)',
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
