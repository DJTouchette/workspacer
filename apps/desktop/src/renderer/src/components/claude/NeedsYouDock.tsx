import React from 'react';
import type { PendingApproval, PendingQuestion } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { ApprovalPrompt } from './ApprovalPrompt';
import { QuestionPicker } from './QuestionPicker';

/**
 * Sticky "needs you" zone docked above the composer. Approvals and question
 * pickers render here — pinned where the user is about to type — instead of
 * floating in the conversation scrollback where they can be scrolled away.
 *
 * A pending question always wins over an approval: claude can fire
 * PermissionRequest in the same turn as an AskUserQuestion PreToolUse, and the
 * approval card from the former is stale once the picker is up.
 */
export const NeedsYouDock: React.FC<{
  approval: PendingApproval | null;
  questions: PendingQuestion[] | null;
  onApprove: (response: 'yes' | 'no') => void;
  onAnswer: (payload: { option?: number; text?: string; answers?: string[] }) => void;
  onDecline?: () => void;
}> = ({ approval, questions, onApprove, onAnswer, onDecline }) => {
  const hasQuestion = !!(questions && questions.length > 0);
  if (!hasQuestion && !approval) return null;

  const accent = hasQuestion ? colors.accent : colors.error;
  const label = hasQuestion ? 'Claude is asking you' : 'Approval needed';
  // A single AskUserQuestion can carry several questions; the picker renders them
  // all together, so label the count honestly ("N questions") rather than a fake
  // "1 of N" position that never advances.
  const total = hasQuestion ? questions!.length : 0;

  return (
    <div
      style={{
        flexShrink: 0,
        borderTop: `1px solid ${colors.border}`,
        backgroundColor: 'rgba(255,255,255,0.02)',
        maxHeight: '45%',
        overflowY: 'auto',
        animation: 'claudeSlideUp 0.18s ease-out',
      }}
    >
      <div style={{ maxWidth: 1040, margin: '0 auto', padding: '6px 16px 2px 16px' }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            fontSize: '0.64rem',
            fontWeight: 700,
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
            color: accent,
          }}
        >
          <span
            style={{
              display: 'inline-block',
              width: 6,
              height: 6,
              borderRadius: '50%',
              backgroundColor: accent,
              animation: 'claudePulseDot 1.4s ease-in-out infinite',
            }}
          />
          {label}
          {total > 1 && (
            <span
              style={{
                marginLeft: 'auto',
                color: colors.muted,
                fontWeight: 600,
                letterSpacing: 0,
                textTransform: 'none',
              }}
            >
              {total} questions
            </span>
          )}
        </div>
        {hasQuestion ? (
          <QuestionPicker questions={questions!} onAnswer={onAnswer} onDecline={onDecline} />
        ) : (
          <ApprovalPrompt approval={approval!} onRespond={onApprove} />
        )}
      </div>
    </div>
  );
};
