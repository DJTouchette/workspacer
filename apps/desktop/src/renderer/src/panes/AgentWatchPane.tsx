import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { claudeColors as colors } from '../components/claude-shared';
import { WorkflowTimeline } from '../components/claude/WorkflowTimeline';
import { AGENT_PURPLE, fmtTokens, fmtDuration, shortModel } from '../components/claude/agentUtils';
import { fmtUSD } from '../lib/sessionStats';
import { AgentSpinner } from '../components/claude/WorkflowAgentRow';
import { useNowTicker } from '../components/claude/useNowTicker';
import { IconDone } from '../components/wksIcons';

interface AgentWatchPaneProps {
  title: string;
  isActive: boolean;
  /** The claudemon session that owns the watched subagent/workflow. */
  watchSessionId?: string;
  watchKind?: 'subagent' | 'workflow';
  /** Subagent id or workflow runId. */
  watchId?: string;
}

/** How often the live transcript re-reads while the subagent runs — matches
 *  the main-process workflowWatcher's tail cadence. */
const TRANSCRIPT_POLL_MS = 2500;

const emptyState = (lines: string[]): React.ReactNode => (
  <div style={{ height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 6, color: colors.mutedDim, fontSize: '0.75rem', textAlign: 'center', padding: 24 }}>
    {lines.map((l, i) => <div key={i} style={i === 0 ? { color: colors.muted, fontWeight: 600 } : undefined}>{l}</div>)}
  </div>
);

/**
 * A live "watch" pane for one unit of delegated work: either a plain Agent-tool
 * subagent (header stats + its transcript, tailed while it runs) or a Workflow
 * run (the embedded timeline). Opened from the inspector rail; purely a viewer —
 * it reads the owning session's snapshot and never touches the session itself.
 */
