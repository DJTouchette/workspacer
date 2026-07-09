import React, { useState } from 'react';
import type { PendingQuestion } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';

/**
 * Agent question resolver — used everywhere a question can be answered
 * (conversation dock, inbox drawer, fleet cards). One question at a time:
 * multi-question sets step through with a progress header and a back
 * affordance, collecting one raw answer per question and submitting them
 * together as `answers: string[]` (the daemon maps numeric picks to option
 * labels on both transports, and types them sequentially into the PTY picker).
 * A single question keeps the immediate `{option}` / `{text}` fast-path.
 *
 * Raw answer encoding per question: option pick → its 1-indexed number as a
 * string; custom text → the text; multi-select → chosen labels joined ", ".
 *
 * `onDecline`, when provided, declines the whole request — the caller cancels
 * the agent's turn (SIGINT) rather than submitting any answer.
 */
export const QuestionPicker: React.FC<{
  questions: PendingQuestion[];
  onAnswer: (payload: { option?: number; text?: string; answers?: string[] }) => void;
  onDecline?: () => void;
}> = ({ questions, onAnswer, onDecline }) => {
  const [idx, setIdx] = useState(0);
  const [drafts, setDrafts] = useState<string[]>(() => questions.map(() => ''));
  const [picked, setPicked] = useState<(string | null)[]>(() => questions.map(() => null));
  const [customText, setCustomText] = useState('');
  const [multiPicks, setMultiPicks] = useState<Set<number>>(new Set());
  const [done, setDone] = useState(false);

  const total = questions.length;
  const q = questions[Math.min(idx, total - 1)];
  if (!q || done) return null;
  const isLast = idx === total - 1;
  const multi = !!q.multi_select;

  /** Record this question's raw answer, then advance or submit the set. */
  const commit = (raw: string, displayLabel: string) => {
    if (total === 1) {
      // Single question: preserve the original fast-path payloads.
      const n = Number(raw);
      if (Number.isInteger(n) && String(n) === raw) onAnswer({ option: n });
      else onAnswer({ text: raw });
      setDone(true);
      return;
    }
    const nextDrafts = [...drafts];
    nextDrafts[idx] = raw;
    setDrafts(nextDrafts);
    const nextPicked = [...picked];
    nextPicked[idx] = displayLabel;
    setPicked(nextPicked);
    setCustomText('');
    setMultiPicks(new Set());
    if (isLast) {
      onAnswer({ answers: nextDrafts.map((d, i) => d || nextPicked[i] || '') });
      setDone(true);
    } else {
      setIdx(idx + 1);
    }
  };

  const commitMulti = () => {
    const labels = (q.options ?? [])
      .filter((_, oi) => multiPicks.has(oi))
      .map((o) => o.label)
      .join(', ');
    if (labels) commit(labels, labels);
  };

  const submitCustom = () => {
    const t = customText.trim();
    if (t) commit(t, t);
  };

  const scale = 'var(--claude-gui-font-scale, 1)';

  return (
    <div
      style={{
        padding: '13px 15px 14px',
        margin: '8px 0',
        borderRadius: 'var(--wks-radius-md)',
        backgroundColor: 'var(--wks-accent-bg)',
        border: '1px solid color-mix(in srgb, var(--wks-accent) 32%, transparent)',
        borderLeft: '2px solid var(--wks-accent)',
        animation: 'claudeFadeIn 0.2s ease-out',
        textAlign: 'left',
      }}
    >
      {/* Header: back + chip + stepper */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
        {total > 1 && idx > 0 && (
          <button
            onClick={() => setIdx(idx - 1)}
            title="Previous question"
            style={{
              border: 'none',
              background: 'transparent',
              color: colors.muted,
              cursor: 'pointer',
              padding: '0 2px',
              fontSize: '0.95rem',
              lineHeight: 1,
              fontFamily: 'inherit',
            }}
          >
            {'‹'}
          </button>
        )}
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            fontSize: `calc(0.6rem * ${scale})`,
            color: colors.accent,
            fontWeight: 700,
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
          }}
        >
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              backgroundColor: 'var(--wks-accent)',
              animation: 'claudePulseDot 1.4s ease-in-out infinite',
            }}
          />
          {q.header || 'Question'}
        </span>
        {total > 1 && (
          <span
            style={{
              marginLeft: 'auto',
              fontSize: `calc(0.64rem * ${scale})`,
              color: colors.muted,
              fontVariantNumeric: 'tabular-nums',
              display: 'flex',
              alignItems: 'center',
              gap: 6,
            }}
          >
            <span style={{ display: 'flex', gap: 3 }}>
              {questions.map((_, i) => (
                <span
                  key={i}
                  style={{
                    width: 5,
                    height: 5,
                    borderRadius: '50%',
                    backgroundColor:
                      i < idx
                        ? 'var(--wks-accent)'
                        : i === idx
                          ? colors.textBright
                          : 'color-mix(in srgb, var(--wks-text-faint) 40%, transparent)',
                  }}
                />
              ))}
            </span>
            {idx + 1} of {total}
          </span>
        )}
      </div>

      {/* The question itself */}
      <div
        style={{
          fontSize: `calc(0.92rem * ${scale})`,
          color: colors.textBright,
          fontWeight: 600,
          lineHeight: 1.5,
          marginBottom: 11,
        }}
      >
        {q.question}
      </div>

      {/* Options — number/checkbox badge + label + description */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {(q.options ?? []).map((opt, oi) => {
          const selected = multi ? multiPicks.has(oi) : picked[idx] === opt.label;
          return (
            <button
              key={opt.label}
              onClick={() => {
                if (multi) {
                  const next = new Set(multiPicks);
                  if (next.has(oi)) next.delete(oi);
                  else next.add(oi);
                  setMultiPicks(next);
                } else {
                  commit(String(oi + 1), opt.label);
                }
              }}
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                gap: 10,
                textAlign: 'left',
                padding: '9px 11px',
                borderRadius: 'var(--wks-radius-sm)',
                border: `1px solid ${selected ? 'var(--wks-accent)' : colors.borderSubtle}`,
                backgroundColor: selected
                  ? 'color-mix(in srgb, var(--wks-accent) 12%, transparent)'
                  : 'rgba(255,255,255,0.02)',
                color: colors.text,
                cursor: 'pointer',
                fontFamily: 'inherit',
                transition: 'border-color 0.12s, background-color 0.12s',
              }}
              onMouseEnter={(e) => {
                if (!selected) {
                  e.currentTarget.style.borderColor = 'var(--wks-border-active)';
                  e.currentTarget.style.backgroundColor = 'rgba(255,255,255,0.045)';
                }
              }}
              onMouseLeave={(e) => {
                if (!selected) {
                  e.currentTarget.style.borderColor = colors.borderSubtle;
                  e.currentTarget.style.backgroundColor = 'rgba(255,255,255,0.02)';
                }
              }}
            >
              {/* Badge */}
              <span
                style={{
                  flexShrink: 0,
                  marginTop: 1,
                  width: 18,
                  height: 18,
                  borderRadius: multi ? 5 : 6,
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: `calc(0.68rem * ${scale})`,
                  fontWeight: 700,
                  fontVariantNumeric: 'tabular-nums',
                  border: `1px solid ${selected ? 'var(--wks-accent)' : colors.borderSubtle}`,
                  backgroundColor: selected ? 'var(--wks-accent)' : 'transparent',
                  color: selected ? '#fff' : colors.muted,
                }}
              >
                {multi ? (selected ? '✓' : '') : `${oi + 1}`}
              </span>
              <span style={{ flex: 1, minWidth: 0 }}>
                <span
                  style={{
                    display: 'block',
                    fontWeight: 600,
                    fontSize: `calc(0.82rem * ${scale})`,
                    color: colors.textBright,
                    lineHeight: 1.4,
                  }}
                >
                  {opt.label}
                </span>
                {opt.description && (
                  <span
                    style={{
                      display: 'block',
                      color: colors.muted,
                      fontSize: `calc(0.74rem * ${scale})`,
                      lineHeight: 1.45,
                      marginTop: 2,
                    }}
                  >
                    {opt.description}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>

      {/* Multi-select confirm */}
      {multi && (
        <button
          onClick={commitMulti}
          disabled={multiPicks.size === 0}
          style={{
            marginTop: 9,
            fontSize: `calc(0.74rem * ${scale})`,
            fontWeight: 600,
            padding: '6px 15px',
            borderRadius: 'var(--wks-radius-sm)',
            border: 'none',
            backgroundColor: multiPicks.size > 0 ? 'var(--wks-accent)' : 'var(--wks-bg-hover)',
            color: multiPicks.size > 0 ? '#fff' : colors.mutedDim,
            cursor: multiPicks.size > 0 ? 'pointer' : 'default',
            fontFamily: 'inherit',
          }}
        >
          {isLast && total > 1 ? 'Finish' : total > 1 ? 'Next' : 'Send'}
        </button>
      )}

      {/* Custom answer */}
      {!multi && (
        <div style={{ display: 'flex', gap: 6, marginTop: 11 }}>
          <input
            placeholder="Or type your own answer…"
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitCustom();
            }}
            style={{
              flex: 1,
              fontSize: `calc(0.78rem * ${scale})`,
              padding: '6px 10px',
              borderRadius: 'var(--wks-radius-sm)',
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.03)',
              color: colors.text,
              outline: 'none',
              fontFamily: 'inherit',
            }}
          />
          <button
            onClick={submitCustom}
            disabled={!customText.trim()}
            style={{
              fontSize: `calc(0.74rem * ${scale})`,
              fontWeight: 600,
              padding: '6px 15px',
              borderRadius: 'var(--wks-radius-sm)',
              border: 'none',
              backgroundColor: customText.trim() ? 'var(--wks-accent)' : 'var(--wks-bg-hover)',
              color: customText.trim() ? '#fff' : colors.mutedDim,
              cursor: customText.trim() ? 'pointer' : 'default',
              fontFamily: 'inherit',
            }}
          >
            {total > 1 && !isLast ? 'Next' : 'Send'}
          </button>
        </div>
      )}

      {/* Decline — cancels the agent's turn instead of answering */}
      {onDecline && (
        <div
          style={{
            display: 'flex',
            justifyContent: 'flex-end',
            marginTop: 10,
            paddingTop: 9,
            borderTop: `1px solid ${colors.borderSubtle}`,
          }}
        >
          <button
            onClick={onDecline}
            title="Decline this question and stop the agent's current turn"
            style={{
              fontSize: `calc(0.72rem * ${scale})`,
              fontWeight: 600,
              padding: '5px 12px',
              borderRadius: 'var(--wks-radius-sm)',
              border: `1px solid ${colors.borderSubtle}`,
              background: 'transparent',
              color: colors.muted,
              cursor: 'pointer',
              fontFamily: 'inherit',
              transition: 'color 0.12s, border-color 0.12s, background-color 0.12s',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.color = colors.error;
              e.currentTarget.style.borderColor = 'color-mix(in srgb, var(--wks-error) 45%, transparent)';
              e.currentTarget.style.backgroundColor = 'color-mix(in srgb, var(--wks-error) 8%, transparent)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.color = colors.muted;
              e.currentTarget.style.borderColor = colors.borderSubtle;
              e.currentTarget.style.backgroundColor = 'transparent';
            }}
          >
            Decline &amp; stop
          </button>
        </div>
      )}
    </div>
  );
};
