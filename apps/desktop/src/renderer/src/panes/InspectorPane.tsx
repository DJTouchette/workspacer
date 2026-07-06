import React from 'react';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { claudeColors as colors } from '../components/claude-shared';
import { InspectorCard } from '../components/claude/InspectorCard';

interface InspectorPaneProps {
  title: string;
  isActive: boolean;
  /** The claudemon session whose live snapshot this pane renders. */
  inspectorSessionId?: string;
  /** The target agent's display name (shown as the card header). */
  inspectorAgentName?: string;
}

/**
 * The Inspector as a standalone pane: the shared {@link InspectorCard} bound to
 * one session's live snapshot, full-height. It subscribes to the owning session
 * the same way {@link AgentWatchPane} does (useClaudeSession), so it live-updates
 * for any agent — not just the one being piloted — and is purely a viewer.
 */
const InspectorPane: React.FC<InspectorPaneProps> = ({
  isActive,
  inspectorSessionId,
  inspectorAgentName,
}) => {
  const { session } = useClaudeSession({
    ptySessionId: inspectorSessionId ?? null,
    active: isActive,
  });

  if (!inspectorSessionId) {
    return (
      <div
        style={{
          height: '100%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: 24,
          textAlign: 'center',
          color: colors.mutedDim,
          fontSize: '0.75rem',
          background: 'var(--wks-bg-base)',
        }}
      >
        This inspector pane lost its target — close it and reopen from the fleet or command palette.
      </div>
    );
  }

  return (
    <div style={{ width: '100%', height: '100%', background: 'var(--wks-bg-base)' }}>
      <InspectorCard snapshot={session} agentName={inspectorAgentName} />
    </div>
  );
};

export default InspectorPane;
