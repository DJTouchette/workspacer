import React, { useMemo, useRef, useState, useEffect } from 'react';
import type { WorkflowRunInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration } from './agentUtils';
import { AgentSpinner, agentMetaStyle, WorkflowAgentRow } from './WorkflowAgentRow';
import { useNowTicker } from './useNowTicker';

export const WorkflowRunCard: React.FC<{ run: WorkflowRunInfo }> = ({ run }) => {
  const running = run.status === 'running';
  const [expanded, setExpanded] = useState(running);
  // Auto-collapse once when the run finishes; user can re-expand
  const prevStatus = useRef(run.status);
  useEffect(() => {
    if (prevStatus.current === 'running' && run.status !== 'running') setExpanded(false);
    prevStatus.current = run.status;
  }, [run.status]);

  const now = useNowTicker(running);
  const finished = run.agents.filter(a => a.status === 'done' || a.status === 'failed').length;
  const elapsed = running ? now - run.startedAt : run.durationMs;
  const tokens = run.totalTokens ?? run.agents.reduce((sum, a) => sum + a.tokens, 0);

  // Group agents by phase. phaseTitle is only known once the final state file
  // lands, so live agents render as a flat list until then.
  const groups = useMemo(() => {
    const out: { title: string | null; agents: WorkflowRunInfo['agents'] }[] = [];
    for (const a of run.agents) {
      const title = a.phaseTitle ?? null;
      const last = out[out.length - 1];
      if (last && last.title === title) last.agents.push(a);
      else out.push({ title, agents: [a] });
    }
    return out;
  }, [run.agents]);

  return (
    <div style={{
      margin: '4px 0',
      border: `1px solid ${colors.borderSubtle}`,
      borderRadius: 6,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      {/* Header */}
      <div
        onClick={() => setExpanded(e => !e)}
        style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '4px 8px', cursor: 'pointer', userSelect: 'none',
          fontSize: '0.72rem',
        }}
      >
        {running ? <AgentSpinner /> : (
          <span style={{ color: run.status === 'failed' ? colors.error : colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>
            {run.status === 'failed' ? '✗' : '✓'}
          </span>
        )}
        <span style={{ color: AGENT_PURPLE, fontWeight: 600, flexShrink: 0 }}>Workflow</span>
        <span
          title={run.description ?? run.name}
          style={{ color: colors.textBright, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}
        >
          {run.name ?? run.runId}
        </span>
        <div style={{ flex: 1 }} />
        <span style={agentMetaStyle}>{finished}/{run.agents.length} agents</span>
        {tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(tokens)} tok</span>}
        {elapsed !== undefined && <span style={agentMetaStyle}>{fmtDuration(elapsed)}</span>}
        <span style={{ color: colors.mutedDim, fontSize: '0.6rem', flexShrink: 0 }}>{expanded ? '▾' : '▸'}</span>
      </div>

      {/* Agent rows, grouped by phase when known */}
      {expanded && (
        <div style={{ padding: '2px 8px 6px 8px', borderTop: `1px solid ${colors.borderSubtle}` }}>
          {run.agents.length === 0 && (
            <div style={{ color: colors.mutedDim, fontSize: '0.68rem', padding: '2px 0' }}>
              starting agents...
            </div>
          )}
          {groups.map((g, gi) => (
            <div key={`${g.title ?? 'live'}-${gi}`}>
              {g.title && (
                <div style={{ color: colors.muted, fontSize: '0.62rem', fontWeight: 600, textTransform: 'uppercase', letterSpacing: 0.5, padding: '3px 0 1px 0' }}>
                  {g.title}
                </div>
              )}
              {g.agents.map(a => <WorkflowAgentRow key={a.id} agent={a} now={now} />)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
