import React from 'react';
import type { PendingQuestion } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';

/**
 * A durable record of a resolved AskUserQuestion, injected into the chat
 * scrollback so you can always see what you were asked and what you picked —
 * even after the daemon rebuilds the conversation snapshot (which would wipe a
 * renderer-only bubble). The picker itself lives in the docked NeedsYouDock and
 * disappears on answer; this is the permanent trace it leaves behind.
 */
export interface ResolvedQuestionRecord {
  /** Signature (question texts joined) — the dedupe/react key. */
  sig: string;
  /** Base-conversation length when resolved — where the card anchors. */
  anchorLen: number;
  timestamp: number;
  questions: PendingQuestion[];
  /** One display string per question; null when the turn was declined. */
  answers: string[] | null;
  declined: boolean;
}

export const AnsweredQuestionCard: React.FC<{ record: ResolvedQuestionRecord }> = ({ record }) => {
  const { questions, answers, declined } = record;
  const accent = declined ? colors.error : colors.accent;

  return (
    <div
      style={{
        margin: '10px 0',
        borderRadius: 'var(--wks-radius-md)',
        border: `1px solid ${colors.borderSubtle}`,
        borderLeft: `2px solid ${accent}`,
        backgroundColor: 'rgba(255,255,255,0.02)',
        padding: '9px 13px 11px',
        textAlign: 'left',
      }}
    >
      {/* Header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: 'calc(0.6rem * var(--claude-gui-font-scale, 1))',
          fontWeight: 700,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: accent,
          marginBottom: 7,
        }}
      >
        <span style={{ fontSize: '0.72rem', lineHeight: 1 }}>{declined ? '⊘' : '✓'}</span>
        {declined ? 'Declined' : questions.length > 1 ? 'You answered' : 'You answered'}
      </div>

      {declined ? (
        <div
          style={{
            fontSize: 'calc(0.78rem * var(--claude-gui-font-scale, 1))',
            color: colors.muted,
            lineHeight: 1.5,
          }}
        >
          {questions.length > 1
            ? `You declined ${questions.length} questions — the turn was cancelled.`
            : 'You declined to answer — the turn was cancelled.'}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
          {questions.map((q, i) => (
            <div key={i}>
              <div
                style={{
                  fontSize: 'calc(0.78rem * var(--claude-gui-font-scale, 1))',
                  color: colors.muted,
                  lineHeight: 1.45,
                  marginBottom: 3,
                }}
              >
                {q.question}
              </div>
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 7 }}>
                <span
                  style={{ color: accent, fontSize: '0.72rem', flexShrink: 0, lineHeight: 1.5 }}
                >
                  ↳
                </span>
                <span
                  style={{
                    fontSize: 'calc(0.82rem * var(--claude-gui-font-scale, 1))',
                    color: colors.textBright,
                    fontWeight: 600,
                    lineHeight: 1.45,
                  }}
                >
                  {answers?.[i] ?? '—'}
                </span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
