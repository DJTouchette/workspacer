import React from 'react';
import type { ToolCall, SubagentInfo, WorkflowRunInfo } from '../../types/claudeSession';
import { WorkLogEntry } from '../claude-shared';
import { WorkflowRunCard } from './WorkflowRunCard';
import { SubagentRow } from './SubagentRow';

/** Most recent live tool calls to render before the transcript absorbs them.
 *  Bounds the visual "pile" if a backlog builds (transcript lag, or orphaned
 *  calls); the full history still lives in the conversation timeline above. */
const MAX_TOOLCALLS_SHOWN = 12;

const InlineWorkLogInner: React.FC<{
  toolCalls: ToolCall[];
  subagents?: SubagentInfo[];
  workflows?: WorkflowRunInfo[];
}> = ({ toolCalls, subagents, workflows }) => {
  if (toolCalls.length === 0 && (!subagents || subagents.length === 0) && (!workflows || workflows.length === 0)) return null;

  const overflow = toolCalls.length - MAX_TOOLCALLS_SHOWN;
  const shownToolCalls = overflow > 0 ? toolCalls.slice(toolCalls.length - MAX_TOOLCALLS_SHOWN) : toolCalls;

  return (
    <div style={{ margin: '4px 0 6px 0', padding: '0 2px' }}>
      {workflows && workflows.map(run => <WorkflowRunCard key={run.runId} run={run} />)}
      {subagents && subagents.map(sub => <SubagentRow key={sub.id} sub={sub} />)}
      {overflow > 0 && (
        <div style={{ fontSize: 11, opacity: 0.55, padding: '2px 4px' }}>
          +{overflow} earlier step{overflow === 1 ? '' : 's'}
        </div>
      )}
      {shownToolCalls.map(tc => <WorkLogEntry key={tc.id} tc={tc} />)}
    </div>
  );
};

export const InlineWorkLog = React.memo(InlineWorkLogInner);
