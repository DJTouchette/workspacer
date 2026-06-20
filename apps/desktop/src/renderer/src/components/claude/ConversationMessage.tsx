import React, { useMemo } from 'react';
import type { ConversationTurn } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { parseMarkdownBlocks } from '../markdown';
import { InlineWorkLog } from './InlineWorkLog';
import { DiffView, hasDiff } from './DiffView';

const ConversationMessageInner: React.FC<{ turn: ConversationTurn; isLast?: boolean }> = ({ turn, isLast }) => {
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

  // Assistant message
  const diffCalls = (turn.toolCalls ?? []).filter(tc => hasDiff(tc));
  const writeCalls = (turn.toolCalls ?? []).filter(tc => tc.name === 'Write' && tc.input?.content);

  return (
    <div style={{
      marginBottom: 12,
      animation: 'claudeFadeIn 0.2s ease-out',
    }}>
      {/* Collapsible tool call summary */}
      {turn.toolCalls && turn.toolCalls.length > 0 && (
        <InlineWorkLog toolCalls={turn.toolCalls} />
      )}

      {/* Inline diffs for Edit/MultiEdit — shown directly in chat */}
      {diffCalls.map(tc => (
        <DiffView
          key={tc.id}
          oldStr={tc.input?.old_string ?? ''}
          newStr={tc.input?.new_string ?? ''}
          filePath={tc.input?.file_path}
        />
      ))}

      {/* Inline file content for Write — shown directly in chat */}
      {writeCalls.map(tc => (
        <div key={tc.id} style={{
          margin: '6px 0',
          borderRadius: 6,
          overflow: 'hidden',
          border: `1px solid ${colors.borderSubtle}`,
          maxHeight: 600,
          overflowY: 'auto',
        }}>
          <div style={{
            padding: '4px 10px',
            backgroundColor: 'rgba(255,255,255,0.03)',
            color: colors.muted,
            fontSize: '0.65rem',
            borderBottom: `1px solid ${colors.borderSubtle}`,
            display: 'flex',
            alignItems: 'center',
            gap: 6,
          }}>
            <span style={{ color: colors.success }}>+</span>
            {tc.input?.file_path?.split(/[/\\]/).pop() ?? 'new file'}
          </div>
          <div style={{ margin: 0, fontSize: '0.7rem', fontFamily: 'var(--claude-mono-font, monospace)' }}>
            {(tc.input?.content?.slice(0, 2000) ?? '').split('\n').map((line: string, i: number) => (
              <div key={`${i}-${line.slice(0, 24)}`} style={{ display: 'flex', lineHeight: 1.5 }}>
                <span style={{ color: 'rgba(150,230,170,0.35)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '0 6px 0 0', fontSize: '0.6rem', borderRight: '1px solid rgba(74,222,128,0.1)' }}>{i + 1}</span>
                <span style={{ color: 'rgb(150, 230, 170)', padding: '0 8px', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
              </div>
            ))}
            {(tc.input?.content?.length ?? 0) > 2000 && (
              <div style={{ padding: '2px 8px 2px 44px', color: colors.muted, fontSize: '0.65rem' }}>...</div>
            )}
          </div>
        </div>
      ))}

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
