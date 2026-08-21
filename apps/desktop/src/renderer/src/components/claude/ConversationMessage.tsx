import React, { useMemo } from 'react';
import type { ConversationTurn } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { parseMarkdownBlocks } from '../markdown';
import { CopyTextButton } from './CopyTextButton';
import { MessageImages } from './MessageImages';
import { FleetMessageCard } from './FleetMessageCard';
import { extractImageAttachments, imagePathsInText } from '../../lib/messageImages';
import { parseFleetMessage } from '../../../../main/shared/fleetMessages';
import { Clock } from 'lucide-react';

/** "14:32" (locale 24h/12h per system) for a turn's ms timestamp; '' if unset. */
export function turnTime(ms: number | undefined): string {
  if (!ms || !Number.isFinite(ms)) return '';
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** The small muted HH:MM stamp shown next to a turn when timestamps are on. */
export const TurnStamp: React.FC<{ ms: number | undefined }> = ({ ms }) => {
  const time = turnTime(ms);
  if (!time) return null;
  return (
    <span
      style={{
        fontSize: '0.66rem',
        fontVariantNumeric: 'tabular-nums',
        color: colors.mutedDim,
        whiteSpace: 'nowrap',
        userSelect: 'none',
      }}
    >
      {time}
    </span>
  );
};

/**
 * A sent-but-not-yet-acknowledged user message: 'sending' while it's on its way
 * to an idle agent, 'queued' when it landed behind work in progress and won't be
 * read until the current turn ends. Absent once the daemon echoes the turn back
 * in the transcript, which is the acknowledgement.
 */
export type PendingState = 'sending' | 'queued';

const PENDING_LABEL: Record<PendingState, string> = {
  sending: 'Sending…',
  // Say what "queued" MEANS: a bare "Queued" read as "sending is broken" the
  // moment an agent was mid-turn for a while — the message is fine, it just
  // waits for the prompt to come back.
  queued: 'Queued — sends when this turn ends',
};

/** The un-acknowledged marker beside a pending user bubble. Queued reads a
 *  step brighter than sending: it can sit for a whole long turn, and a label
 *  that dim was being missed entirely. */
const PendingMark: React.FC<{ state: PendingState }> = ({ state }) => (
  <span
    role="status"
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 3,
      fontSize: '0.6rem',
      color: state === 'queued' ? colors.muted : colors.mutedDim,
      whiteSpace: 'nowrap',
      userSelect: 'none',
    }}
  >
    <Clock size={10} strokeWidth={2.25} />
    {PENDING_LABEL[state]}
  </span>
);

