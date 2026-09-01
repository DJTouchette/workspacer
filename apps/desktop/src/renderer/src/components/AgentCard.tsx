import React, { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, Compass, Diamond, Maximize2, Settings } from 'lucide-react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { QuestionPicker } from './claude/QuestionPicker';
import { AgentCardBody } from './AgentCardBody';
import { Surface } from './Surface';
import { useAttention } from '../contexts/AttentionContext';
import { usePageVisible } from '../hooks/usePageVisible';
import { StatusGlyph } from './statusGlyph';
import { AgentLogo } from './agentLogos';
import { HubChip } from './HubChip';
import { hubOfflineLabel } from '../lib/federation';
import { shortModelLabel } from '../lib/modelLabel';
import {
  deriveSessionStats,
  fmtTokens,
  fmtUSD,
  ctxColor,
  isSnapshotStale,
  planProgress,
  summarizeFileChanges,
  withRecordedUsage,
} from '../lib/sessionStats';
import {
  absentUsageTitle,
  useRecordedUsage,
  useRecordedUsageUnavailable,
} from '../contexts/RecordedUsageContext';
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
      return { color: 'var(--wks-warning)', label: 'Needs approval', pulse: true };
    case 'waiting_input':
      return { color: 'var(--wks-warning)', label: 'Waiting for input', pulse: true };
    case 'thinking':
      return {
        color: 'var(--wks-busy)',
        label: 'Thinking',
        pulse: false,
      };
    case 'streaming':
      return {
        color: 'var(--wks-busy)',
        label: 'In flight',
        pulse: false,
      };
    case 'background':
      return {
        color: 'var(--wks-busy)',
        label: 'Background work',
        pulse: false,
      };
    case 'idle':
      return { color: 'var(--wks-success)', label: 'Standing by', pulse: false };
    default:
      return { color: 'var(--wks-text-faint)', label: 'Stopped', pulse: false };
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
  /** When set, a small expand button appears in the header — the Fleet Deck
   *  wires it to flip the card in place into the live InspectorCard. */
  onInspect?: () => void;
}

/**
 * A live Fleet Deck tile — the "telemetry face" of one agent, rendered purely
 * from its snapshot. A leading rail tints by ambient state and the card pulses
 * when the agent is blocked on you. Clicking the card body pilots into the
 * agent; the action zone lets you resolve the common cases (approve / answer a
 * question / drop a quick message) without ever leaving the deck.
 *
 * DRAWING RULE (see `Surface`): the card is ONE surface — `raised`, so it
 * separates itself with a fill and owns no border. Everything inside it is a
 * band or a control, never a second bordered box: the action zone is a
 * fill-only footer, its buttons are fill-only chips, the compose field is a
 * fill-only input. Depth stops at two. Don't re-add a border to anything here
 * that already has a fill.
 */
