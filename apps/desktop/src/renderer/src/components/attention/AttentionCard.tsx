import React from 'react';
import type { AttentionItem, AttentionKind } from '../../types/attention';
import { ApprovalPrompt } from '../claude/ApprovalPrompt';
import { QuestionPicker } from '../claude/QuestionPicker';
import { useAttention, SNOOZE_MINUTES } from '../../contexts/AttentionContext';

const KIND_VISUAL: Record<AttentionKind, { label: string; color: string; glyph: string }> = {
  approval: { label: 'Needs approval', color: 'var(--wks-error)', glyph: '!' },
  question: { label: 'Question', color: 'var(--wks-accent)', glyph: '?' },
  error: { label: 'Error', color: 'var(--wks-error)', glyph: '×' },
  stuck: { label: 'Stuck', color: 'var(--wks-warning)', glyph: '…' },
  bigdiff: { label: 'Review', color: 'var(--wks-warning)', glyph: '±' },
  done: { label: 'Finished', color: 'var(--wks-success)', glyph: '✓' },
};

/** Last path segment, for the compact cwd footer (full path stays in the title). */
function basename(p?: string): string {
  if (!p) return '';
  return (
    p
      .replace(/[/\\]+$/, '')
      .split(/[/\\]/)
      .pop() || p
  );
}

function relTime(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  return `${Math.round(m / 60)}h`;
}

interface Props {
  item: AttentionItem;
  selected: boolean;
}

/**
 * One card in the Triage Inbox. Pure projection of an AttentionItem — embeds
 * ClaudePane's own ApprovalPrompt / QuestionPicker so resolution semantics
 * can't drift, and resolves via the by-sessionId actions on AttentionContext.
 */
export const AttentionCard: React.FC<Props> = ({ item, selected }) => {
  const { approve, answer, openAgent, dismiss, snooze, setSelectedSig, respawn, reviewFile } =
    useAttention();
  const v = KIND_VISUAL[item.kind];
  // Finished / big-diff cards close the loop with review + respawn affordances.
  const canReview = item.kind === 'done' || item.kind === 'bigdiff';
  const canRespawn = item.kind === 'done';

  return (
    <div
      data-attention-sig={item.signature}
      onMouseDown={() => setSelectedSig(item.signature)}
      style={{
        borderRadius: 'var(--wks-radius-lg)',
        border: `1px solid ${selected ? v.color : 'var(--wks-glass-border)'}`,
        boxShadow: selected
          ? `0 0 0 1px ${v.color}, 0 8px 24px var(--wks-shadow)`
          : '0 2px 10px var(--wks-shadow)',
        background: 'var(--wks-bg-surface)',
        overflow: 'hidden',
        transition: 'border-color 0.12s, box-shadow 0.12s',
      }}
    >
      {/* Header: kind badge + label + agent + relative time */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '9px 12px',
          borderBottom: '1px solid var(--wks-glass-border)',
        }}
      >
        <span
          style={{
            width: 17,
            height: 17,
            borderRadius: 'var(--wks-radius-sm)',
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: `color-mix(in srgb, ${v.color} 18%, transparent)`,
            border: `1px solid color-mix(in srgb, ${v.color} 45%, transparent)`,
            color: v.color,
            fontSize: '0.66rem',
            fontWeight: 700,
            lineHeight: 1,
          }}
        >
          {v.glyph}
        </span>
        <span
          style={{
            fontSize: '0.72rem',
            fontWeight: 700,
            color: v.color,
            textTransform: 'uppercase',
            letterSpacing: '0.04em',
          }}
        >
          {v.label}
        </span>
        <span
          style={{
            fontSize: '0.78rem',
            fontWeight: 600,
            color: 'var(--wks-text-primary)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {item.agentName}
        </span>
        <span
          style={{
            marginLeft: 'auto',
            fontSize: '0.69rem',
            color: 'var(--wks-text-faint)',
            fontVariantNumeric: 'tabular-nums',
            flexShrink: 0,
          }}
        >
          {relTime(item.createdAt)}
        </span>
      </div>

      {/* Body: the resolver, reused verbatim from ClaudePane */}
      <div style={{ padding: '4px 12px 10px' }}>
        {item.payload.type === 'approval' && (
          <ApprovalPrompt approval={item.payload.approval} onRespond={(r) => approve(item, r)} />
        )}
        {item.payload.type === 'question' && (
          <QuestionPicker questions={item.payload.questions} onAnswer={(p) => answer(item, p)} />
        )}
        {item.payload.type === 'summary' && (
          <div
            style={{
              fontSize: '0.78rem',
              color: 'var(--wks-text-secondary)',
              lineHeight: 1.5,
              padding: '6px 2px',
            }}
          >
            {item.detail}
          </div>
        )}
      </div>

      {/* Footer: triage actions */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '8px 12px',
          borderTop: '1px solid var(--wks-glass-border)',
          background: 'var(--wks-glass-strong)',
        }}
      >
        <CardBtn label="Open" hint="o" onClick={() => openAgent(item.agentId)} />
        {canReview && (
          <CardBtn
            label="Review"
            onClick={() => {
              reviewFile(item.cwd, undefined, item.agentId);
              dismiss(item.signature);
            }}
          />
        )}
        {canRespawn && (
          <CardBtn
            label="Respawn"
            onClick={() => {
              respawn(item.agentId);
              openAgent(item.agentId);
            }}
          />
        )}
        <CardBtn label="Snooze" hint="s" onClick={() => snooze(item.signature, SNOOZE_MINUTES)} />
        <CardBtn label="Dismiss" hint="e" onClick={() => dismiss(item.signature)} />
        <span
          style={{
            marginLeft: 'auto',
            fontSize: '0.66rem',
            color: 'var(--wks-text-faint)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            maxWidth: 180,
          }}
          title={item.cwd}
        >
          {basename(item.cwd)}
        </span>
      </div>
    </div>
  );
};

const CardBtn: React.FC<{ label: string; hint?: string; onClick: () => void }> = ({
  label,
  hint,
  onClick,
}) => (
  <button
    onClick={(e) => {
      e.stopPropagation();
      onClick();
    }}
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      fontSize: '0.7rem',
      fontFamily: 'inherit',
      fontWeight: 600,
      padding: '3px 9px',
      borderRadius: 'var(--wks-radius-sm)',
      cursor: 'pointer',
      border: '1px solid var(--wks-glass-border)',
      background: 'var(--wks-bg-surface)',
      color: 'var(--wks-text-secondary)',
    }}
  >
    {label}
    {hint && (
      <kbd
        style={{
          fontSize: '0.62rem',
          color: 'var(--wks-text-faint)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 3,
          padding: '0 3px',
          fontFamily: 'var(--wks-font-mono)',
        }}
      >
        {hint}
      </kbd>
    )}
  </button>
);
