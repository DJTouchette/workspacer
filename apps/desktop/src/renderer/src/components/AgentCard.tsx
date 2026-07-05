import React, { useEffect, useMemo, useState } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { QuestionPicker } from './claude/QuestionPicker';
import { AgentCardBody } from './AgentCardBody';
import { useAttention } from '../contexts/AttentionContext';
import { usePageVisible } from '../hooks/usePageVisible';
import { StatusGlyph } from './statusGlyph';
import { AgentLogo } from './agentLogos';
import { shortModelLabel } from '../lib/modelLabel';
import {
  fmtTokens,
  fmtUSD,
  ctxColor,
  isSnapshotStale,
  summarizeFileChanges,
} from '../lib/sessionStats';
import { useGitBranch } from '../hooks/useGitBranch';
function relTime(ts: number | undefined): string {
  if (!ts) return '';
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}
function baseName(p: string | undefined): string {
  if (!p) return '';
  const parts = p.replace(/\/+$/, '').split('/');
  return parts[parts.length - 1] || p;
}

interface StateVisual {
  color: string;
  label: string;
  pulse: boolean;
}
function stateVisual(s: SessionAmbientState | undefined): StateVisual {
  switch (s) {
    case 'waiting_approval':
      return { color: 'var(--wks-warning, #e0a000)', label: 'Needs approval', pulse: true };
    case 'waiting_input':
      return { color: 'var(--wks-warning, #e0a000)', label: 'Waiting for input', pulse: true };
    case 'thinking':
      return { color: 'var(--wks-accent, #4a9eff)', label: 'Thinking', pulse: false };
    case 'streaming':
      return { color: 'var(--wks-accent, #4a9eff)', label: 'Working', pulse: false };
    case 'idle':
      return { color: 'var(--wks-success, #3fb950)', label: 'Idle', pulse: false };
    default:
      return { color: 'var(--wks-text-faint, #666)', label: 'Stopped', pulse: false };
  }
}

function lastAssistant(snap: ClaudeSessionSnapshot | undefined): string {
  const turns = snap?.conversation ?? [];
  for (let i = turns.length - 1; i >= 0; i--) {
    if (turns[i].role === 'assistant' && turns[i].content?.trim()) return turns[i].content.trim();
  }
  return '';
}

interface Props {
  agent: AgentWorkspace;
  snapshot?: ClaudeSessionSnapshot;
  /** Score-derived buoyancy badge ("needs you" etc.), already computed by the deck. */
  rank?: number;
  /** Override for the card-body click. Default pilots into the agent's
   *  workspace; the Agents pane passes an "open a watch pane" action instead. */
  onOpen?: () => void;
}

/**
 * A live Fleet Deck tile — the "telemetry face" of one agent, rendered purely
 * from its snapshot. Border tints by ambient state and pulses when the agent is
 * blocked on you. Clicking the card body pilots into the agent; the action zone
 * lets you resolve the common cases (approve / answer a question / drop a quick
 * message) without ever leaving the deck.
 */
