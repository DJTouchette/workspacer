import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { ToolCall, SubagentInfo, WorkflowRunInfo } from '../../types/claudeSession';
import { claudeColors as colors, WorkLogEntry } from '../claude-shared';
import { DiffView, hasDiff } from './DiffView';
import { SubagentRow } from './SubagentRow';
import { WorkflowRunCard } from './WorkflowRunCard';
import { AgentSpinner } from './WorkflowAgentRow';
import { requestOpenInEditor } from '../../lib/editorBus';

const EDIT_TOOLS = new Set(['Edit', 'MultiEdit', 'NotebookEdit']);
const CMD_TOOLS = new Set(['Bash', 'PowerShell']);
const SEARCH_TOOLS = new Set(['Grep', 'Glob']);

interface WorkSummary {
  text: string;
  added: number;
  removed: number;
  failed: number;
  editedFiles: string[];
}

/** Collapse a run of tool calls into a one-line human summary. */
function summarizeWork(calls: ToolCall[]): WorkSummary {
  const editedFiles = new Set<string>();
  let reads = 0, cmds = 0, searches = 0, agents = 0, workflows = 0, other = 0;
  let added = 0, removed = 0, failed = 0;

  for (const tc of calls) {
    if (tc.status === 'failed') failed++;
    if (EDIT_TOOLS.has(tc.name)) {
      if (tc.input?.file_path) editedFiles.add(tc.input.file_path);
      const old = tc.input?.old_string ?? '';
      const nw = tc.input?.new_string ?? '';
      if (old) removed += old.split('\n').length;
      if (nw) added += nw.split('\n').length;
    } else if (tc.name === 'Write') {
      if (tc.input?.file_path) editedFiles.add(tc.input.file_path);
      if (tc.input?.content) added += tc.input.content.split('\n').length;
    } else if (tc.name === 'Read') reads++;
    else if (CMD_TOOLS.has(tc.name)) cmds++;
    else if (SEARCH_TOOLS.has(tc.name)) searches++;
    else if (tc.name === 'Agent') agents++;
    else if (tc.name === 'Workflow') workflows++;
    else other++;
  }

  const parts: string[] = [];
  if (editedFiles.size > 0) parts.push(`${editedFiles.size} file${editedFiles.size !== 1 ? 's' : ''} changed`);
  if (cmds > 0) parts.push(`${cmds} command${cmds !== 1 ? 's' : ''}`);
  if (reads > 0) parts.push(`read ${reads}`);
  if (searches > 0) parts.push(`${searches} search${searches !== 1 ? 'es' : ''}`);
  if (workflows > 0) parts.push(`${workflows} workflow${workflows !== 1 ? 's' : ''}`);
  if (agents > 0) parts.push(`${agents} agent${agents !== 1 ? 's' : ''}`);
  if (other > 0) parts.push(`${other} other`);

  return { text: parts.join(' · '), added, removed, failed, editedFiles: [...editedFiles] };
}

/**
 * A collapsed run of consecutive tool calls in the timeline — "what Claude
 * did between saying things". Expands to the individual steps with inline
 * diffs; subagent and workflow runs render as rich cards anchored at the
 * tool call that spawned them.
 *
 * `live` marks the most recent card while Claude is still working: it
 * defaults to expanded, then auto-collapses once when work moves on — except
 * for cards that contain a workflow/subagent run, which stay collapsed so the
 * rich run cards (not a flood of raw tool calls) are what you see while a
 * workflow is going.
 */
