import React, { useState } from 'react';
import type { ProjectSummary, Session } from '../types';
import { colors, formatElapsed, formatCost, projectName } from '../utils';
import TemplateGrid from './TemplateGrid';

interface Props {
  projects: ProjectSummary[];
  sessions: Session[];
  onSelectSession: (sessionId: string) => void;
  onLaunchTask?: (cwd: string) => void;
}

const stateColors: Record<string, string> = {
  idle: '#4ade80',
  thinking: '#fbbf24',
  streaming: '#60a5fa',
  waiting_input: '#c084fc',
  waiting_approval: '#f87171',
};

const stateLabels: Record<string, string> = {
  idle: 'Idle',
  thinking: 'Thinking',
  streaming: 'Streaming',
  waiting_input: 'Waiting',
  waiting_approval: 'Approval',
};

const ProjectView: React.FC<Props> = ({ projects, sessions, onSelectSession, onLaunchTask }) => {
  const [expandedPath, setExpandedPath] = useState<string | null>(null);

  // Sort projects by last activity (most recent first)
  const sorted = [...projects].sort((a, b) => {
    const ta = a.lastActivity ? new Date(a.lastActivity).getTime() : 0;
    const tb = b.lastActivity ? new Date(b.lastActivity).getTime() : 0;
    return tb - ta;
  });

  const getProjectSessions = (path: string): Session[] => {
    return sessions.filter(s => s.cwd === path || s.cwd.startsWith(path + '/'));
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))', gap: 14 }}>
      {sorted.map(proj => {
        const isExpanded = expandedPath === proj.path;
        const hasActive = proj.activeSessions > 0;
        const projSessions = getProjectSessions(proj.path);

        return (
          <div
            key={proj.path}
            style={{
              borderRadius: 10, overflow: 'hidden',
              border: `1px solid ${hasActive ? colors.accent : colors.borderSubtle}`,
              backgroundColor: colors.bgSurface,
              gridColumn: isExpanded ? '1 / -1' : undefined,
              transition: 'border-color 0.2s',
            }}
          >
            {/* Project header */}
            <div
              onClick={() => setExpandedPath(isExpanded ? null : proj.path)}
              style={{
                padding: '10px 14px', cursor: 'pointer', userSelect: 'none',
                borderBottom: isExpanded ? `1px solid ${colors.borderSubtle}` : 'none',
                display: 'flex', alignItems: 'center', gap: 10,
              }}
            >
              <div style={{
                width: 10, height: 10, borderRadius: '50%', flexShrink: 0,
                backgroundColor: hasActive ? colors.accent : colors.textMuted,
                boxShadow: hasActive ? `0 0 8px ${colors.accent}60` : 'none',
              }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: colors.textBright, fontWeight: 600, fontSize: '0.82rem', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {proj.name}
                </div>
                <div style={{ fontSize: '0.55rem', color: colors.textMuted, fontFamily: 'monospace', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {proj.path}
                </div>
              </div>
              <span style={{ fontSize: '0.6rem', color: colors.textMuted, transform: isExpanded ? 'rotate(90deg)' : 'rotate(0deg)', transition: 'transform 0.15s' }}>{'\u25B6'}</span>
            </div>

            {/* Metrics bar */}
            <div style={{ display: 'flex', borderBottom: isExpanded ? `1px solid ${colors.borderSubtle}` : 'none' }}>
              {[
                { label: 'Active', value: `${proj.activeSessions}` },
                { label: 'Total', value: `${proj.totalSessions}` },
                { label: 'Cost', value: formatCost(proj.totalCost) },
                { label: 'Time', value: proj.totalActiveMs > 0 ? formatElapsed(proj.totalActiveMs) : '-' },
                { label: 'Tools', value: `${proj.totalToolCalls}` },
                { label: 'Files', value: `${proj.filesChanged}` },
              ].map((m, i) => (
                <div key={m.label} style={{ flex: 1, padding: '6px 8px', textAlign: 'center', borderRight: i < 5 ? `1px solid ${colors.borderSubtle}` : 'none' }}>
                  <div style={{ fontSize: '0.85rem', fontWeight: 600, color: colors.textBright, fontVariantNumeric: 'tabular-nums' }}>{m.value}</div>
                  <div style={{ fontSize: '0.5rem', color: colors.textMuted }}>{m.label}</div>
                </div>
              ))}
            </div>

            {/* Expanded: sessions + templates + launch */}
            {isExpanded && (
              <div style={{ padding: '10px 14px' }}>
                {/* Sessions for this project */}
                {projSessions.length > 0 && (
                  <div style={{ marginBottom: 12 }}>
                    <div style={{ fontSize: '0.62rem', fontWeight: 600, color: colors.textMuted, marginBottom: 6, textTransform: 'uppercase', letterSpacing: '0.05em' }}>Sessions</div>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                      {projSessions.map(s => {
                        const badgeColor = stateColors[s.ambientState] || colors.textMuted;
                        return (
                          <div
                            key={s.sessionId}
                            onClick={() => onSelectSession(s.sessionId)}
                            style={{
                              display: 'flex', alignItems: 'center', gap: 8,
                              padding: '6px 10px', borderRadius: 6, cursor: 'pointer',
                              border: `1px solid ${colors.borderSubtle}`,
                              backgroundColor: colors.bg,
                            }}
                            onMouseEnter={e => { e.currentTarget.style.borderColor = colors.accent; }}
                            onMouseLeave={e => { e.currentTarget.style.borderColor = colors.borderSubtle; }}
                          >
                            <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: badgeColor, flexShrink: 0 }} />
                            <span style={{ fontSize: '0.65rem', color: colors.textBright, fontWeight: 500, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                              {projectName(s.cwd)}
                            </span>
                            <span style={{
                              fontSize: '0.52rem', fontWeight: 600, padding: '1px 6px', borderRadius: 4,
                              backgroundColor: `${badgeColor}15`, color: badgeColor,
                            }}>
                              {s.status === 'ended' ? 'Ended' : (stateLabels[s.ambientState] || s.ambientState)}
                            </span>
                            <span style={{ fontSize: '0.52rem', color: colors.textMuted }}>
                              {s.totalToolCalls} tools
                            </span>
                            <span style={{ fontSize: '0.55rem', color: colors.textMuted }}>{'\u2192'}</span>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}

                {/* Templates for this project */}
                <TemplateGrid
                  projectPath={proj.path}
                  onLaunch={(_template, _cwd) => {
                    if (onLaunchTask) onLaunchTask(proj.path);
                  }}
                />

                {/* Launch task button */}
                {onLaunchTask && (
                  <button
                    onClick={() => onLaunchTask(proj.path)}
                    style={{
                      padding: '6px 14px', borderRadius: 6,
                      border: `1px solid ${colors.accent}`,
                      backgroundColor: 'transparent', color: colors.accent,
                      fontSize: '0.65rem', fontWeight: 600, cursor: 'pointer',
                      marginTop: 4,
                    }}
                    onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.accent}15`; }}
                    onMouseLeave={e => { e.currentTarget.style.backgroundColor = 'transparent'; }}
                  >
                    + Launch task in {proj.name}
                  </button>
                )}
              </div>
            )}
          </div>
        );
      })}

      {/* Empty state */}
      {sorted.length === 0 && (
        <div style={{ gridColumn: '1 / -1', textAlign: 'center', padding: '60px 0' }}>
          <div style={{ fontSize: '0.9rem', color: colors.textBright, fontWeight: 600, marginBottom: 6 }}>
            No projects found
          </div>
          <div style={{ fontSize: '0.72rem', color: colors.textMuted }}>
            Launch an agent or queue a task to see project groupings.
          </div>
        </div>
      )}
    </div>
  );
};

export default ProjectView;
