import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { ToolCall, SubagentInfo, WorkflowRunInfo } from '../../types/claudeSession';
import { claudeColors as colors, WorkLogEntry } from '../claude-shared';
import { DiffView, PatchDiffView, hasDiff, ReadView, hasRead } from './DiffView';
import { SubagentRow } from './SubagentRow';
import { WorkflowRunCard } from './WorkflowRunCard';
import { AgentSpinner } from './WorkflowAgentRow';
import { FileLink } from './FileLink';
import { patchLineCounts } from '../../lib/turnChanges';

// Tool-name sets span providers: claude Edit/Bash/Grep · codex
// apply_patch/shell/exec_command/web_search · opencode/pi patch.
const EDIT_TOOLS = new Set(['Edit', 'MultiEdit', 'NotebookEdit', 'apply_patch', 'patch']);
const CMD_TOOLS = new Set(['Bash', 'PowerShell', 'shell', 'exec_command']);
const SEARCH_TOOLS = new Set(['Grep', 'Glob', 'web_search', 'WebSearch']);

interface WorkSummary {
  text: string;
  added: number;
  removed: number;
  failed: number;
  editedFiles: string[];
}

/** Collapse a run of tool calls into a one-line human summary. */
export function summarizeWork(calls: ToolCall[]): WorkSummary {
  const editedFiles = new Set<string>();
  let reads = 0,
    cmds = 0,
    searches = 0,
    agents = 0,
    workflows = 0,
    other = 0;
  let added = 0,
    removed = 0,
    failed = 0;

  for (const tc of calls) {
    if (tc.status === 'failed') failed++;
    if (EDIT_TOOLS.has(tc.name)) {
      const filePath = tc.input?.file_path ?? tc.input?.path;
      if (filePath) editedFiles.add(filePath);
      // Codex apply_patch may touch several files in one call — `changes`
      // carries them all (`[{ path, kind, diff }]`).
      if (Array.isArray(tc.input?.changes)) {
        for (const ch of tc.input.changes) {
          if (typeof ch?.path === 'string' && ch.path) editedFiles.add(ch.path);
        }
      }
      if (typeof tc.input?.diff === 'string') {
        // apply_patch-style input: count the patch's +/- lines.
        const counts = patchLineCounts(tc.input.diff);
        added += counts.added;
        removed += counts.removed;
      } else {
        // MultiEdit carries its changes in an `edits` array rather than top-level
        // old_string/new_string — sum across each sub-edit.
        const edits = Array.isArray(tc.input?.edits)
          ? tc.input.edits
          : [{ old_string: tc.input?.old_string, new_string: tc.input?.new_string }];
        for (const e of edits) {
          const old = e?.old_string ?? '';
          const nw = e?.new_string ?? '';
          if (old) removed += old.split('\n').length;
          if (nw) added += nw.split('\n').length;
        }
      }
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
  if (editedFiles.size > 0)
    parts.push(`${editedFiles.size} file${editedFiles.size !== 1 ? 's' : ''} changed`);
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
 * `live` marks the most recent card while Claude is still working and `isLast`
 * marks the most recent card overall; either keeps the card expanded so the
 * latest step stays open at a glance. A card auto-collapses once it's
 * superseded (a newer card appears). Cards that contain a workflow/subagent run
 * stay collapsed regardless, so the rich run cards (not a flood of raw tool
 * calls) are what you see while a workflow is going.
 */
const WorkCardInner: React.FC<{
  toolCalls: ToolCall[];
  subagentByToolId?: Map<string, SubagentInfo>;
  workflowByToolId?: Map<string, WorkflowRunInfo>;
  live?: boolean;
  /** The most recent work card in the timeline — kept open after work ends. */
  isLast?: boolean;
  cwd?: string;
}> = ({ toolCalls, subagentByToolId, workflowByToolId, live, isLast, cwd }) => {
  // A card that spawned a workflow/subagent run shows that run via its rich
  // card (surfaced even when collapsed); don't auto-expand into the raw tool
  // list, which is the "bunch of tool calls" flood during a workflow.
  const hasOrchestration = useMemo(
    () => toolCalls.some((tc) => workflowByToolId?.has(tc.id) || subagentByToolId?.has(tc.id)),
    [toolCalls, workflowByToolId, subagentByToolId],
  );
  // Open while this is the live/last card; collapse once a newer card supersedes
  // it. (Orchestration cards stay closed so their rich run cards lead instead.)
  const active = (!!live || !!isLast) && !hasOrchestration;
  const [expanded, setExpanded] = useState(active);
  const [filesOpen, setFilesOpen] = useState(false);
  const wasActive = useRef(active);
  useEffect(() => {
    if (wasActive.current && !active) {
      setExpanded(false);
      setFilesOpen(false);
    } else if (!wasActive.current && active) {
      setExpanded(true);
    }
    wasActive.current = active;
  }, [active]);

  const summary = useMemo(() => summarizeWork(toolCalls), [toolCalls]);
  const anyRunning = live || toolCalls.some((tc) => tc.status === 'running');

  // Running agent/workflow cards surface even while the card is collapsed —
  // ongoing parallel work shouldn't hide behind a closed disclosure. File
  // edits surface too: the diff is what the user reviews, so collapsing a run
  // must not bury it.
  const visibleWhenCollapsed = useMemo(() => {
    if (expanded) return [];
    const out: React.ReactNode[] = [];
    for (const tc of toolCalls) {
      const wf = workflowByToolId?.get(tc.id);
      if (wf && wf.status === 'running') out.push(<WorkflowRunCard key={`wf-${tc.id}`} run={wf} />);
      const sub = subagentByToolId?.get(tc.id);
      if (sub && sub.status === 'running') out.push(<SubagentRow key={`sub-${tc.id}`} sub={sub} />);
      if (hasDiff(tc)) {
        out.push(
          <React.Fragment key={`edit-${tc.id}`}>
            <WorkLogEntry tc={tc} />
            {typeof tc.input?.diff === 'string' ? (
              <PatchDiffView
                patch={tc.input.diff}
                filePath={tc.input?.file_path ?? tc.input?.path}
                cwd={cwd}
              />
            ) : (
              <DiffView
                oldStr={tc.input?.old_string ?? ''}
                newStr={tc.input?.new_string ?? ''}
                filePath={tc.input?.file_path}
                cwd={cwd}
              />
            )}
          </React.Fragment>,
        );
      }
    }
    return out;
  }, [expanded, toolCalls, subagentByToolId, workflowByToolId, cwd]);

  return (
    <div
      style={{
        margin: '4px 0 10px 0',
        borderRadius: 'var(--wks-radius-md)',
        border: `1px solid ${colors.borderSubtle}`,
        backgroundColor: 'rgba(255,255,255,0.015)',
        overflow: 'hidden',
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      <div
        onClick={() => setExpanded((e) => !e)}
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
        {anyRunning ? (
          <AgentSpinner />
        ) : (
          <span
            style={{
              color: summary.failed > 0 ? colors.error : colors.success,
              fontSize: '0.7rem',
              width: 12,
              textAlign: 'center',
              flexShrink: 0,
            }}
          >
            {summary.failed > 0 ? '✗' : '✓'}
          </span>
        )}
        <span style={{ color: colors.text, fontWeight: 600, flexShrink: 0 }}>
          {toolCalls.length} step{toolCalls.length !== 1 ? 's' : ''}
        </span>
        {summary.editedFiles.length > 0 && (
          <span
            onClick={(e) => {
              e.stopPropagation();
              setFilesOpen((f) => !f);
            }}
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
            .filter((p) => !/^\d+ files? changed$/.test(p))
            .join(' · ');
          return rest ? (
            <span
              style={{
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                minWidth: 0,
              }}
            >
              {rest}
            </span>
          ) : null;
        })()}
        <div style={{ flex: 1 }} />
        {(summary.added > 0 || summary.removed > 0) && (
          <span
            style={{
              fontFamily: 'var(--claude-mono-font, monospace)',
              fontSize: '0.66rem',
              flexShrink: 0,
            }}
          >
            {summary.added > 0 && <span style={{ color: colors.success }}>+{summary.added}</span>}
            {summary.added > 0 && summary.removed > 0 && ' '}
            {summary.removed > 0 && <span style={{ color: colors.error }}>−{summary.removed}</span>}
          </span>
        )}
        <span style={{ color: colors.mutedDim, fontSize: '0.64rem', flexShrink: 0 }}>
          {expanded ? '▾' : '▸'}
        </span>
      </div>

      {filesOpen && !expanded && summary.editedFiles.length > 0 && (
        <div
          style={{
            borderTop: `1px solid ${colors.borderSubtle}`,
            padding: '4px 10px 6px 10px',
            display: 'flex',
            flexDirection: 'column',
            gap: 1,
          }}
        >
          {summary.editedFiles.map((filePath) => {
            const basename = filePath.replace(/\\/g, '/').split('/').pop() ?? filePath;
            const dir = filePath.replace(/\\/g, '/').split('/').slice(0, -1).join('/');
            return (
              <FileLink
                key={filePath}
                path={filePath}
                cwd={cwd}
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 6,
                  padding: '2px 4px',
                  borderRadius: 'var(--wks-radius-sm)',
                  width: '100%',
                  color: colors.accent,
                  fontSize: '0.7rem',
                }}
              >
                <span style={{ flexShrink: 0 }}>{basename}</span>
                {dir && (
                  <span
                    style={{
                      color: colors.mutedDim,
                      fontSize: '0.64rem',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      minWidth: 0,
                    }}
                  >
                    {dir}
                  </span>
                )}
              </FileLink>
            );
          })}
        </div>
      )}

      {visibleWhenCollapsed.length > 0 && (
        <div style={{ padding: '0 10px 6px 10px' }}>{visibleWhenCollapsed}</div>
      )}

      {expanded && (
        <div
          style={{ padding: '2px 10px 8px 10px', borderTop: `1px solid ${colors.borderSubtle}` }}
        >
          {toolCalls.map((tc) => {
            const wf = workflowByToolId?.get(tc.id);
            if (wf) return <WorkflowRunCard key={tc.id} run={wf} />;
            const sub = subagentByToolId?.get(tc.id);
            if (sub) return <SubagentRow key={tc.id} sub={sub} />;
            return (
              <React.Fragment key={tc.id}>
                <WorkLogEntry tc={tc} />
                {hasDiff(tc) &&
                  (typeof tc.input?.diff === 'string' ? (
                    <PatchDiffView
                      patch={tc.input.diff}
                      filePath={tc.input?.file_path ?? tc.input?.path}
                      cwd={cwd}
                    />
                  ) : (
                    <DiffView
                      oldStr={tc.input?.old_string ?? ''}
                      newStr={tc.input?.new_string ?? ''}
                      filePath={tc.input?.file_path}
                      cwd={cwd}
                    />
                  ))}
                {hasRead(tc) && (
                  <ReadView
                    response={String(tc.response)}
                    filePath={tc.input?.file_path}
                    cwd={cwd}
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
