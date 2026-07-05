import React, { useEffect, useMemo, useState } from 'react';
import type { WorkflowRunInfo, WorkflowAgentInfo } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AGENT_PURPLE, fmtTokens, fmtDuration, shortModel } from './agentUtils';
import { fmtUSD } from '../../lib/sessionStats';
import { agentStatusIcon } from './WorkflowAgentRow';
import { useNowTicker } from './useNowTicker';

/** Bar color by agent status. */
function statusColor(status: WorkflowAgentInfo['status']): string {
  switch (status) {
    case 'running': return AGENT_PURPLE;
    case 'failed': return colors.error;
    case 'queued': return colors.mutedDim;
    default: return colors.success;
  }
}

/**
 * Full-height timeline view of one workflow run: phases as swimlanes, each agent
 * a time-positioned bar so you can see what ran in parallel vs. sequentially, and
 * a detail panel (click a bar) with the agent's prompt, result, and stats. Opened
 * as a modal from a WorkflowRunCard's expand button, or `embedded` (no backdrop /
 * Escape / close button) inside an agent-watch pane; re-reads the live run each
 * render so it keeps updating while the workflow runs.
 *
 * `transcriptRunId` overrides the runId used for transcript drill-ins: the
 * session-fleet view feeds a SYNTHETIC run built from plain Agent-tool
 * subagents, whose transcripts live outside any workflow (runId = null).
 */
