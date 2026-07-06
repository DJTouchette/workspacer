import React from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { InspectorCard } from './InspectorCard';

/**
 * Right-hand inspector for the Claude pane GUI view: a persistent home for the
 * session's plan, files, agents, and usage — so workflow/agent state doesn't
 * live only in the scrollback. It is the docked chrome (fixed width + left
 * border + a close control) around the shared {@link InspectorCard}, which owns
 * the tab strip and all tab bodies; the same card also powers the standalone
 * inspector pane, the Fleet-Deck card expansion and the sidebar hover peek.
 */
export const InspectorRail: React.FC<{
  session: ClaudeSessionSnapshot | null;
  onClose: () => void;
}> = ({ session, onClose }) => {
  return (
    <div
      style={{
        width: 320,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        borderLeft: `1px solid ${colors.border}`,
        backgroundColor: 'rgba(255,255,255,0.012)',
        overflow: 'hidden',
      }}
    >
      <InspectorCard
        snapshot={session}
        headerAccessory={
          <button
            onClick={onClose}
            title="Hide inspector"
            style={{
              fontSize: '0.8rem',
              border: 'none',
              background: 'transparent',
              color: colors.mutedDim,
              cursor: 'pointer',
              padding: '2px 6px',
            }}
          >
            ×
          </button>
        }
      />
    </div>
  );
};
