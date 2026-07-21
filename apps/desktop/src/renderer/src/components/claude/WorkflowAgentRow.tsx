import React from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import type { WorkflowAgentInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration, shortModel } from './agentUtils';
import { fmtUSD } from '../../lib/sessionStats';

// Expanded detail block (full prompt / result), shared by workflow + subagent rows.
export const agentDetailStyle: React.CSSProperties = {
  paddingLeft: 18,
  paddingTop: 2,
  fontSize: '0.69rem',
  lineHeight: 1.4,
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-word',
  fontFamily: 'var(--claude-mono-font, monospace)',
};
export const agentDetailLabel: React.CSSProperties = {
  color: colors.mutedDim,
  fontSize: '0.62rem',
  fontWeight: 700,
  textTransform: 'uppercase',
  letterSpacing: 0.4,
};
import { IconQueued, IconError, IconDone } from '../wksIcons';

// ── Shared styles ──

export const agentMetaStyle: React.CSSProperties = {
  color: colors.mutedDim,
  fontSize: '0.68rem',
  fontVariantNumeric: 'tabular-nums',
  whiteSpace: 'nowrap',
  flexShrink: 0,
};

export const lastToolLineStyle: React.CSSProperties = {
  paddingLeft: 18,
  fontSize: '0.7rem',
  color: colors.muted,
  lineHeight: 1.3,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
  fontFamily: 'var(--claude-mono-font, monospace)',
};

// ── AgentSpinner ──

export const AgentSpinner: React.FC<{ color?: string }> = ({ color = AGENT_PURPLE }) => (
  <span
    style={{
      display: 'inline-block',
      width: 12,
      height: 12,
      border: `1.5px solid ${color}`,
      borderTopColor: 'transparent',
      borderRadius: '50%',
      animation: 'claudeSpinner 0.8s linear infinite',
      flexShrink: 0,
    }}
  />
);

// ── agentStatusIcon ──

export const agentStatusIcon = (status: WorkflowAgentInfo['status']): React.ReactNode => {
  switch (status) {
    case 'queued':
      // Pack "queued" clock — quiet/muted, no accent.
      return (
        <IconQueued
          size={12}
          strokeWidth={2.2}
          style={{ color: colors.mutedDim, flexShrink: 0 }}
          accent={colors.mutedDim}
        />
      );
    case 'running':
      return <AgentSpinner />;
    case 'failed':
      // Pack "error" triangle, tinted red.
      return (
        <IconError size={12} strokeWidth={2.2} style={{ color: colors.error, flexShrink: 0 }} />
      );
    default:
      // Pack "done" check-circle, tinted green.
      return (
        <IconDone
          size={12}
          strokeWidth={2.2}
          style={{ color: colors.success, flexShrink: 0 }}
          accent={colors.success}
        />
      );
  }
};

// ── WorkflowAgentRow ──

const WorkflowAgentRowInner: React.FC<{ agent: WorkflowAgentInfo; now: number }> = ({
  agent,
  now,
}) => {
  const running = agent.status === 'running';
  const failed = agent.status === 'failed';
  const title = agent.label ?? agent.promptPreview ?? agent.id;
  const duration =
    agent.durationMs ?? (running && agent.startedAt ? now - agent.startedAt : undefined);
  // The prompt/result are captured by the watcher; let the user expand to read
  // them (and, for a failed agent, see why it failed).
  const canExpand = !!(agent.promptPreview || agent.resultPreview);
  const [expanded, setExpanded] = React.useState(false);

  return (
    <div style={{ padding: '1px 0' }}>
      <div
        onClick={canExpand ? () => setExpanded((e) => !e) : undefined}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: '0.72rem',
          lineHeight: 1.4,
          cursor: canExpand ? 'pointer' : 'default',
        }}
      >
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
        {(agent.costUSD ?? 0) > 0 && <span style={agentMetaStyle}>{fmtUSD(agent.costUSD!)}</span>}
        {agent.toolCalls > 0 && <span style={agentMetaStyle}>{agent.toolCalls} tools</span>}
        {duration !== undefined && <span style={agentMetaStyle}>{fmtDuration(duration)}</span>}
        {canExpand && (
          <span
            style={{ color: colors.mutedDim, display: 'flex', alignItems: 'center', flexShrink: 0 }}
          >
            {expanded ? (
              <ChevronDown size={12} strokeWidth={2} />
            ) : (
              <ChevronRight size={12} strokeWidth={2} />
            )}
          </span>
        )}
      </div>
      {running && agent.lastToolName && (
        <div style={lastToolLineStyle}>
          {'└'} {agent.lastToolName}
          {agent.lastToolSummary ? ` ${agent.lastToolSummary}` : ''}
        </div>
      )}
      {expanded && (
        <div style={{ paddingBottom: 3 }}>
          {agent.promptPreview && (
            <div style={agentDetailStyle}>
              <span style={agentDetailLabel}>Prompt </span>
              <span style={{ color: colors.muted }}>{agent.promptPreview}</span>
            </div>
          )}
          {agent.resultPreview && (
            <div style={agentDetailStyle}>
              <span style={agentDetailLabel}>{failed ? 'Error ' : 'Result '}</span>
              <span style={{ color: failed ? colors.error : colors.text }}>
                {agent.resultPreview}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

// Memoized: finished rows (durationMs set, status !== 'running') don't change
// and won't re-render when the parent's 1Hz `now` tick fires — only running
// rows receive a meaningful `now` update and actually need to re-render.
export const WorkflowAgentRow = React.memo(WorkflowAgentRowInner);
