import React, { useState } from 'react';
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
 *
 * The dock is MINIMIZABLE: a long question set eats the pane, and you often
 * need to re-read the conversation before answering. Collapsing leaves a
 * one-line bar (the ask stays pending — nothing is answered or declined) and
 * hands the height back to the transcript. Collapse is keyed to the pending
 * item, so a NEW ask always arrives expanded rather than silently hidden
 * behind a bar the user collapsed for the previous one.
 */
export const NeedsYouDock: React.FC<{
  approval: PendingApproval | null;
  questions: PendingQuestion[] | null;
  onApprove: (response: 'yes' | 'no') => void;
  onAnswer: (payload: {
    option?: number;
    text?: string;
    answers?: string[];
    answerKinds?: string[];
  }) => void;
  onDecline?: () => void;
}> = ({ approval, questions, onApprove, onAnswer, onDecline }) => {
  // Signature of what's currently docked; collapse only applies while it's up.
  const hasQuestion = !!(questions && questions.length > 0);
  const signature = hasQuestion
    ? `q:${questions!.map((q) => q.question).join('\u0000')}`
    : approval
      ? `a:${approval.toolName}:${approval.timestamp}`
      : '';
  const [collapsedSig, setCollapsedSig] = useState<string | null>(null);
  const collapsed = !!signature && collapsedSig === signature;

  if (!hasQuestion && !approval) return null;

  const accent = hasQuestion ? colors.accent : colors.error;
  const label = hasQuestion ? 'Claude is asking you' : 'Approval needed';
  // A single AskUserQuestion can carry several questions; the picker renders them
  // all together, so label the count honestly ("N questions") rather than a fake
  // "1 of N" position that never advances.
  const total = hasQuestion ? questions!.length : 0;
  // Collapsed preview — enough to remember what's waiting without expanding.
  const preview = hasQuestion ? questions![0].question : `${approval!.toolName}`;

  const dot = (
    <span
      style={{
        display: 'inline-block',
        flexShrink: 0,
        width: 6,
        height: 6,
        borderRadius: '50%',
        backgroundColor: accent,
        animation: 'claudePulseDot 1.4s ease-in-out infinite',
      }}
    />
  );

  const toggle = () => setCollapsedSig(collapsed ? null : signature);

  if (collapsed) {
    return (
      <div
        style={{
          flexShrink: 0,
          borderTop: `1px solid ${colors.border}`,
          backgroundColor: 'rgba(255,255,255,0.02)',
        }}
      >
        <button
          onClick={toggle}
          title="Show the pending request (still waiting on you)"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            width: '100%',
            maxWidth: 'var(--wks-chat-width)',
            margin: '0 auto',
            padding: '6px 16px',
            border: 'none',
            background: 'transparent',
            cursor: 'pointer',
            fontFamily: 'inherit',
            textAlign: 'left',
          }}
        >
          {dot}
          <span
            style={{
              fontSize: '0.64rem',
              fontWeight: 700,
              textTransform: 'uppercase',
              letterSpacing: '0.06em',
              color: accent,
              flexShrink: 0,
            }}
          >
            {label}
            {total > 1 ? ` · ${total}` : ''}
          </span>
          <span
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.72rem',
              color: colors.muted,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {preview}
          </span>
          <span style={{ fontSize: '0.68rem', fontWeight: 600, color: accent, flexShrink: 0 }}>
            {hasQuestion ? 'Answer ⌃' : 'Review ⌃'}
          </span>
        </button>
      </div>
    );
  }

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
      <div
        style={{
          maxWidth: 'var(--wks-chat-width)',
          margin: '0 auto',
          padding: '6px 16px 2px 16px',
        }}
      >
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
          {dot}
          {label}
          <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 10 }}>
            {total > 1 && (
              <span
                style={{
                  color: colors.muted,
                  fontWeight: 600,
                  letterSpacing: 0,
                  textTransform: 'none',
                }}
              >
                {total} questions
              </span>
            )}
            <button
              onClick={toggle}
              title="Minimize — keeps the request pending so you can read the conversation"
              aria-label="Minimize"
              style={{
                border: 'none',
                background: 'transparent',
                color: colors.muted,
                cursor: 'pointer',
                fontFamily: 'inherit',
                fontSize: '0.72rem',
                lineHeight: 1,
                padding: '2px 4px',
              }}
            >
              ⌄
            </button>
          </span>
        </div>
        {hasQuestion ? (
          <QuestionPicker questions={questions!} onAnswer={onAnswer} onDecline={onDecline} dense />
        ) : (
          <ApprovalPrompt approval={approval!} onRespond={onApprove} />
        )}
      </div>
    </div>
  );
};
