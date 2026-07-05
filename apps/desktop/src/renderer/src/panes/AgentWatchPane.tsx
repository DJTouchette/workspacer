import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  ConversationTurn,
  ToolCall,
  WorkflowAgentInfo,
  WorkflowRunInfo,
} from '../types/claudeSession';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { claudeColors as colors } from '../components/claude-shared';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { WorkCard } from '../components/claude/WorkCard';
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
  /** 'agents' = fleet timeline of ALL the session's plain subagents. */
  watchKind?: 'subagent' | 'workflow' | 'agents';
  /** Subagent id, workflow runId, or (for 'agents') the sessionId again. */
  watchId?: string;
}

/** How often the live transcript re-reads while the subagent runs — matches
 *  the main-process workflowWatcher's tail cadence. */
const TRANSCRIPT_POLL_MS = 2500;

const emptyState = (lines: string[]): React.ReactNode => (
  <div
    style={{
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 6,
      color: colors.mutedDim,
      fontSize: '0.75rem',
      textAlign: 'center',
      padding: 24,
    }}
  >
    {lines.map((l, i) => (
      <div key={i} style={i === 0 ? { color: colors.muted, fontWeight: 600 } : undefined}>
        {l}
      </div>
    ))}
  </div>
);

/**
 * A live "watch" pane for delegated work: a plain Agent-tool subagent (header
 * stats + its transcript, tailed while it runs), a Workflow run (the embedded
 * timeline), or the session's whole subagent fleet ('agents' — the same
 * timeline fed a synthetic run built from every plain subagent, so a couple of
 * loose Agent calls get the workflow-style fleet view too). Opened from the
 * inspector rail; purely a viewer — it reads the owning session's snapshot and
 * never touches the session itself.
 */
