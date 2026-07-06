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
 */
export const QuestionPicker: React.FC<{
  questions: PendingQuestion[];
  onAnswer: (payload: { option?: number; text?: string; answers?: string[] }) => void;
}> = ({ questions, onAnswer }) => {
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

  return (
    <div
      style={{
        padding: '12px 14px 14px',
        margin: '8px 0',
        borderRadius: 'var(--wks-radius-md)',
        backgroundColor: 'var(--wks-accent-bg)',
        border: '1px solid color-mix(in srgb, var(--wks-accent) 35%, transparent)',
        animation: 'claudeFadeIn 0.2s ease-out',
        textAlign: 'left',
      }}
    >
      {/* Header: chip + stepper */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
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
              fontSize: '0.85rem',
              lineHeight: 1,
              fontFamily: 'inherit',
            }}
          >
            {'‹'}
          </button>
        )}
        <span
          style={{
            fontSize: '0.6rem',
            color: colors.accent,
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.05em',
          }}
        >
          {q.header || 'Question'}
        </span>
        {total > 1 && (
          <span
            style={{
              marginLeft: 'auto',
              fontSize: '0.62rem',
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
          fontSize: '0.85rem',
          color: colors.textBright,
          fontWeight: 600,
          lineHeight: 1.45,
          marginBottom: 10,
        }}
      >
        {q.question}
      </div>

      {/* Options — label + readable description on its own line */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
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
                textAlign: 'left',
                padding: '7px 11px',
                borderRadius: 'var(--wks-radius-sm)',
                border: `1px solid ${
                  selected ? 'var(--wks-accent)' : colors.borderSubtle
                }`,
                backgroundColor: selected
                  ? 'color-mix(in srgb, var(--wks-accent) 10%, transparent)'
                  : 'rgba(255,255,255,0.03)',
                color: colors.text,
                cursor: 'pointer',
                fontFamily: 'inherit',
                transition: 'border-color 0.12s, background-color 0.12s',
              }}
              onMouseEnter={(e) => {
                if (!selected) e.currentTarget.style.borderColor = 'var(--wks-border-active)';
              }}
              onMouseLeave={(e) => {
                if (!selected) e.currentTarget.style.borderColor = colors.borderSubtle;
              }}
            >
              <span style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
                <span
                  style={{
                    color: colors.accent,
                    fontWeight: 700,
                    fontSize: '0.7rem',
                    fontVariantNumeric: 'tabular-nums',
                    flexShrink: 0,
                  }}
                >
                  {multi ? (selected ? '☑' : '☐') : `${oi + 1}`}
                </span>
                <span
                  style={{ fontWeight: 600, fontSize: '0.76rem', color: colors.textBright }}
                >
                  {opt.label}
                </span>
              </span>
              {opt.description && (
                <div
                  style={{
                    color: colors.muted,
                    fontSize: '0.7rem',
                    lineHeight: 1.45,
                    marginTop: 3,
                    paddingLeft: multi ? 22 : 16,
                  }}
                >
                  {opt.description}
                </div>
              )}
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
            marginTop: 8,
            fontSize: '0.7rem',
            fontWeight: 600,
            padding: '5px 14px',
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
        <div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
          <input
            placeholder="Or type your own answer…"
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitCustom();
            }}
            style={{
              flex: 1,
              fontSize: '0.72rem',
              padding: '5px 9px',
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
              fontSize: '0.7rem',
              fontWeight: 600,
              padding: '5px 14px',
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
    </div>
  );
};
