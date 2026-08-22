/**
 * A fleet/supervisor system message as a structured card. The wakes that
 * workspacer injects into a manager's conversation ("[fleet] Worker finished",
 * "[fleet] Catch-up", "[supervisor] … blocked") arrive through claudemon's
 * plain-text /message endpoint, so they land in the transcript as ordinary
 * user turns — one giant paragraph each. ConversationMessage recognizes them
 * (shared/fleetMessages.ts parser, same module that BUILDS them) and renders
 * this instead of a user bubble: a kind badge, one row per worker with a
 * clickable session chip, and the worker's last-reply excerpt collapsed
 * behind a toggle. Anything the parser doesn't match stays a raw bubble.
 */
import React, { useState } from 'react';
import { AlertTriangle, Check, ChevronRight, History } from 'lucide-react';
import type { FleetMessage, FleetMessageEntry } from '../../../../main/shared/fleetMessages';
import { claudeColors as colors } from '../claude-shared';
import { Surface } from '../Surface';
import { useAttentionOptional } from '../../contexts/AttentionContext';
import { TurnStamp } from './ConversationMessage';

/** Per-kind presentation: overline text, icon, and the Surface tone rail. */
const KIND_META: Record<
  FleetMessage['kind'],
  {
    badge: string;
    badgePlural: string;
    icon: React.ComponentType<{ size?: number | string; strokeWidth?: number | string }>;
    tone: string;
  }
> = {
  'worker-finished': {
    badge: 'Fleet · worker finished',
    badgePlural: 'Fleet · workers finished',
    icon: Check,
    tone: 'var(--wks-success)',
  },
  'catch-up': {
    badge: 'Fleet · catch-up',
    badgePlural: 'Fleet · catch-up',
    icon: History,
    tone: 'var(--wks-success)',
  },
  threshold: {
    badge: 'Fleet · threshold crossed',
    badgePlural: 'Fleet · thresholds crossed',
    icon: AlertTriangle,
    tone: 'var(--wks-warning)',
  },
  blocked: {
    badge: 'Supervisor · agent blocked',
    badgePlural: 'Supervisor · agents blocked',
    icon: AlertTriangle,
    tone: 'var(--wks-warning)',
  },
};

/** The worker-finished card's HONEST face when every worker in the wake died:
 *  a green check reading "worker finished" over an out-of-credits crash is the
 *  same lie the wake header used to tell (see shared/workerFailure). Mixed
 *  wakes keep the normal face — "finished" is true of the entries that did —
 *  and each failed row carries its own FAILED chip. */
const FAILED_META = {
  badge: 'Fleet · worker FAILED',
  badgePlural: 'Fleet · workers FAILED',
  icon: AlertTriangle,
  tone: 'var(--wks-error)',
};

/** Blocked-on chip hue follows the ambient-status vocabulary:
 *  approval → error (waiting_approval), question → purple (waiting_input). */
const BLOCKED_ON_COLOR: Record<'approval' | 'question', string> = {
  approval: colors.error,
  question: colors.purple,
};

/** Last path segment, for the compact cwd chip (full path in the tooltip). */
function basename(p: string): string {
  const parts = p.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || p;
}

/** `session:<id>` as a chip. Clickable (focuses the worker's workspace) when
 *  the fleet still knows the session; plain otherwise (ended and gone). */
const SessionChip: React.FC<{ sessionId: string }> = ({ sessionId }) => {
  const attention = useAttentionOptional();
  const agent = attention?.agents.find((a) => a.sessionId === sessionId);
  const short = sessionId.length > 10 ? `${sessionId.slice(0, 8)}…` : sessionId;
  const style: React.CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    padding: '1px 6px',
    borderRadius: 'var(--wks-radius-pill)',
    border: `1px solid ${colors.borderSubtle}`,
    background: 'transparent',
    fontFamily: 'var(--wks-font-mono)',
    fontSize: '0.6rem',
    color: agent ? colors.accent : colors.muted,
    whiteSpace: 'nowrap',
    flexShrink: 0,
  };
  if (!agent || !attention) {
    return (
      <span title={`session:${sessionId} (no longer in the fleet)`} style={style}>
        session:{short}
      </span>
    );
  }
  return (
    <button
      title={`Open ${agent.name || 'agent'} (session:${sessionId})`}
      onClick={() => attention.openAgent(agent.id)}
      style={{ ...style, cursor: 'pointer', fontWeight: 500 }}
    >
      session:{short}
    </button>
  );
};

/** One worker row: label · session chip · cwd/blocked-on, then the collapsed
 *  last-reply excerpt (if the wake carried one). */
