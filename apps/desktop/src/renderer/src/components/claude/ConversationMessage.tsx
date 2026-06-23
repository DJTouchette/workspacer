import React, { useMemo } from 'react';
import type { ConversationTurn } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { parseMarkdownBlocks } from '../markdown';

const ConversationMessageInner: React.FC<{ turn: ConversationTurn }> = ({ turn }) => {
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
      <div style={{
        display: 'flex',
        justifyContent: 'flex-end',
        marginBottom: 12,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}>
        <div style={{
          maxWidth: '80%',
          padding: '8px 14px',
          borderRadius: '16px 16px 4px 16px',
          backgroundColor: colors.userBubble,
          border: `1px solid ${colors.userBubbleBorder}`,
        }}>
          <pre style={{
            margin: 0,
            fontSize: '0.8rem',
            lineHeight: 1.6,
            color: colors.text,
            fontFamily: 'var(--claude-mono-font, monospace)',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
          }}>
            {turn.content || '(empty)'}
          </pre>
        </div>
      </div>
    );
  }

  // Assistant message — text only. Tool calls (with their diffs) and any
  // sub-agent / workflow runs they spawned render in the WorkCard that follows
  // this message in the timeline, so the chat reads as text → work → text
  // instead of a flat flood of tool rows under every message.
  return (
    <div style={{
      marginBottom: 12,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      {parsedContent ? (
        <div style={{
          paddingLeft: 4,
          fontSize: '0.8rem',
          lineHeight: 1.6,
          color: colors.text,
        }}>
          {parsedContent}
        </div>
      ) : null}
    </div>
  );
};

export const ConversationMessage = React.memo(ConversationMessageInner);
