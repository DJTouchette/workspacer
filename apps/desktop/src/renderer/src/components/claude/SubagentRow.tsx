import React from 'react';
import type { SubagentInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration, shortModel } from './agentUtils';
import { fmtUSD } from '../../lib/sessionStats';
import { AgentSpinner, agentMetaStyle, lastToolLineStyle } from './WorkflowAgentRow';
import { IconDone } from '../wksIcons';

export const SubagentRow: React.FC<{
  sub: SubagentInfo;
  /** When set the row is clickable — opens a live watch pane for this agent. */
  onOpen?: () => void;
}> = ({ sub, onOpen }) => {
  const running = sub.status === 'running';
  // Only completed subagents have a final duration; running ones don't get a
  // `now` tick here, so we show elapsed on completion (consistent with workflow
  // rows, which do tick).
  const duration = sub.completedAt && sub.startedAt ? sub.completedAt - sub.startedAt : undefined;
  return (
    <div
      onClick={onOpen}
      title={onOpen ? 'Watch this agent in a pane' : undefined}
      style={{ padding: '1px 0', cursor: onOpen ? 'pointer' : undefined, borderRadius: 4 }}
      onMouseEnter={
        onOpen
          ? (e) => {
              e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
            }
          : undefined
      }
      onMouseLeave={
        onOpen
          ? (e) => {
              e.currentTarget.style.backgroundColor = 'transparent';
            }
          : undefined
      }
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: '0.75rem',
          lineHeight: 1.4,
        }}
      >
        {running ? (
          <AgentSpinner />
        ) : (
          <IconDone
            size={12}
            strokeWidth={2.2}
            style={{ color: colors.success, flexShrink: 0 }}
            accent={colors.success}
          />
        )}
        <span style={{ color: AGENT_PURPLE, fontWeight: 600 }}>Agent</span>
        <span style={{ color: colors.text, flexShrink: 0 }}>{sub.type}</span>
        {sub.description ? (
          <span
            style={{
              color: colors.muted,
              fontSize: '0.7rem',
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            title={sub.description}
          >
            {sub.description}
          </span>
        ) : (
          <div style={{ flex: 1 }} />
        )}
        {sub.model && <span style={agentMetaStyle}>{shortModel(sub.model)}</span>}
        {(sub.toolCalls ?? 0) > 0 && <span style={agentMetaStyle}>{sub.toolCalls} tools</span>}
        {(sub.tokens ?? 0) > 0 && <span style={agentMetaStyle}>{fmtTokens(sub.tokens)} tok</span>}
        {(sub.costUSD ?? 0) > 0 && <span style={agentMetaStyle}>{fmtUSD(sub.costUSD!)}</span>}
        {duration !== undefined && <span style={agentMetaStyle}>{fmtDuration(duration)}</span>}
      </div>
      {running && sub.lastToolName && (
        <div style={lastToolLineStyle}>
          {'└'} {sub.lastToolName}
          {sub.lastToolSummary ? ` ${sub.lastToolSummary}` : ''}
        </div>
      )}
    </div>
  );
};