export const AgentCard: React.FC<Props> = ({ agent, snapshot, onOpen, onInspect }) => {
  const { openAgent, approve, answer, sendMessage, feed } = useAttention();
  const pageVisible = usePageVisible();
  const state = snapshot?.ambientState;
  // Federation: a peer hub's agent wears a hub chip; when that peer's link is
  // down the tile tombstones — muted "hub offline — last seen …" instead of a
  // live ambient state, but the card stays (last known snapshot).
  const hub = snapshot?.hub ?? agent.hub;
  const hubOffline = !!(hub && snapshot?.hubOffline);
  const v = hubOffline
    ? {
        color: 'var(--wks-text-faint)',
        label: hubOfflineLabel(snapshot?.lastActivity, Date.now()),
        pulse: false,
      }
    : stateVisual(agent.sessionId ? state : undefined);
  // Managed providers (codex/opencode) have no transcript-derived `usage` —
  // their telemetry rides the statusLine. deriveSessionStats merges both (same
  // fallback InspectorCard's Usage tab uses), so the tile's model / context
  // meter / cost light up for every provider.
  // At a cold start there is no snapshot at all (a restored agent's session is
  // a stopped daemon row), so every figure below would be absent even though
  // the history DB recorded them. Merge the recorded ones UNDER the live ones.
  const stats = withRecordedUsage(deriveSessionStats(snapshot), useRecordedUsage(agent.sessionId));
  // Why the figures below are missing, when they are. Null means the recorded
  // source answered and simply had nothing for this session; a string means it
  // was never asked, which is a different sentence to put on screen.
  const usageUnavailable = useRecordedUsageUnavailable();
  const ctxPct = stats.ctxPct;
  // Occupancy comes from deriveSessionStats, which owns the same two-source
  // rule this used to restate (transcript count first, pct × the provider's
  // window for a managed provider that has no transcript) PLUS the guard that
  // restatement was missing: it never multiplies a percentage by a window the
  // session has already been observed to exceed, which is one of the ways an
  // absurd token figure gets manufactured.
  const ctxTokens = stats.contextTokens;

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
  // Remote agents never respawn locally, so their stopped card drops the hint.
  const bodyFallback = agent.sessionId
    ? 'No activity yet'
    : hub
      ? 'Stopped'
      : 'Stopped — click to respawn';
  const recentTools = useMemo(
    () => (snapshot?.completedToolCalls ?? []).slice(-2).reverse(),
    [snapshot?.completedToolCalls],
  );
  const fileStats = useMemo(
    () => summarizeFileChanges(snapshot?.fileChanges ?? []),
    [snapshot?.fileChanges],
  );
  const plan = useMemo(() => planProgress(snapshot?.plan), [snapshot?.plan]);

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
    <Surface
      elevation="raised"
      radius="lg"
      tone={v.color}
      interactive
      onClick={onOpen ?? (() => openAgent(agent.id))}
      title={`${agent.name} — ${v.label}\n${agent.cwd}`}
      style={{
        display: 'flex',
        flexDirection: 'column',
        minHeight: 260,
        // Only the blocked-on-you ring is drawn here; at rest the surface's own
        // hairline shadow is the whole treatment (leaving this undefined lets
        // the class rule through instead of out-specifying it).
        boxShadow: v.pulse ? `0 0 0 1px ${v.color}` : undefined,
        animation: v.pulse && pageVisible ? 'fleetPulse 1.8s ease-in-out infinite' : undefined,
        transition: 'transform 0.12s ease, box-shadow 0.14s ease, background-color 0.12s',
      }}
      onMouseEnter={(e) => {
        const el = e.currentTarget as HTMLElement;
        el.style.transform = 'translateY(-2px)';
        // Don't fight the pulse animation's box-shadow on blocked cards.
        if (!v.pulse) el.style.boxShadow = '0 6px 20px var(--wks-shadow)';
      }}
      onMouseLeave={(e) => {
        const el = e.currentTarget as HTMLElement;
        el.style.transform = '';
        // Clearing (rather than restoring a literal) hands the shadow back to
        // the Surface class.
        if (!v.pulse) el.style.boxShadow = '';
      }}
    >
      {/* Header — the state rail on the card's edge is the ambient colour, so
          the label + glyph are the only status marks needed here. */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '12px 14px 8px' }}>
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            fontSize: '0.9rem',
            fontWeight: 700,
            color: 'var(--wks-text-primary)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {agent.manager ? (
            <Compass size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
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
            fontSize: '0.72rem',
            fontWeight: 600,
            color: v.color,
            flexShrink: 0,
          }}
        >
          <StatusGlyph
            state={agent.sessionId && !hubOffline ? state : undefined}
            size={13}
            strokeWidth={2.2}
            accent="currentColor"
          />
          {v.label}
        </span>
        {onInspect && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onInspect();
            }}
            title="Inspect (plan · flows · agents · files · usage) — press i"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              flexShrink: 0,
              width: 22,
              height: 22,
              padding: 0,
              borderRadius: 'var(--wks-radius-md)',
              // Ghost control: no resting outline, the hover fill is the whole
              // affordance — one fewer rectangle per card at rest.
              border: 'none',
              background: 'transparent',
              color: 'var(--wks-text-faint)',
              cursor: 'pointer',
              transition: 'color 0.12s, background-color 0.12s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
              (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
              (e.currentTarget as HTMLElement).style.background = 'transparent';
            }}
          >
            <Maximize2 size={13} strokeWidth={2} />
          </button>
        )}
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
        {stats.model && (
          <span style={{ color: 'var(--wks-text-secondary)' }}>{shortModelLabel(stats.model)}</span>
        )}
        {hub && <HubChip name={hub} offline={hubOffline} />}
        {turns > 0 && (
          <span>
            · {turns} turn{turns > 1 ? 's' : ''}
          </span>
        )}
        {snapshot?.lastActivity ? <span>· {relTime(snapshot.lastActivity)}</span> : null}
        {stale && (
          <span
            title={`Says "${v.label}" but nothing has arrived since ${relTime(snapshot?.lastActivity)} — the stream may have stalled.`}
            style={{
              color: 'var(--wks-warning)',
              fontWeight: 700,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 3,
            }}
          >
            <AlertTriangle size={11} strokeWidth={2} /> stale
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
          plan={plan}
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
            fontSize: '0.72rem',
            color: 'var(--wks-accent)',
            fontWeight: 600,
          }}
        >
          {runningWf.length > 0 && (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
              <Settings size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
              {runningWf[0].name || 'workflow'}
            </span>
          )}
          {runningSubs > 0 && (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
              <Diamond size={10} strokeWidth={2.25} style={{ flexShrink: 0 }} />
              {runningSubs} subagent{runningSubs > 1 ? 's' : ''}
            </span>
          )}
        </div>
      )}

      {/* Metrics: context bar + cost */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 14px 10px' }}>
        {ctxPct !== undefined ? (
          <>
            <span
              data-testid="agent-row-context-bar"
              aria-label={`Active context ${Math.round(ctxPct)}% of runtime-confirmed window`}
              style={{
                flex: 1,
                height: 5,
                borderRadius: 'var(--wks-radius-pill)',
                background: 'var(--wks-border-subtle)',
                overflow: 'hidden',
              }}
            >
              <span
                style={{
                  display: 'block',
                  height: '100%',
                  width: `${Math.min(100, Math.max(2, ctxPct))}%`,
                  background: ctxColor(ctxPct),
                }}
              />
            </span>
            <span
              style={{
                fontSize: '0.66rem',
                color: ctxColor(ctxPct),
                fontVariantNumeric: 'tabular-nums',
                flexShrink: 0,
              }}
            >
              {ctxTokens !== undefined ? `${fmtTokens(ctxTokens)} · ` : ''}
              {Math.round(ctxPct)}%
            </span>
            {/* Cost only when known — codex has no pricing, so no fake $0.00. */}
            {stats.costUSD !== undefined && (
              <span
                title={
                  stats.recorded ? 'Last recorded for this session — not a live reading' : undefined
                }
                style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}
              >
                {fmtUSD(stats.costUSD)}
              </span>
            )}
          </>
        ) : stats.costUSD !== undefined || stats.billedTokens !== undefined ? (
          // No live context reading, but this session HAS recorded figures.
          // Labelled as last-recorded so it can't be read as live spend.
          <span
            title="Last recorded for this session — not a live reading"
            style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}
          >
            {[
              stats.billedTokens !== undefined ? `${fmtTokens(stats.billedTokens)} billed` : '',
              stats.costUSD !== undefined ? fmtUSD(stats.costUSD) : '',
            ]
              .filter(Boolean)
              .join(' · ')}{' '}
            <span style={{ color: 'var(--wks-text-disabled)' }}>last recorded</span>
          </span>
        ) : (
          // Nothing live and nothing recorded. "No usage yet" claimed the
          // stronger of the two facts available — that this agent has spent
          // nothing — when the weaker one is all we have, and when the source
          // is unreachable we do not even have that. Say which it is.
          <span
            title={agent.sessionId ? absentUsageTitle(usageUnavailable) : undefined}
            style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}
          >
            {agent.sessionId ? (usageUnavailable ? 'Usage unavailable' : 'No usage recorded') : ''}
          </span>
        )}
      </div>

      {/* Action zone: approve / answer a question / compose a message */}
      {(hasAction || showCompose) && (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            // Depth 2 and last: a flush footer band separated by its fill
            // alone. Not a `Surface` — it has no corners of its own, it just
            // shades the bottom of the one the card already is. No borderTop:
            // the fill step is the separation — but it has to be a step you
            // can actually see. `--wks-bg-elevated` is not: against the card's
            // own `--wks-bg-surface` it is rgb(32,32,36) vs rgb(30,30,33) in
            // the default dark theme, and the two are equal-ish in several
            // others. A translucent foreground wash instead: it is a fixed
            // fraction of the theme's own text/background contrast, so it
            // reads in all 18 themes (light included), and because it composites
            // over whatever fill the card currently has it keeps its step on
            // hover too — `--wks-bg-hover` would have matched the card's own
            // hover fill exactly and vanished under the pointer.
            padding: '10px 14px 12px',
            background: 'color-mix(in srgb, var(--wks-text-primary) 7%, transparent)',
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
                  color: 'var(--wks-warning)',
                  marginBottom: 6,
                }}
              >
                Permission: {approvalItem.title}
                {approvalItem.detail ? ` — ${approvalItem.detail}` : ''}
              </div>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                <button
                  onClick={() => approve(approvalItem, 'yes')}
                  style={qa('var(--wks-success)')}
                >
                  Allow
                </button>
                <button
                  onClick={() => approve(approvalItem, 'always')}
                  style={qa('var(--wks-accent)')}
                >
                  Allow all
                </button>
                <button onClick={() => approve(approvalItem, 'no')} style={qa('var(--wks-error)')}>
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
                  fontSize: '0.72rem',
                  padding: '6px 8px',
                  borderRadius: 'var(--wks-radius-md)',
                  lineHeight: 1.4,
                  // Fill-only inset field. Focus is still visible: the deck's
                  // `textarea:focus-visible` rule paints an accent glow ring,
                  // and elsewhere the browser's own ring shows through.
                  border: 'none',
                  background: 'var(--wks-bg-input)',
                  color: 'var(--wks-text-primary)',
                  fontFamily: 'inherit',
                }}
              />
              <button
                onClick={submitDraft}
                disabled={!draft.trim()}
                style={{
                  fontSize: '0.72rem',
                  fontWeight: 600,
                  fontFamily: 'inherit',
                  padding: '6px 12px',
                  borderRadius: 'var(--wks-radius-md)',
                  cursor: draft.trim() ? 'pointer' : 'default',
                  flexShrink: 0,
                  border: 'none',
                  background: draft.trim() ? 'var(--wks-accent)' : 'var(--wks-bg-hover)',
                  color: draft.trim() ? 'var(--wks-text-on-accent)' : 'var(--wks-text-disabled)',
                }}
              >
                Send
              </button>
            </div>
          )}
        </div>
      )}
    </Surface>
  );
};

/**
 * Quick-action chip. Fill-only (a colour-mix tint of its own semantic colour),
 * never outlined — four outlined pills inside an already-shaded footer was a
 * third ring of rectangles.
 */
function qa(color: string): React.CSSProperties {
  return {
    fontSize: '0.72rem',
    fontWeight: 600,
    fontFamily: 'inherit',
    padding: '4px 12px',
    borderRadius: 'var(--wks-radius-md)',
    border: 'none',
    background: `color-mix(in srgb, ${color} 16%, transparent)`,
    color,
    cursor: 'pointer',
  };
}
