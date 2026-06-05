import React from 'react';
import type { WorkflowAgentInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration, shortModel } from './agentUtils';

// ── Shared styles ──

export const agentMetaStyle: React.CSSProperties = {
  color: colors.mutedDim,
  fontSize: '0.65rem',
  fontVariantNumeric: 'tabular-nums',
  whiteSpace: 'nowrap',
  flexShrink: 0,
};

export const lastToolLineStyle: React.CSSProperties = {
  paddingLeft: 18,
  fontSize: '0.68rem',
  color: colors.muted,
  lineHeight: 1.3,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
  fontFamily: 'var(--claude-mono-font, monospace)',
};

// ── AgentSpinner ──

export const AgentSpinner: React.FC<{ color?: string }> = ({ color = AGENT_PURPLE }) => (
  <span style={{
    display: 'inline-block', width: 12, height: 12,
    border: `1.5px solid ${color}`, borderTopColor: 'transparent',
    borderRadius: '50%', animation: 'claudeSpinner 0.8s linear infinite', flexShrink: 0,
  }} />
);

// ── agentStatusIcon ──

export const agentStatusIcon = (status: WorkflowAgentInfo['status']): React.ReactNode => {
  switch (status) {
    case 'queued':
      return <span style={{ color: colors.mutedDim, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'◌'}</span>;
    case 'running':
      return <AgentSpinner />;
    case 'failed':
      return <span style={{ color: colors.error, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'✗'}</span>;
    default:
      return <span style={{ color: colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'✓'}</span>;
  }
};

// ── WorkflowAgentRow ──

export const WorkflowAgentRow: React.FC<{ agent: WorkflowAgentInfo; now: number }> = ({ agent, now }) => {
  const running = agent.status === 'running';
  const title = agent.label ?? agent.promptPreview ?? agent.id;
  const duration = agent.durationMs ?? (running && agent.startedAt ? now - agent.startedAt : undefined);

  return (
    <div style={{ padding: '1px 0' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.72rem', lineHeight: 1.4 }}>
        {agentStatusIcon(agent.status)}
        <span
          title={agent.promptPreview ?? title}
          style={{
            color: running ? colors.textBright : colors.text,
            flex: 1,
            minWidth: 0,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {title}
        </span>
        {agent.model && <span style={agentMetaStyle}>{shortModel(agent.model)}</span>}
        {agent.tokens > 0 && <span style={agentMetaStyle}>{fmtTokens(agent.tokens)} tok</span>}
        {duration !== undefined && <span style={agentMetaStyle}>{fmtDuration(duration)}</span>}
      </div>
      {running && agent.lastToolName && (
        <div style={lastToolLineStyle}>
          {'└'} {agent.lastToolName}{agent.lastToolSummary ? ` ${agent.lastToolSummary}` : ''}
        </div>
      )}
    </div>
  );
};