export const WorkflowTimeline: React.FC<{
  sessionId: string;
  run: WorkflowRunInfo;
  onClose?: () => void;
  embedded?: boolean;
  /** Kind label in the header (default "Workflow"). */
  heading?: string;
  transcriptRunId?: string | null;
}> = ({ sessionId, run, onClose, embedded, heading, transcriptRunId }) => {
  const running = run.status === 'running';
  const now = useNowTicker(running);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Full transcript of the selected agent, fetched on demand (main reads the
  // agent-<id>.jsonl). null = none/failed load; [] = loaded-but-empty.
  const [transcript, setTranscript] = useState<{ role: string; text: string }[] | null>(null);
  const [loadingTx, setLoadingTx] = useState(false);

  useEffect(() => {
    if (embedded || !onClose) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose, embedded]);

  const txRunId = transcriptRunId !== undefined ? transcriptRunId : run.runId;
  useEffect(() => {
    if (!selectedId) { setTranscript(null); return; }
    let cancelled = false;
    setLoadingTx(true);
    setTranscript(null);
    window.electronAPI.workflowAgentTranscript(sessionId, txRunId, selectedId)
      .then((t) => { if (!cancelled) setTranscript(t); })
      .catch(() => { if (!cancelled) setTranscript(null); })
      .finally(() => { if (!cancelled) setLoadingTx(false); });
    return () => { cancelled = true; };
  }, [selectedId, sessionId, txRunId]);

  const finished = run.agents.filter(a => a.status === 'done' || a.status === 'failed').length;
  const failed = run.agents.filter(a => a.status === 'failed').length;
  const tokens = run.totalTokens ?? run.agents.reduce((s, a) => s + (a.tokens ?? 0), 0);
  const cost = run.totalCostUSD ?? run.agents.reduce((s, a) => s + (a.costUSD ?? 0), 0);
  const elapsed = running && run.startedAt ? now - run.startedAt : run.durationMs;

  // Shared time axis across all agents so overlapping bars read as "ran together".
  const start = run.startedAt;
  const end = run.completedAt ?? now;
  const span = Math.max(1, end - start);

  // Phase title → detail, and agents grouped into phase swimlanes (declared-phase
  // order first, then any leftover/ungrouped agents).
  const { lanes, phaseDetail } = useMemo(() => {
    const detail = new Map<string, string>();
    for (const p of run.phases) if (p.detail) detail.set(p.title, p.detail);
    const byTitle = new Map<string, WorkflowAgentInfo[]>();
    const order: string[] = [];
    const ungrouped: WorkflowAgentInfo[] = [];
    for (const a of run.agents) {
      if (!a.phaseTitle) { ungrouped.push(a); continue; }
      if (!byTitle.has(a.phaseTitle)) { byTitle.set(a.phaseTitle, []); order.push(a.phaseTitle); }
      byTitle.get(a.phaseTitle)!.push(a);
    }
    const out: { title: string | null; agents: WorkflowAgentInfo[] }[] =
      order.map(t => ({ title: t, agents: byTitle.get(t)! }));
    if (ungrouped.length) out.push({ title: null, agents: ungrouped });
    return { lanes: out, phaseDetail: detail };
  }, [run.agents, run.phases]);

  const selected = selectedId ? run.agents.find(a => a.id === selectedId) : undefined;

  // Modal chrome only when floating; embedded fills its pane edge-to-edge.
  const cardStyle: React.CSSProperties = embedded
    ? { width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--wks-bg-base)', overflow: 'hidden' }
    : {
        width: 'min(1100px, 92vw)', height: 'min(760px, 88vh)', display: 'flex', flexDirection: 'column',
        background: 'var(--wks-bg-raised)', border: `1px solid ${colors.border}`,
        borderRadius: 12, overflow: 'hidden', boxShadow: '0 24px 64px rgba(0,0,0,0.5)',
      };

  const card = (
      <div
        onClick={embedded ? undefined : (e) => e.stopPropagation()}
        style={cardStyle}
      >
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 16px', borderBottom: `1px solid ${colors.borderSubtle}` }}>
          <span style={{ color: AGENT_PURPLE, fontWeight: 700, fontSize: '0.9rem' }}>{heading ?? 'Workflow'}</span>
          <span style={{ color: colors.textBright, fontWeight: 600, fontSize: '0.9rem', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }} title={run.description ?? run.name}>
            {run.name ?? run.runId}
          </span>
          <div style={{ flex: 1 }} />
          <span style={metaStyle}>{finished}/{run.agents.length} agents</span>
          {failed > 0 && <span style={{ ...metaStyle, color: colors.error, fontWeight: 700 }}>{failed} failed</span>}
          {tokens > 0 && <span style={metaStyle}>{fmtTokens(tokens)} tok</span>}
          {cost > 0 && <span style={metaStyle}>{fmtUSD(cost)}</span>}
          {elapsed !== undefined && <span style={metaStyle}>{fmtDuration(elapsed)}</span>}
          {!embedded && onClose && <button onClick={onClose} title="Close (Esc)" style={closeBtn}>✕</button>}
        </div>

        {/* Body: timeline + detail */}
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          <div style={{ flex: 1, overflowY: 'auto', padding: '10px 14px' }}>
            {run.agents.length === 0 && (
              <div style={{ color: colors.mutedDim, fontSize: '0.8rem', padding: 20, textAlign: 'center' }}>Starting agents…</div>
            )}
            {lanes.map((lane, li) => (
              <div key={`${lane.title ?? 'ungrouped'}-${li}`} style={{ marginBottom: 14 }}>
                <div style={{ color: colors.muted, fontSize: '0.66rem', fontWeight: 700, textTransform: 'uppercase', letterSpacing: 0.6, marginBottom: 2 }}>
                  {lane.title ?? 'Agents'}
                </div>
                {lane.title && phaseDetail.get(lane.title) && (
                  <div style={{ color: colors.mutedDim, fontSize: '0.66rem', marginBottom: 4 }}>{phaseDetail.get(lane.title)}</div>
                )}
                {lane.agents.map(a => {
                  const aStart = a.startedAt ?? end;
                  const aEnd = a.completedAt ?? (a.status === 'running' ? now : aStart);
                  const leftPct = Math.max(0, Math.min(100, ((aStart - start) / span) * 100));
                  const widthPct = Math.max(1.5, Math.min(100 - leftPct, ((aEnd - aStart) / span) * 100));
                  const dur = a.durationMs ?? (a.status === 'running' && a.startedAt ? now - a.startedAt : undefined);
                  const sel = selectedId === a.id;
                  return (
                    <div
                      key={a.id}
                      onClick={() => setSelectedId(sel ? null : a.id)}
                      title={a.promptPreview ?? a.label ?? a.id}
                      style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '2px 0', cursor: 'pointer' }}
                    >
                      <span style={{ width: 14, flexShrink: 0 }}>{agentStatusIcon(a.status)}</span>
                      <span style={{ width: 150, flexShrink: 0, fontSize: '0.72rem', color: sel ? colors.textBright : colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {a.label ?? a.promptPreview ?? a.id}
                      </span>
                      {/* Time track */}
                      <div style={{ flex: 1, position: 'relative', height: 16, background: 'var(--wks-bg-base)', borderRadius: 4, border: sel ? `1px solid ${AGENT_PURPLE}` : `1px solid ${colors.borderSubtle}` }}>
                        <div style={{
                          position: 'absolute', top: 2, bottom: 2, left: `${leftPct}%`, width: `${widthPct}%`,
                          background: statusColor(a.status), borderRadius: 3, minWidth: 3,
                          opacity: a.status === 'queued' ? 0.4 : 0.85,
                        }} />
                      </div>
                      <span style={{ ...metaStyle, width: 96, textAlign: 'right' }}>
                        {a.tokens > 0 ? `${fmtTokens(a.tokens)} · ` : ''}{dur !== undefined ? fmtDuration(dur) : ''}
                      </span>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>

          {/* Detail panel */}
          {selected && (
            <div style={{ width: 340, flexShrink: 0, borderLeft: `1px solid ${colors.borderSubtle}`, overflowY: 'auto', padding: '12px 14px', fontSize: '0.72rem' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
                <span style={{ width: 14 }}>{agentStatusIcon(selected.status)}</span>
                <span style={{ color: colors.textBright, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {selected.label ?? selected.id}
                </span>
              </div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '2px 10px', color: colors.muted, fontSize: '0.66rem', marginBottom: 10 }}>
                {selected.model && <span>{shortModel(selected.model)}</span>}
                {selected.tokens > 0 && <span>{fmtTokens(selected.tokens)} tok</span>}
                {(selected.costUSD ?? 0) > 0 && <span>{fmtUSD(selected.costUSD!)}</span>}
                {selected.toolCalls > 0 && <span>{selected.toolCalls} tools</span>}
                {(selected.durationMs ?? (selected.status === 'running' && selected.startedAt ? now - selected.startedAt : undefined)) !== undefined && (
                  <span>{fmtDuration(selected.durationMs ?? (now - (selected.startedAt ?? now)))}</span>
                )}
              </div>
              {selected.promptPreview && (
                <div style={{ marginBottom: 10 }}>
                  <div style={detailLabel}>Prompt</div>
                  <div style={detailBody}>{selected.promptPreview}</div>
                </div>
              )}
              {selected.resultPreview && (
                <div>
                  <div style={detailLabel}>{selected.status === 'failed' ? 'Error' : 'Result'}</div>
                  <div style={{ ...detailBody, color: selected.status === 'failed' ? colors.error : colors.text }}>{selected.resultPreview}</div>
                </div>
              )}
              {!selected.promptPreview && !selected.resultPreview && (
                <div style={{ color: colors.mutedDim }}>No prompt/result captured{selected.status === 'running' ? ' yet' : ''}.</div>
              )}

              {/* Full transcript drill-in */}
              <div style={{ marginTop: 12, borderTop: `1px solid ${colors.borderSubtle}`, paddingTop: 8 }}>
                <div style={detailLabel}>Transcript</div>
                {loadingTx && <div style={{ color: colors.mutedDim }}>Loading…</div>}
                {!loadingTx && (transcript?.length ?? 0) === 0 && (
                  <div style={{ color: colors.mutedDim }}>{transcript === null ? 'Transcript unavailable.' : 'No messages yet.'}</div>
                )}
                {!loadingTx && transcript && transcript.map((t, i) => (
                  <div key={i} style={{ marginBottom: 8 }}>
                    <div style={{ ...detailLabel, color: t.role === 'user' ? colors.accent : AGENT_PURPLE }}>{t.role}</div>
                    <div style={detailBody}>{t.text}</div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
  );

  if (embedded) return card;
  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(2px)',
      }}
    >
      {card}
    </div>
  );
};

const metaStyle: React.CSSProperties = {
  color: colors.mutedDim, fontSize: '0.68rem', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', flexShrink: 0,
};
const closeBtn: React.CSSProperties = {
  background: 'none', border: 'none', color: colors.muted, cursor: 'pointer', fontSize: '0.9rem', padding: '2px 6px', flexShrink: 0,
};
const detailLabel: React.CSSProperties = {
  color: colors.mutedDim, fontSize: '0.58rem', fontWeight: 700, textTransform: 'uppercase', letterSpacing: 0.4, marginBottom: 2,
};
const detailBody: React.CSSProperties = {
  color: colors.text, fontSize: '0.7rem', lineHeight: 1.45, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
  fontFamily: 'var(--claude-mono-font, monospace)',
};
