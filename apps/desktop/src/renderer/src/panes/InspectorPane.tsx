import React, { useState } from 'react';
import TaskInspector from '../components/TaskInspector';
import { SmallButton } from '../components/settings/primitives';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { claudeColors as colors } from '../components/claude-shared';
import { InspectorCard } from '../components/claude/InspectorCard';

interface InspectorPaneProps {
  title: string;
  isActive: boolean;
  /** The claudemon session whose live snapshot this pane renders. */
  inspectorSessionId?: string;
  inspectorTaskId?: string;
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
  inspectorTaskId,
}) => {
  const [tasks, setTasks] = useState(!!inspectorTaskId);
  const { session } = useClaudeSession({
    ptySessionId: inspectorSessionId ?? null,
    active: isActive,
  });

  if (!inspectorSessionId && !inspectorTaskId) {
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
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
        background: 'var(--wks-bg-base)',
      }}
    >
      <div style={{ padding: 8 }}>
        <SmallButton
          label="Session"
          disabled={!inspectorSessionId}
          onClick={() => setTasks(false)}
        />{' '}
        <SmallButton label="Tasks" onClick={() => setTasks(true)} />
      </div>
      {tasks ? (
        <TaskInspector
          projectCwd={session?.cwd}
          taskId={inspectorTaskId}
          sessionId={inspectorSessionId}
          remote={!!session?.hub}
        />
      ) : (
        <InspectorCard
          snapshot={session}
          sessionId={inspectorSessionId}
          agentName={inspectorAgentName}
        />
      )}
    </div>
  );
};

export default InspectorPane;