const WorkCardInner: React.FC<{
  toolCalls: ToolCall[];
  subagentByToolId?: Map<string, SubagentInfo>;
  workflowByToolId?: Map<string, WorkflowRunInfo>;
  live?: boolean;
  cwd?: string;
}> = ({ toolCalls, subagentByToolId, workflowByToolId, live, cwd }) => {
  // A card that spawned a workflow/subagent run shows that run via its rich
  // card (surfaced even when collapsed); don't auto-expand into the raw tool
  // list, which is the "bunch of tool calls" flood during a workflow.
  const hasOrchestration = useMemo(
    () => toolCalls.some(tc => workflowByToolId?.has(tc.id) || subagentByToolId?.has(tc.id)),
    [toolCalls, workflowByToolId, subagentByToolId],
  );
  const [expanded, setExpanded] = useState(!!live && !hasOrchestration);
  const [filesOpen, setFilesOpen] = useState(false);
  const wasLive = useRef(!!live);
  useEffect(() => {
    if (wasLive.current && !live) { setExpanded(false); setFilesOpen(false); }
    wasLive.current = !!live;
  }, [live]);

  const summary = useMemo(() => summarizeWork(toolCalls), [toolCalls]);
  const anyRunning = live || toolCalls.some(tc => tc.status === 'running');

  // Running agent/workflow cards surface even while the card is collapsed —
  // ongoing parallel work shouldn't hide behind a closed disclosure.
  const visibleWhenCollapsed = useMemo(() => {
    if (expanded) return [];
    const out: React.ReactNode[] = [];
    for (const tc of toolCalls) {
      const wf = workflowByToolId?.get(tc.id);
      if (wf && wf.status === 'running') out.push(<WorkflowRunCard key={`wf-${tc.id}`} run={wf} />);
      const sub = subagentByToolId?.get(tc.id);
      if (sub && sub.status === 'running') out.push(<SubagentRow key={`sub-${tc.id}`} sub={sub} />);
    }
    return out;
  }, [expanded, toolCalls, subagentByToolId, workflowByToolId]);

  return (
    <div style={{
      margin: '4px 0 10px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.015)',
      overflow: 'hidden',
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      <div
        onClick={() => setExpanded(e => !e)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '5px 10px',
          cursor: 'pointer',
          userSelect: 'none',
          fontSize: '0.7rem',
          color: colors.muted,
        }}
      >
        {anyRunning ? <AgentSpinner /> : (
          <span style={{
            color: summary.failed > 0 ? colors.error : colors.success,
            fontSize: '0.7rem',
            width: 12,
            textAlign: 'center',
            flexShrink: 0,
          }}>
            {summary.failed > 0 ? '✗' : '✓'}
          </span>
        )}
        <span style={{ color: colors.text, fontWeight: 600, flexShrink: 0 }}>
          {toolCalls.length} step{toolCalls.length !== 1 ? 's' : ''}
        </span>
        {summary.editedFiles.length > 0 && (
          <span
            onClick={e => { e.stopPropagation(); setFilesOpen(f => !f); }}
            title={filesOpen ? 'Hide changed files' : 'Show changed files'}
            style={{
              color: filesOpen ? colors.accent : colors.muted,
              cursor: 'pointer',
              flexShrink: 0,
              textDecoration: 'none',
              borderBottom: `1px dotted ${filesOpen ? colors.accent : colors.mutedDim}`,
            }}
          >
            {summary.editedFiles.length} file{summary.editedFiles.length !== 1 ? 's' : ''} changed
          </span>
        )}
        {/* Rest of summary (commands, reads, searches, etc.) excluding the files-changed part */}
        {(() => {
          const rest = summary.text
            .split(' · ')
            .filter(p => !/^\d+ files? changed$/.test(p))
            .join(' · ');
          return rest ? (
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
              {rest}
            </span>
          ) : null;
        })()}
        <div style={{ flex: 1 }} />
        {(summary.added > 0 || summary.removed > 0) && (
          <span style={{ fontFamily: 'var(--claude-mono-font, monospace)', fontSize: '0.62rem', flexShrink: 0 }}>
            {summary.added > 0 && <span style={{ color: colors.success }}>+{summary.added}</span>}
            {summary.added > 0 && summary.removed > 0 && ' '}
            {summary.removed > 0 && <span style={{ color: colors.error }}>−{summary.removed}</span>}
          </span>
        )}
        <span style={{ color: colors.mutedDim, fontSize: '0.6rem', flexShrink: 0 }}>{expanded ? '▾' : '▸'}</span>
      </div>

      {filesOpen && !expanded && summary.editedFiles.length > 0 && (
        <div style={{
          borderTop: `1px solid ${colors.borderSubtle}`,
          padding: '4px 10px 6px 10px',
          display: 'flex',
          flexDirection: 'column',
          gap: 1,
        }}>
          {summary.editedFiles.map(filePath => {
            const basename = filePath.replace(/\\/g, '/').split('/').pop() ?? filePath;
            const dir = filePath.replace(/\\/g, '/').split('/').slice(0, -1).join('/');
            return (
              <button
                key={filePath}
                title={filePath}
                onClick={() => requestOpenInEditor({ path: filePath, cwd })}
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 6,
                  background: 'none',
                  border: 'none',
                  padding: '2px 4px',
                  borderRadius: 4,
                  cursor: 'pointer',
                  textAlign: 'left',
                  width: '100%',
                  color: colors.accent,
                  fontSize: '0.68rem',
                  fontFamily: 'var(--claude-mono-font, monospace)',
                  transition: 'background 0.1s',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = 'rgba(255,255,255,0.06)'; }}
                onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = 'none'; }}
              >
                <span style={{ flexShrink: 0 }}>{basename}</span>
                {dir && (
                  <span style={{
                    color: colors.mutedDim,
                    fontSize: '0.6rem',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    minWidth: 0,
                  }}>
                    {dir}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}

      {visibleWhenCollapsed.length > 0 && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {visibleWhenCollapsed}
        </div>
      )}

      {expanded && (
        <div style={{ padding: '2px 10px 8px 10px', borderTop: `1px solid ${colors.borderSubtle}` }}>
          {toolCalls.map(tc => {
            const wf = workflowByToolId?.get(tc.id);
            if (wf) return <WorkflowRunCard key={tc.id} run={wf} />;
            const sub = subagentByToolId?.get(tc.id);
            if (sub) return <SubagentRow key={tc.id} sub={sub} />;
            return (
              <React.Fragment key={tc.id}>
                <WorkLogEntry tc={tc} />
                {hasDiff(tc) && (
                  <DiffView
                    oldStr={tc.input?.old_string ?? ''}
                    newStr={tc.input?.new_string ?? ''}
                    filePath={tc.input?.file_path}
                  />
                )}
              </React.Fragment>
            );
          })}
        </div>
      )}
    </div>
  );
};

export const WorkCard = React.memo(WorkCardInner);
