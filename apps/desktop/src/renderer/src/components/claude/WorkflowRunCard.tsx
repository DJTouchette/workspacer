import React, { useMemo, useRef, useState, useEffect } from 'react';
import type { WorkflowRunInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration } from './agentUtils';
import { fmtUSD } from '../../lib/sessionStats';
import { AgentSpinner, agentMetaStyle, WorkflowAgentRow } from './WorkflowAgentRow';
import { useNowTicker } from './useNowTicker';
import { requestWorkflow } from '../../lib/workflowBus';

/** One phase within a run: a collapsible group with a done/total · failed ·
 *  tokens roll-up, so a 20-agent multi-phase run stays scannable. Completed
 *  phases auto-collapse; running / failed phases stay open. */
const PhaseGroup: React.FC<{
  title: string;
  detail?: string;
  agents: WorkflowRunInfo['agents'];
  now: number;
}> = ({ title, detail, agents, now }) => {
  const done = agents.filter(a => a.status === 'done' || a.status === 'failed').length;
  const failed = agents.filter(a => a.status === 'failed').length;
  const tokens = agents.reduce((s, a) => s + (a.tokens ?? 0), 0);
  const active = agents.some(a => a.status === 'running' || a.status === 'queued') || failed > 0;
  const [expanded, setExpanded] = useState(active);
  // Follow the phase into/out of activity (collapse when it finishes cleanly,
  // reopen if it starts working or something fails) without fighting a manual toggle.
  const prevActive = useRef(active);
  useEffect(() => {
    if (active !== prevActive.current) { setExpanded(active); prevActive.current = active; }
  }, [active]);

  return (
    <div style={{ padding: '2px 0' }}>
      <div
        onClick={() => setExpanded(e => !e)}
        style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer', userSelect: 'none', padding: '1px 0' }}
      >
        <span style={{ color: colors.mutedDim, fontSize: '0.6rem', flexShrink: 0, width: 8 }}>{expanded ? '▾' : '▸'}</span>
        <span style={{ color: colors.muted, fontSize: '0.62rem', fontWeight: 600, textTransform: 'uppercase', letterSpacing: 0.5, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {title}
        </span>
        <div style={{ flex: 1 }} />
        <span style={agentMetaStyle}>{done}/{agents.length}</span>
        {failed > 0 && <span style={{ ...agentMetaStyle, color: colors.error, fontWeight: 600 }}>{failed} failed</span>}
        {tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(tokens)} tok</span>}
      </div>
      {expanded && (
        <div style={{ paddingLeft: 8 }}>
          {detail && <div style={{ color: colors.mutedDim, fontSize: '0.62rem', lineHeight: 1.3, paddingBottom: 1 }}>{detail}</div>}
          {agents.map(a => <WorkflowAgentRow key={a.id} agent={a} now={now} />)}
        </div>
      )}
    </div>
  );
};

export const WorkflowRunCard: React.FC<{ run: WorkflowRunInfo }> = ({ run }) => {
  const running = run.status === 'running';
  const [expanded, setExpanded] = useState(running);
  // Auto-collapse once when a run finishes SUCCESSFULLY; user can re-expand. A
  // failed run stays open so the failure (and which agent failed) is visible.
  const prevStatus = useRef(run.status);
  useEffect(() => {
    if (prevStatus.current === 'running' && run.status === 'completed') setExpanded(false);
    if (prevStatus.current === 'running' && run.status === 'failed') setExpanded(true);
    prevStatus.current = run.status;
  }, [run.status]);

  const now = useNowTicker(running);
  const finished = run.agents.filter(a => a.status === 'done' || a.status === 'failed').length;
  const failed = run.agents.filter(a => a.status === 'failed').length;
  const elapsed = running && run.startedAt ? now - run.startedAt : run.durationMs;
  const tokens = run.totalTokens ?? run.agents.reduce((sum, a) => sum + (a.tokens ?? 0), 0);
  const cost = run.totalCostUSD ?? run.agents.reduce((sum, a) => sum + (a.costUSD ?? 0), 0);

  // Phase title → detail (from the run's declared phases), so a phase group can
  // show what it's for. Parsed by the watcher but previously never surfaced.
  const phaseDetail = useMemo(() => {
    const m = new Map<string, string>();
    for (const p of run.phases) if (p.detail) m.set(p.title, p.detail);
    return m;
  }, [run.phases]);

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
        {failed > 0 && <span style={{ ...agentMetaStyle, color: colors.error, fontWeight: 600 }}>{failed} failed</span>}
        {tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(tokens)} tok</span>}
        {cost > 0 && <span style={agentMetaStyle}>{fmtUSD(cost)}</span>}
        {elapsed !== undefined && <span style={agentMetaStyle}>{fmtDuration(elapsed)}</span>}
        <button
          onClick={(e) => { e.stopPropagation(); requestWorkflow(run.runId); }}
          title="Open timeline"
          style={{ background: 'none', border: 'none', color: colors.mutedDim, cursor: 'pointer', fontSize: '0.72rem', padding: '0 2px', flexShrink: 0, lineHeight: 1 }}
        >⤢</button>
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
            g.title
              ? <PhaseGroup key={`${g.title}-${gi}`} title={g.title} detail={phaseDetail.get(g.title)} agents={g.agents} now={now} />
              // Live/ungrouped agents (phase not known yet) render flat.
              : <div key={`live-${gi}`}>{g.agents.map(a => <WorkflowAgentRow key={a.id} agent={a} now={now} />)}</div>
          ))}
        </div>
      )}
    </div>
  );
};
