import React from 'react';
import type { SubagentInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens } from './agentUtils';
import { AgentSpinner, agentMetaStyle, lastToolLineStyle } from './WorkflowAgentRow';

export const SubagentRow: React.FC<{ sub: SubagentInfo }> = ({ sub }) => (
  <div style={{ padding: '1px 0' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.75rem', lineHeight: 1.4 }}>
      {sub.status === 'running' ? <AgentSpinner /> : (
        <span style={{ color: colors.success, fontSize: '0.7rem', width: 12, textAlign: 'center', flexShrink: 0 }}>{'✓'}</span>
      )}
      <span style={{ color: AGENT_PURPLE, fontWeight: 600 }}>Agent</span>
      <span style={{ color: colors.text, flexShrink: 0 }}>{sub.type}</span>
      {sub.description ? (
        <span style={{ color: colors.muted, fontSize: '0.7rem', flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={sub.description}>
          {sub.description}
        </span>
      ) : (
        <div style={{ flex: 1 }} />
      )}
      {(sub.toolCalls ?? 0) > 0 && <span style={agentMetaStyle}>{sub.toolCalls} tools</span>}
      {(sub.tokens ?? 0) > 0 && <span style={agentMetaStyle}>{fmtTokens(sub.tokens)} tok</span>}
    </div>
    {sub.status === 'running' && sub.lastToolName && (
      <div style={lastToolLineStyle}>
        {'└'} {sub.lastToolName}{sub.lastToolSummary ? ` ${sub.lastToolSummary}` : ''}
      </div>
    )}
  </div>
);