const AgentWatchPane: React.FC<AgentWatchPaneProps> = ({ isActive, watchSessionId, watchKind, watchId }) => {
  const { session } = useClaudeSession({ ptySessionId: watchSessionId ?? null, active: isActive });

  const sub = watchKind === 'subagent' ? session?.subagents.find((s) => s.id === watchId) : undefined;
  const run = watchKind === 'workflow' ? session?.workflows.find((w) => w.runId === watchId) : undefined;
  const running = sub?.status === 'running';
  const now = useNowTicker(!!running);

  // Subagent transcript, re-read on a slow poll while the agent runs (the
  // watcher tails the same file, so this stays ~2.5s behind reality at worst).
  // null = unavailable/never loaded; [] = loaded but no messages yet.
  const [transcript, setTranscript] = useState<{ role: string; text: string }[] | null>(null);
  const fetchTranscript = useCallback(() => {
    if (watchKind !== 'subagent' || !watchSessionId || !watchId) return;
    window.electronAPI.workflowAgentTranscript(watchSessionId, null, watchId)
      .then((t) => { if (t) setTranscript(t); })
      .catch(() => {});
  }, [watchKind, watchSessionId, watchId]);

  useEffect(() => {
    fetchTranscript();
    if (!running || !isActive) return;
    const t = setInterval(fetchTranscript, TRANSCRIPT_POLL_MS);
    return () => clearInterval(t);
  }, [fetchTranscript, running, isActive]);

  // Follow the tail: stick to the bottom while new turns stream in, unless the
  // user has scrolled up to read something.
  const scrollRef = useRef<HTMLDivElement>(null);
  const turnCount = transcript?.length ?? 0;
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    if (nearBottom) el.scrollTop = el.scrollHeight;
  }, [turnCount]);

  if (!watchSessionId || !watchKind || !watchId) {
    return emptyState(['Nothing to watch', 'This pane lost its watch target — close it and reopen from the inspector.']);
  }

  // ── Workflow run: the embedded timeline is the whole surface ──
  if (watchKind === 'workflow') {
    if (!run) {
      return emptyState([
        'Workflow run not available',
        session
          ? 'Only the most recent runs are kept in the live snapshot — this one may have aged out.'
          : 'The owning session isn’t running (it may have ended or the app restarted).',
      ]);
    }
    return <WorkflowTimeline embedded sessionId={watchSessionId} run={run} />;
  }

  // ── Plain subagent: header stats + live transcript ──
  const duration = sub
    ? (sub.completedAt ?? (running ? now : sub.startedAt)) - sub.startedAt
    : undefined;

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--wks-bg-base)', color: colors.text, overflow: 'hidden' }}>
      {/* Header */}
      <div style={{ padding: '10px 14px', borderBottom: `1px solid ${colors.borderSubtle}`, flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: '0.85rem' }}>
          {running
            ? <AgentSpinner />
            : <IconDone size={14} strokeWidth={2.2} style={{ color: sub ? colors.success : colors.mutedDim, flexShrink: 0 }} accent={colors.success} />}
          <span style={{ color: AGENT_PURPLE, fontWeight: 700 }}>Agent</span>
          <span style={{ color: colors.textBright, fontWeight: 600 }}>{sub?.type ?? 'subagent'}</span>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: '0.66rem', color: running ? AGENT_PURPLE : colors.mutedDim, fontWeight: 600 }}>
            {sub ? (running ? 'running' : 'complete') : 'not in live snapshot'}
          </span>
        </div>
        {sub?.description && (
          <div style={{ fontSize: '0.72rem', color: colors.muted, marginTop: 4, lineHeight: 1.4 }}>{sub.description}</div>
        )}
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: '2px 12px', fontSize: '0.66rem', color: colors.mutedDim, marginTop: 6, fontVariantNumeric: 'tabular-nums' }}>
          {sub?.model && <span>{shortModel(sub.model)}</span>}
          {(sub?.toolCalls ?? 0) > 0 && <span>{sub!.toolCalls} tools</span>}
          {(sub?.tokens ?? 0) > 0 && <span>{fmtTokens(sub!.tokens)} tok</span>}
          {(sub?.costUSD ?? 0) > 0 && <span>{fmtUSD(sub!.costUSD!)}</span>}
          {duration !== undefined && duration >= 0 && <span>{fmtDuration(duration)}</span>}
        </div>
        {running && sub?.lastToolName && (
          <div style={{ fontSize: '0.68rem', color: colors.muted, marginTop: 6, fontFamily: 'var(--claude-mono-font, monospace)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {'└'} {sub.lastToolName}{sub.lastToolSummary ? ` ${sub.lastToolSummary}` : ''}
          </div>
        )}
      </div>

      {/* Transcript */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: 'auto', padding: '12px 16px' }}>
        {transcript === null && (
          <div style={{ color: colors.mutedDim, fontSize: '0.72rem', textAlign: 'center', marginTop: 40 }}>
            {sub ? 'Loading transcript…' : 'Transcript unavailable — the owning session isn’t being watched (it may have ended or the app restarted).'}
          </div>
        )}
        {transcript !== null && transcript.length === 0 && (
          <div style={{ color: colors.mutedDim, fontSize: '0.72rem', textAlign: 'center', marginTop: 40 }}>No messages yet.</div>
        )}
        {transcript?.map((t, i) => (
          <div key={i} style={{ marginBottom: 12, maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}>
            <div style={{ fontSize: '0.58rem', fontWeight: 700, textTransform: 'uppercase', letterSpacing: 0.4, marginBottom: 3, color: t.role === 'user' ? colors.accent : AGENT_PURPLE }}>
              {t.role === 'user' ? (i === 0 ? 'prompt' : 'user') : 'agent'}
            </div>
            <div style={{ fontSize: '0.72rem', lineHeight: 1.5, whiteSpace: 'pre-wrap', wordBreak: 'break-word', color: colors.text, fontFamily: 'var(--claude-mono-font, monospace)' }}>
              {t.text}
            </div>
          </div>
        ))}
        {running && (
          <div style={{ display: 'flex', justifyContent: 'center', padding: '6px 0 12px' }}>
            <AgentSpinner />
          </div>
        )}
      </div>
    </div>
  );
};

export default AgentWatchPane;
