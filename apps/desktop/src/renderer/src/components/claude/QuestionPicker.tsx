import React, { useState } from 'react';
import type { PendingQuestion } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';

export const QuestionPicker: React.FC<{
  questions: PendingQuestion[];
  onAnswer: (payload: { option?: number; text?: string; answers?: string[] }) => void;
}> = ({ questions, onAnswer }) => {
  const [customText, setCustomText] = useState('');
  const single = questions.length === 1 ? questions[0] : null;

  return (
    <div
      style={{
        padding: '12px 14px',
        margin: '8px 0',
        borderRadius: 'var(--wks-radius-md)',
        backgroundColor: 'var(--wks-accent-bg)',
        border: `1px solid ${colors.accent}`,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      {questions.map((q, qi) => (
        <div key={q.question} style={{ marginBottom: qi < questions.length - 1 ? 12 : 0 }}>
          {q.header && (
            <div
              style={{
                fontSize: '0.6rem',
                color: colors.mutedDim,
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
                marginBottom: 4,
              }}
            >
              {q.header}
            </div>
          )}
          <div
            style={{
              fontSize: '0.8rem',
              color: colors.textBright,
              fontWeight: 600,
              marginBottom: 8,
            }}
          >
            {q.question}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {(q.options ?? []).map((opt, oi) => (
              <button
                key={opt.label}
                onClick={() => onAnswer({ option: oi + 1 })}
                style={{
                  textAlign: 'left',
                  padding: '6px 10px',
                  borderRadius: 'var(--wks-radius-sm)',
                  border: `1px solid ${colors.border}`,
                  backgroundColor: 'rgba(255,255,255,0.03)',
                  color: colors.text,
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                  fontSize: '0.75rem',
                }}
              >
                <span style={{ color: colors.accent, fontWeight: 700, marginRight: 8 }}>
                  {oi + 1}.
                </span>
                <span style={{ fontWeight: 600 }}>{opt.label}</span>
                {opt.description && (
                  <span style={{ color: colors.muted, marginLeft: 8, fontSize: '0.7rem' }}>
                    — {opt.description}
                  </span>
                )}
              </button>
            ))}
          </div>
        </div>
      ))}

      {single && (
        <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
          <input
            placeholder="Or type a custom answer..."
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && customText.trim()) {
                onAnswer({ text: customText.trim() });
                setCustomText('');
              }
            }}
            style={{
              flex: 1,
              fontSize: '0.75rem',
              padding: '4px 8px',
              borderRadius: 'var(--wks-radius-sm)',
              border: `1px solid ${colors.border}`,
              backgroundColor: 'rgba(255,255,255,0.03)',
              color: colors.text,
              outline: 'none',
              fontFamily: 'inherit',
            }}
          />
          <button
            onClick={() => {
              if (customText.trim()) {
                onAnswer({ text: customText.trim() });
                setCustomText('');
              }
            }}
            disabled={!customText.trim()}
            style={{
              fontSize: '0.7rem',
              padding: '4px 12px',
              borderRadius: 'var(--wks-radius-sm)',
              border: `1px solid ${colors.accent}`,
              backgroundColor: customText.trim() ? 'var(--wks-accent)' : 'transparent',
              color: customText.trim() ? '#fff' : colors.muted,
              cursor: customText.trim() ? 'pointer' : 'default',
              fontFamily: 'inherit',
            }}
          >
            Send
          </button>
        </div>
      )}
    </div>
  );
};