export const AgentCard: React.FC<Props> = ({ agent, snapshot, onOpen }) => {
  const { openAgent, approve, answer, sendMessage, feed } = useAttention();
  const pageVisible = usePageVisible();
  const state = snapshot?.ambientState;
  const v = stateVisual(agent.sessionId ? state : undefined);
  const usage = snapshot?.usage;
  const ctxFrac =
    usage && usage.contextLimit > 0 ? Math.min(1, usage.contextTokens / usage.contextLimit) : 0;

  const activeTool = snapshot?.activeToolCalls?.[snapshot.activeToolCalls.length - 1];
  const runningSubs = (snapshot?.subagents ?? []).filter((s) => s.status === 'running').length;
  const runningWf = (snapshot?.workflows ?? []).filter((w) => w.status === 'running');
  const approvalItem = feed.find((it) => it.agentId === agent.id && it.kind === 'approval');
  const questionItem = feed.find((it) => it.agentId === agent.id && it.kind === 'question');
  const turns = (snapshot?.conversation ?? []).length;

  const working = state === 'thinking' || state === 'streaming';
  const branch = useGitBranch(agent.cwd);

  // Staleness needs a clock even when no snapshots arrive (that IS the stale
  // case) — a slow tick re-evaluates it without re-rendering per second.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!working) return;
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [working]);
  const stale = isSnapshotStale(state, snapshot?.lastActivity, now);

  // Body: the last message always leads (as markdown); tool activity lives in
  // the chip row, so the two no longer alternate.
  const bodyText = lastAssistant(snapshot);
  const bodyFallback = agent.sessionId ? 'No activity yet' : 'Stopped — click to respawn';
  const recentTools = useMemo(
    () => (snapshot?.completedToolCalls ?? []).slice(-2).reverse(),
    [snapshot?.completedToolCalls],
  );
  const fileStats = useMemo(
    () => summarizeFileChanges(snapshot?.fileChanges ?? []),
    [snapshot?.fileChanges],
  );

  const [draft, setDraft] = useState('');
  const submitDraft = () => {
    if (!agent.sessionId || !draft.trim()) return;
    sendMessage(agent.sessionId, draft.trim());
    setDraft('');
  };

  const hasAction = !!(approvalItem || questionItem);
  // The compose box duplicates the question picker's own text field, so hide it
  // while a question is up to avoid two rival inputs on the same card.
  const showCompose = !!agent.sessionId && !questionItem;

  return (
    <div
      onClick={onOpen ?? (() => openAgent(agent.id))}
      title={`${agent.name} — ${v.label}\n${agent.cwd}`}
      style={{
        display: 'flex',
        flexDirection: 'column',
        minHeight: 260,
        cursor: 'pointer',
        borderRadius: 'var(--wks-radius-lg)',
        overflow: 'hidden',
        background: 'var(--wks-bg-surface)',
        border: `1.5px solid ${v.color}`,
        boxShadow: v.pulse ? `0 0 0 1px ${v.color}` : '0 4px 16px var(--wks-shadow)',
        animation: v.pulse && pageVisible ? 'fleetPulse 1.8s ease-in-out infinite' : undefined,
        transition: 'transform 0.1s ease, box-shadow 0.12s ease',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.transform = 'translateY(-2px)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.transform = '';
      }}
    >
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '12px 14px 8px' }}>
        <span
          style={{
            width: 10,
            height: 10,
            borderRadius: '50%',
            background: v.color,
            flexShrink: 0,
            boxShadow: state && state !== 'idle' ? `0 0 8px ${v.color}` : 'none',
          }}
        />
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            fontSize: '0.95rem',
            fontWeight: 700,
            color: 'var(--wks-text-primary)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {agent.kind === 'supervisor' ? (
            <span>🧭</span>
          ) : (
            <AgentLogo
              provider={agent.provider ?? 'claude'}
              size={14}
              style={{ color: 'var(--wks-text-tertiary)', flexShrink: 0 }}
            />
          )}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {agent.name}
          </span>
        </span>
        <span
          style={{
            marginLeft: 'auto',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            fontSize: '0.7rem',
            fontWeight: 600,
            color: v.color,
            flexShrink: 0,
          }}
        >
          <StatusGlyph
            state={agent.sessionId ? state : undefined}
            size={13}
            strokeWidth={2.2}
            accent="currentColor"
          />
          {v.label}
        </span>
      </div>

      {/* Meta line: model · turns · last activity · folder */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          flexWrap: 'wrap',
          padding: '0 14px 8px',
          fontSize: '0.66rem',
          color: 'var(--wks-text-faint)',
        }}
      >
        {usage?.model && (
          <span style={{ color: 'var(--wks-text-secondary)' }}>{shortModelLabel(usage.model)}</span>
        )}
        {turns > 0 && (
          <span>
            · {turns} turn{turns > 1 ? 's' : ''}
          </span>
        )}
        {snapshot?.lastActivity ? <span>· {relTime(snapshot.lastActivity)}</span> : null}
        {stale && (
          <span
            title={`Says "${v.label}" but nothing has arrived since ${relTime(snapshot?.lastActivity)} — the stream may have stalled.`}
            style={{ color: 'var(--wks-warning, #e0a000)', fontWeight: 700 }}
          >
            ⚠ stale
          </span>
        )}
        <span
          style={{
            marginLeft: 'auto',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            minWidth: 0,
            maxWidth: '60%',
          }}
        >
          {branch && (
            <span
              style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
              title={`branch ${branch}`}
            >
              ⎇ {branch}
            </span>
          )}
          {agent.cwd && (
            <span
              style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
              title={agent.cwd}
            >
              {baseName(agent.cwd)}
            </span>
          )}
        </span>
      </div>

      {/* Body: tool chips + last message as markdown + changed-files line */}
      <div style={{ flex: 1, paddingBottom: 10, minHeight: 0, display: 'flex' }}>
        <AgentCardBody
          text={bodyText}
          fallback={bodyFallback}
          active={working ? activeTool : undefined}
          recent={recentTools}
          fileStats={fileStats}
          compact={hasAction}
        />
      </div>

      {/* Orchestration mini-progress */}
      {(runningSubs > 0 || runningWf.length > 0) && (
        <div
          style={{
            padding: '0 14px 8px',
            display: 'flex',
            gap: 10,
            fontSize: '0.68rem',
            color: 'var(--wks-accent)',
            fontWeight: 600,
          }}
        >
          {runningWf.length > 0 && <span>⚙ {runningWf[0].name || 'workflow'}</span>}
          {runningSubs > 0 && (
            <span>
              ◇ {runningSubs} subagent{runningSubs > 1 ? 's' : ''}
            </span>
          )}
        </div>
      )}

      {/* Metrics: context bar + cost */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 14px 10px' }}>
        {usage && usage.contextTokens > 0 ? (
          <>
            <span
              style={{
                flex: 1,
                height: 5,
                borderRadius: 3,
                background: 'var(--wks-border-subtle, #2a2a2a)',
                overflow: 'hidden',
              }}
            >
              <span
                style={{
                  display: 'block',
                  height: '100%',
                  width: `${Math.max(2, ctxFrac * 100)}%`,
                  background: ctxColor(ctxFrac * 100),
                }}
              />
            </span>
            <span
              style={{
                fontSize: '0.64rem',
                color: ctxColor(ctxFrac * 100),
                fontVariantNumeric: 'tabular-nums',
                flexShrink: 0,
              }}
            >
              {fmtTokens(usage.contextTokens)} · {Math.round(ctxFrac * 100)}%
            </span>
            <span style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>
              {fmtUSD(usage.costUSD)}
            </span>
          </>
        ) : (
          <span style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)' }}>
            {agent.sessionId ? 'No usage yet' : ''}
          </span>
        )}
      </div>

      {/* Action zone: approve / answer a question / compose a message */}
      {(hasAction || showCompose) && (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            padding: '10px 14px 12px',
            borderTop: '1px solid var(--wks-glass-border)',
            background: 'var(--wks-glass-strong)',
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
          }}
        >
          {/* Approval — yes / allow-all / no */}
          {approvalItem && (
            <div>
              <div
                style={{
                  fontSize: '0.66rem',
                  fontWeight: 600,
                  color: 'var(--wks-warning, #e0a000)',
                  marginBottom: 6,
                }}
              >
                Permission: {approvalItem.title}
                {approvalItem.detail ? ` — ${approvalItem.detail}` : ''}
              </div>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                <button
                  onClick={() => approve(approvalItem, 'yes')}
                  style={qa('var(--wks-success, #3fb950)')}
                >
                  Allow
                </button>
                <button
                  onClick={() => approve(approvalItem, 'always')}
                  style={qa('var(--wks-accent, #4a9eff)')}
                >
                  Allow all
                </button>
                <button
                  onClick={() => approve(approvalItem, 'no')}
                  style={qa('var(--wks-error, #f87171)')}
                >
                  Deny
                </button>
                <button
                  onClick={() => openAgent(agent.id)}
                  style={{ ...qa('var(--wks-text-secondary)'), marginLeft: 'auto' }}
                >
                  Open
                </button>
              </div>
            </div>
          )}

          {/* Question — option buttons + custom answer (reuses the standard picker) */}
          {questionItem && questionItem.payload.type === 'question' && (
            <QuestionPicker
              questions={questionItem.payload.questions}
              onAnswer={(p) => answer(questionItem, p)}
            />
          )}

          {/* Compose — drop a free message to the agent without leaving the deck */}
          {showCompose && (
            <div style={{ display: 'flex', gap: 6, alignItems: 'flex-end' }}>
              <textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    submitDraft();
                  }
                }}
                placeholder={`Message ${agent.name}…`}
                rows={1}
                style={{
                  flex: 1,
                  resize: 'none',
                  minHeight: 30,
                  maxHeight: 90,
                  fontSize: '0.74rem',
                  padding: '6px 9px',
                  borderRadius: 6,
                  lineHeight: 1.4,
                  border: '1px solid var(--wks-glass-border)',
                  background: 'var(--wks-bg-input, rgba(255,255,255,0.03))',
                  color: 'var(--wks-text-primary)',
                  outline: 'none',
                  fontFamily: 'inherit',
                }}
              />
              <button
                onClick={submitDraft}
                disabled={!draft.trim()}
                style={{
                  fontSize: '0.7rem',
                  fontWeight: 700,
                  fontFamily: 'inherit',
                  padding: '6px 12px',
                  borderRadius: 6,
                  cursor: draft.trim() ? 'pointer' : 'default',
                  flexShrink: 0,
                  border: `1px solid ${draft.trim() ? 'var(--wks-accent, #4a9eff)' : 'var(--wks-glass-border)'}`,
                  background: draft.trim() ? 'var(--wks-accent, #4a9eff)' : 'transparent',
                  color: draft.trim() ? '#0d0d10' : 'var(--wks-text-faint)',
                }}
              >
                Send
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

function qa(color: string): React.CSSProperties {
  return {
    fontSize: '0.7rem',
    fontWeight: 700,
    fontFamily: 'inherit',
    padding: '4px 14px',
    borderRadius: 6,
    border: `1px solid ${color}`,
    background: 'transparent',
    color,
    cursor: 'pointer',
  };
}
