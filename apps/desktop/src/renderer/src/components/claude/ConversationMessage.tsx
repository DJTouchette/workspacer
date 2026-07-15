import React, { useMemo } from 'react';
import type { ConversationTurn } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { parseMarkdownBlocks } from '../markdown';

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
        fontSize: '0.62rem',
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

const ConversationMessageInner: React.FC<{ turn: ConversationTurn; showTimestamp?: boolean }> = ({
  turn,
  showTimestamp,
}) => {
  const isUser = turn.role === 'user';
  // Memoize per content string; module-level LRU cache in markdown.tsx also
  // deduplicates across instances, so this just avoids the map lookup overhead
  // on re-renders where turn.content hasn't changed.
  const parsedContent = useMemo(
    () => (turn.content ? parseMarkdownBlocks(turn.content) : null),
    [turn.content],
  );

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
            border: `1px solid ${colors.userBubbleBorder}`,
          }}
        >
          <div
            style={{
              fontSize: 'calc(0.8rem * var(--claude-gui-font-scale, 1))',
              lineHeight: 1.6,
              color: colors.textBright,
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            }}
          >
            {turn.content || '(empty)'}
          </div>
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
      style={{
        marginBottom: 12,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      {parsedContent ? (
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
      ) : null}
    </div>
  );
};

export const ConversationMessage = React.memo(ConversationMessageInner);