const EntryRow: React.FC<{ entry: FleetMessageEntry }> = ({ entry }) => {
  const [showReply, setShowReply] = useState(false);
  return (
    <div style={{ padding: '6px 12px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
        <span
          style={{
            fontSize: 'calc(0.76rem * var(--claude-gui-font-scale, 1))',
            fontWeight: 600,
            color: colors.textBright,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {entry.label}
        </span>
        <SessionChip sessionId={entry.sessionId} />
        {entry.stopped && (
          <span
            style={{
              padding: '1px 6px',
              borderRadius: 'var(--wks-radius-pill)',
              fontSize: '0.6rem',
              fontWeight: 600,
              color: colors.error,
              background: `color-mix(in srgb, ${colors.error} 12%, transparent)`,
              whiteSpace: 'nowrap',
              flexShrink: 0,
            }}
          >
            stopped/killed
          </span>
        )}
        {entry.failed && (
          <span
            title={entry.failed}
            style={{
              padding: '1px 6px',
              borderRadius: 'var(--wks-radius-pill)',
              fontSize: '0.6rem',
              fontWeight: 600,
              color: colors.error,
              background: `color-mix(in srgb, ${colors.error} 12%, transparent)`,
              maxWidth: 220,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              flexShrink: 0,
            }}
          >
            FAILED: {entry.failed}
          </span>
        )}
        {entry.crossed && (
          <span
            title={entry.crossed}
            style={{
              padding: '1px 6px',
              borderRadius: 'var(--wks-radius-pill)',
              fontSize: '0.6rem',
              fontWeight: 600,
              color: colors.warning,
              background: `color-mix(in srgb, ${colors.warning} 12%, transparent)`,
              whiteSpace: 'nowrap',
              flexShrink: 0,
            }}
          >
            {entry.crossed}
          </span>
        )}
        {entry.blockedOn && (
          <span
            style={{
              padding: '1px 6px',
              borderRadius: 'var(--wks-radius-pill)',
              fontSize: '0.6rem',
              fontWeight: 600,
              color: BLOCKED_ON_COLOR[entry.blockedOn],
              background: `color-mix(in srgb, ${BLOCKED_ON_COLOR[entry.blockedOn]} 12%, transparent)`,
              whiteSpace: 'nowrap',
              flexShrink: 0,
            }}
          >
            {entry.blockedOn === 'approval' ? 'needs approval' : 'has a question'}
          </span>
        )}
        {entry.cwd && entry.cwd !== '?' && (
          <span
            title={entry.cwd}
            style={{
              fontSize: '0.66rem',
              fontFamily: 'var(--wks-font-mono)',
              color: colors.muted,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {basename(entry.cwd)}
          </span>
        )}
      </div>
      {entry.lastReply && (
        <div style={{ marginTop: 4 }}>
          <button
            onClick={() => setShowReply((v) => !v)}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              padding: 0,
              border: 'none',
              background: 'transparent',
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: '0.66rem',
              fontWeight: 600,
              color: colors.muted,
            }}
          >
            <ChevronRight
              size={11}
              aria-hidden
              style={{
                transform: showReply ? 'rotate(90deg)' : 'none',
                transition: 'transform 0.15s',
              }}
            />
            last reply
          </button>
          {showReply && (
            <div
              style={{
                marginTop: 4,
                padding: '6px 10px',
                borderLeft: `2px solid ${colors.borderSubtle}`,
                fontSize: 'calc(0.76rem * var(--claude-gui-font-scale, 1))',
                lineHeight: 1.55,
                color: colors.text,
                wordBreak: 'break-word',
              }}
            >
              {entry.lastReply}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

const FleetMessageCardInner: React.FC<{
  message: FleetMessage;
  timestamp?: number;
  showTimestamp?: boolean;
}> = ({ message, timestamp, showTimestamp }) => {
  const allFailed =
    message.kind === 'worker-finished' &&
    message.entries.length > 0 &&
    message.entries.every((e) => e.failed);
  const meta = allFailed ? FAILED_META : KIND_META[message.kind];
  const Icon = meta.icon;
  const badge = message.entries.length > 1 ? meta.badgePlural : meta.badge;
  return (
    <Surface
      elevation="raised"
      tone={meta.tone}
      style={{
        margin: '4px 0 12px 0',
        overflow: 'hidden',
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '7px 12px 0 12px',
        }}
      >
        <span style={{ display: 'flex', alignItems: 'center', color: meta.tone }}>
          <Icon size={12} strokeWidth={2} />
        </span>
        <span
          style={{
            fontSize: '0.6rem',
            fontWeight: 700,
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
            color: colors.muted,
            flex: 1,
            minWidth: 0,
          }}
        >
          {badge}
          {message.entries.length > 1 ? ` (${message.entries.length})` : ''}
        </span>
        {showTimestamp && <TurnStamp ms={timestamp} />}
      </div>
      <div style={{ padding: '2px 0 4px 0' }}>
        {message.entries.map((e) => (
          <EntryRow key={e.sessionId} entry={e} />
        ))}
      </div>
    </Surface>
  );
};

export const FleetMessageCard = React.memo(FleetMessageCardInner);