const AgentWatchPane: React.FC<AgentWatchPaneProps> = ({
  isActive,
  watchSessionId,
  watchKind,
  watchId,
}) => {
  const { session } = useClaudeSession({ ptySessionId: watchSessionId ?? null, active: isActive });

  const sub =
    watchKind === 'subagent' ? session?.subagents.find((s) => s.id === watchId) : undefined;
  const run =
    watchKind === 'workflow' ? session?.workflows.find((w) => w.runId === watchId) : undefined;
  const running = sub?.status === 'running';
  const now = useNowTicker(!!running);

  // GUI view (default) renders the full conversation experience — markdown +
  // WorkCards — from the rich turn parse; Raw is the original mono transcript.
  const [view, setView] = useState<'gui' | 'raw'>('gui');

  // Subagent transcript, re-read on a slow poll while the agent runs (the
  // watcher tails the same file, so this stays ~2.5s behind reality at worst).
  // null = unavailable/never loaded; [] = loaded but no messages yet.
  const [transcript, setTranscript] = useState<{ role: string; text: string }[] | null>(null);
  const [conv, setConv] = useState<ConversationTurn[] | null>(null);
  const fetchTranscript = useCallback(() => {
    if (watchKind !== 'subagent' || !watchSessionId || !watchId) return;
    window.electronAPI
      .workflowAgentTranscript(watchSessionId, null, watchId)
      .then((t) => {
        if (t) setTranscript(t);
      })
      .catch(() => {});
    window.electronAPI
      .workflowAgentConversation(watchSessionId, null, watchId)
      .then((t) => {
        if (t) setConv(t);
      })
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
  const turnCount = (view === 'gui' ? conv?.length : transcript?.length) ?? 0;
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    if (nearBottom) el.scrollTop = el.scrollHeight;
  }, [turnCount]);

  // GUI timeline: text turns as chat messages, consecutive tool-call turns
  // collapsed into one WorkCard — the same reading shape as the main GUI view
  // (text → work → text), minus its windowing/orchestration extras.
  const cwd = session?.cwd;
  const guiItems = useMemo(() => {
    if (!conv) return null;
    const items: React.ReactNode[] = [];
    let work: ToolCall[] = [];
    let workKey = 0;
    const flush = (isLast: boolean) => {
      if (work.length === 0) return;
      items.push(
        <WorkCard
          key={`work-${workKey}`}
          toolCalls={work}
          live={!!running && isLast}
          isLast={isLast}
          cwd={cwd}
        />,
      );
      work = [];
    };
    conv.forEach((turn, i) => {
      const calls = turn.toolCalls ?? [];
      if (calls.length > 0) {
        if (work.length === 0) workKey = i;
        work.push(...calls);
        return;
      }
      flush(false);
      if (turn.content) items.push(<ConversationMessage key={`msg-${i}`} turn={turn} />);
    });
    flush(true);
    return items;
  }, [conv, running, cwd]);

  // Fleet mode: fold the session's plain subagents into a synthetic workflow
  // run so WorkflowTimeline renders them as one swimlane of time bars.
  const subagents = session?.subagents;
  const fleetRun = useMemo<WorkflowRunInfo | null>(() => {
    if (watchKind !== 'agents' || !subagents || subagents.length === 0) return null;
    const agents: WorkflowAgentInfo[] = subagents.map((s) => ({
      id: s.id,
      label: s.type,
      model: s.model,
      status: s.status === 'running' ? 'running' : 'done',
      startedAt: s.startedAt,
      completedAt: s.completedAt,
      durationMs: s.completedAt !== undefined ? s.completedAt - s.startedAt : undefined,
      tokens: s.tokens ?? 0,
      costUSD: s.costUSD,
      toolCalls: s.toolCalls ?? 0,
      lastToolName: s.lastToolName,
      lastToolSummary: s.lastToolSummary,
      promptPreview: s.description,
    }));
    const live = agents.some((a) => a.status === 'running');
    const startedAt = Math.min(...subagents.map((s) => s.startedAt));
    const completedAt = live
      ? undefined
      : Math.max(...subagents.map((s) => s.completedAt ?? s.startedAt));
    return {
      runId: `fleet:${watchSessionId}`,
      name: 'fleet',
      description: 'All plain Agent-tool subagents of this session',
      status: live ? 'running' : 'completed',
      startedAt,
      completedAt,
      durationMs: completedAt !== undefined ? completedAt - startedAt : undefined,
      phases: [],
      agents,
    };
  }, [watchKind, subagents, watchSessionId]);

  if (!watchSessionId || !watchKind || !watchId) {
    return emptyState([
      'Nothing to watch',
      'This pane lost its watch target — close it and reopen from the inspector.',
    ]);
  }

  // ── Session fleet: every plain subagent on the workflow timeline ──
  if (watchKind === 'agents') {
    if (!fleetRun) {
      return emptyState([
        'No agents to watch',
        session
          ? 'This session hasn’t spawned any subagents yet — they’ll appear here live.'
          : 'The owning session isn’t running (it may have ended or the app restarted).',
      ]);
    }
    // Subagent transcripts live outside any workflow run → null runId.
    return (
      <WorkflowTimeline
        embedded
        sessionId={watchSessionId}
        run={fleetRun}
        heading="Agent"
        transcriptRunId={null}
      />
    );
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
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--wks-bg-base)',
        color: colors.text,
        overflow: 'hidden',
      }}
    >
      {/* Header */}
      <div
        style={{
          padding: '10px 14px',
          borderBottom: `1px solid ${colors.borderSubtle}`,
          flexShrink: 0,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: '0.85rem' }}>
          {running ? (
            <AgentSpinner />
          ) : (
            <IconDone
              size={14}
              strokeWidth={2.2}
              style={{ color: sub ? colors.success : colors.mutedDim, flexShrink: 0 }}
              accent={colors.success}
            />
          )}
          <span style={{ color: AGENT_PURPLE, fontWeight: 700 }}>Agent</span>
          <span style={{ color: colors.textBright, fontWeight: 600 }}>
            {sub?.type ?? 'subagent'}
          </span>
          <div style={{ flex: 1 }} />
          {/* GUI / Raw view toggle */}
          <span
            style={{
              display: 'inline-flex',
              border: `1px solid ${colors.borderSubtle}`,
              borderRadius: 6,
              overflow: 'hidden',
              flexShrink: 0,
            }}
          >
            {(['gui', 'raw'] as const).map((v) => (
              <button
                key={v}
                onClick={() => setView(v)}
                title={
                  v === 'gui'
                    ? 'Rendered conversation (markdown + tool cards)'
                    : 'Raw transcript text'
                }
                style={{
                  fontSize: '0.62rem',
                  fontWeight: 600,
                  padding: '2px 8px',
                  border: 'none',
                  cursor: 'pointer',
                  textTransform: 'uppercase',
                  letterSpacing: '0.04em',
                  backgroundColor: view === v ? 'var(--wks-accent-bg)' : 'transparent',
                  color: view === v ? colors.accent : colors.mutedDim,
                }}
              >
                {v === 'gui' ? 'GUI' : 'Raw'}
              </button>
            ))}
          </span>
          <span
            style={{
              fontSize: '0.66rem',
              color: running ? AGENT_PURPLE : colors.mutedDim,
              fontWeight: 600,
            }}
          >
            {sub ? (running ? 'running' : 'complete') : 'not in live snapshot'}
          </span>
        </div>
        {sub?.description && (
          <div style={{ fontSize: '0.72rem', color: colors.muted, marginTop: 4, lineHeight: 1.4 }}>
            {sub.description}
          </div>
        )}
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: '2px 12px',
            fontSize: '0.66rem',
            color: colors.mutedDim,
            marginTop: 6,
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {sub?.model && <span>{shortModel(sub.model)}</span>}
          {(sub?.toolCalls ?? 0) > 0 && <span>{sub!.toolCalls} tools</span>}
          {(sub?.tokens ?? 0) > 0 && <span>{fmtTokens(sub!.tokens)} tok</span>}
          {(sub?.costUSD ?? 0) > 0 && <span>{fmtUSD(sub!.costUSD!)}</span>}
          {duration !== undefined && duration >= 0 && <span>{fmtDuration(duration)}</span>}
        </div>
        {running && sub?.lastToolName && (
          <div
            style={{
              fontSize: '0.68rem',
              color: colors.muted,
              marginTop: 6,
              fontFamily: 'var(--claude-mono-font, monospace)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {'└'} {sub.lastToolName}
            {sub.lastToolSummary ? ` ${sub.lastToolSummary}` : ''}
          </div>
        )}
      </div>

      {/* Transcript */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: 'auto', padding: '12px 16px' }}>
        {(view === 'gui' ? conv === null : transcript === null) && (
          <div
            style={{
              color: colors.mutedDim,
              fontSize: '0.72rem',
              textAlign: 'center',
              marginTop: 40,
            }}
          >
            {sub
              ? 'Loading transcript…'
              : 'Transcript unavailable — the owning session isn’t being watched (it may have ended or the app restarted).'}
          </div>
        )}
        {(view === 'gui'
          ? conv !== null && conv.length === 0
          : transcript !== null && transcript.length === 0) && (
          <div
            style={{
              color: colors.mutedDim,
              fontSize: '0.72rem',
              textAlign: 'center',
              marginTop: 40,
            }}
          >
            No messages yet.
          </div>
        )}
        {view === 'gui' && guiItems && (
          <div style={{ maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}>{guiItems}</div>
        )}
        {view === 'raw' &&
          transcript?.map((t, i) => (
            <div
              key={i}
              style={{ marginBottom: 12, maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}
            >
              <div
                style={{
                  fontSize: '0.58rem',
                  fontWeight: 700,
                  textTransform: 'uppercase',
                  letterSpacing: 0.4,
                  marginBottom: 3,
                  color: t.role === 'user' ? colors.accent : AGENT_PURPLE,
                }}
              >
                {t.role === 'user' ? (i === 0 ? 'prompt' : 'user') : 'agent'}
              </div>
              <div
                style={{
                  fontSize: '0.72rem',
                  lineHeight: 1.5,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                  color: colors.text,
                  fontFamily: 'var(--claude-mono-font, monospace)',
                }}
              >
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