const ConversationMessageInner: React.FC<{
  turn: ConversationTurn;
  showTimestamp?: boolean;
  pending?: PendingState;
  /** Session cwd — resolves a relative image path mentioned in the message. */
  cwd?: string;
}> = ({ turn, showTimestamp, pending, cwd }) => {
  const isUser = turn.role === 'user';
  // System-injected fleet/supervisor wakes arrive as plain user turns (the
  // /message endpoint has no metadata channel) — recognize them and render a
  // structured card instead of a paragraph bubble. A parse miss (any other
  // user text, including a malformed wake) falls through to the raw bubble.
  const fleetMessage = useMemo(
    () => (isUser && turn.content ? parseFleetMessage(turn.content) : null),
    [isUser, turn.content],
  );
  // What you attached (markers, stripped from the text) versus what the message
  // merely mentions (left in the text, thumbnailed underneath).
  const attached = useMemo(
    () =>
      isUser
        ? extractImageAttachments(turn.content)
        : { text: turn.content ?? '', paths: [], markers: [] },
    [isUser, turn.content],
  );
  const mentioned = useMemo(
    () => (isUser ? [] : imagePathsInText(turn.content)),
    [isUser, turn.content],
  );
  // Memoize per content string; module-level LRU cache in markdown.tsx also
  // deduplicates across instances, so this just avoids the map lookup overhead
  // on re-renders where turn.content hasn't changed.
  const parsedContent = useMemo(
    () => (turn.content ? parseMarkdownBlocks(turn.content) : null),
    [turn.content],
  );

  if (fleetMessage) {
    return (
      <FleetMessageCard
        message={fleetMessage}
        timestamp={turn.timestamp}
        showTimestamp={showTimestamp}
      />
    );
  }

  if (isUser) {
    return (
      <div
        style={{
          display: 'flex',
          justifyContent: 'flex-end',
          alignItems: 'flex-end',
          gap: 8,
          marginBottom: 12,
          animation: 'claudeFadeIn 0.2s ease-out',
        }}
      >
        {pending && <PendingMark state={pending} />}
        {showTimestamp && <TurnStamp ms={turn.timestamp} />}
        <div
          style={{
            maxWidth: '80%',
            padding: '8px 14px',
            // Speech-tail bubble via the radius tokens so corners follow the
            // user's corner-style setting (square collapses to 0 like the rest).
            borderRadius:
              'var(--wks-radius-lg) var(--wks-radius-lg) var(--wks-radius-sm) var(--wks-radius-lg)',
            backgroundColor: colors.userBubble,
            // Dashed + dimmed until the agent has it: the bubble reads as
            // provisional at a glance, and goes solid the moment the daemon
            // echoes the turn back. Same geometry either way, so nothing shifts
            // when it settles.
            border: `1px ${pending ? 'dashed' : 'solid'} ${colors.userBubbleBorder}`,
            opacity: pending ? 0.72 : 1,
            transition: 'opacity 0.15s',
          }}
        >
          {attached.paths.length > 0 && (
            <MessageImages
              paths={attached.paths}
              cwd={cwd}
              style={{ marginBottom: attached.text ? 8 : 0 }}
              fallback={
                // No tile will ever render (file gone, too big, a format no
                // browser draws): show what was attached rather than an empty
                // bubble. The marker was stripped on extension alone, before
                // anything tried to decode it.
                <div
                  style={{
                    fontSize: 'calc(0.75rem * var(--claude-gui-font-scale, 1))',
                    lineHeight: 1.6,
                    color: colors.muted,
                    wordBreak: 'break-word',
                    marginBottom: attached.text ? 8 : 0,
                  }}
                >
                  {attached.markers.join(' ')}
                </div>
              }
            />
          )}
          {/* An image-only message IS the image — no "(empty)" placeholder under
              a bubble whose whole content is the picture above it. */}
          {(attached.text || attached.paths.length === 0) && (
            <div
              style={{
                fontSize: 'calc(0.8rem * var(--claude-gui-font-scale, 1))',
                lineHeight: 1.6,
                color: colors.textBright,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
              }}
            >
              {attached.text || '(empty)'}
            </div>
          )}
        </div>
      </div>
    );
  }

  // Assistant message — text only. Tool calls (with their diffs) and any
  // sub-agent / workflow runs they spawned render in the WorkCard that follows
  // this message in the timeline, so the chat reads as text → work → text
  // instead of a flat flood of tool rows under every message.
  return (
    <div
      className="wks-hover-host"
      style={{
        // With text the action row below doubles as the trailing gap (it holds
        // its 18px whether or not it's revealed); the empty case keeps the
        // plain margin.
        marginBottom: parsedContent ? 0 : 12,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      {parsedContent ? (
        <>
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-start' }}>
            <div
              style={{
                flex: 1,
                minWidth: 0,
                paddingLeft: 4,
                fontSize: 'calc(0.8rem * var(--claude-gui-font-scale, 1))',
                lineHeight: 1.6,
                color: colors.text,
              }}
            >
              {parsedContent}
            </div>
            {showTimestamp && <TurnStamp ms={turn.timestamp} />}
          </div>
          {mentioned.length > 0 && (
            <MessageImages paths={mentioned} cwd={cwd} style={{ marginTop: 6, marginLeft: 4 }} />
          )}
          {/* Reserved (invisible until hover) so it can't reflow a streaming
              transcript — see .wks-hover-actions. */}
          <div className="wks-hover-actions" style={{ paddingLeft: 1, marginTop: 1 }}>
            <CopyTextButton text={turn.content ?? ''} />
          </div>
        </>
      ) : null}
    </div>
  );
};

export const ConversationMessage = React.memo(ConversationMessageInner);
