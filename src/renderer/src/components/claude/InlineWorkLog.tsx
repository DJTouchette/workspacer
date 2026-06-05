import React from 'react';
import type { ToolCall, SubagentInfo, WorkflowRunInfo } from '../../types/claudeSession';
import { WorkLogEntry } from '../claude-shared';
import { WorkflowRunCard } from './WorkflowRunCard';
import { SubagentRow } from './SubagentRow';

export const InlineWorkLog: React.FC<{
  toolCalls: ToolCall[];
  subagents?: SubagentInfo[];
  workflows?: WorkflowRunInfo[];
}> = ({ toolCalls, subagents, workflows }) => {
  if (toolCalls.length === 0 && (!subagents || subagents.length === 0) && (!workflows || workflows.length === 0)) return null;

  return (
    <div style={{ margin: '4px 0 6px 0', padding: '0 2px' }}>
      {workflows && workflows.map(run => <WorkflowRunCard key={run.runId} run={run} />)}
      {subagents && subagents.map(sub => <SubagentRow key={sub.id} sub={sub} />)}
      {toolCalls.map(tc => <WorkLogEntry key={tc.id} tc={tc} />)}
    </div>
  );
};
