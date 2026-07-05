/**
 * The rich body of a Fleet Deck AgentCard: the agent's last message rendered
 * as real markdown (not a raw clamped string), a chip row for the active +
 * recent tool calls, and a one-line changed-files summary.
 *
 * Memoized with a cheap comparator — AgentCard itself re-renders on every
 * attention-context tick, so this subtree bails out unless its own inputs
 * (message text, tool ids/statuses, file counts) actually changed.
 */

import React, { useEffect, useRef, useState } from 'react';
import type { ToolCall } from '../types/claudeSession';
import type { PlanProgress } from '../lib/sessionStats';
import { formatToolSummary, ensureKeyframes, claudeColors } from './claude-shared';
import { Markdown } from './markdown';

/** Cap the source text before parsing — bounds markdown parse + DOM cost
 *  across a deck of cards, and keeps the LRU cache hot (deterministic cut). */
export function excerptForCard(text: string): string {
  const MAX_LINES = 12;
  const MAX_CHARS = 700;
  let out = text.length > MAX_CHARS ? text.slice(0, MAX_CHARS) : text;
  const lines = out.split('\n');
  if (lines.length > MAX_LINES) out = lines.slice(0, MAX_LINES).join('\n');
  return out;
}

/** True while the element's content overflows its max-height box. */
function useIsOverflowing(ref: React.RefObject<HTMLDivElement | null>, deps: unknown[]): boolean {
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const check = () => setOverflowing(el.scrollHeight > el.clientHeight + 1);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return overflowing;
}

const chipBase: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 5,
  padding: '1px 8px',
  borderRadius: 9,
  fontSize: '0.62rem',
  fontFamily: 'var(--claude-mono-font, monospace)',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  flexShrink: 1,
  minWidth: 0,
};

const ToolChipRow: React.FC<{ active?: ToolCall; recent: ToolCall[] }> = ({ active, recent }) => {
  if (!active && recent.length === 0) return null;
  return (
    <div
      style={{
        display: 'flex',
        gap: 6,
        padding: '0 14px 7px',
        overflow: 'hidden',
        flexWrap: 'nowrap',
      }}
    >
      {active && (
        <span
          title={formatToolSummary(active).call}
          style={{
            ...chipBase,
            maxWidth: '48%',
            color: 'var(--wks-accent, #4a9eff)',
            border: '1px solid color-mix(in srgb, var(--wks-accent, #4a9eff) 45%, transparent)',
            background: 'color-mix(in srgb, var(--wks-accent, #4a9eff) 10%, transparent)',
          }}
        >
          <span
            style={{
              display: 'inline-block',
              width: 9,
              height: 9,
              border: '1.5px solid currentColor',
              borderTopColor: 'transparent',
              borderRadius: '50%',
              animation: 'claudeSpinner 0.8s linear infinite',
              flexShrink: 0,
            }}
          />
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {formatToolSummary(active).call}
          </span>
        </span>
      )}
      {recent.map((tc) => (
        <span
          key={tc.id}
          title={formatToolSummary(tc).call}
          style={{
            ...chipBase,
            color: 'var(--wks-text-faint)',
            border: '1px solid var(--wks-border-subtle, #2a2a2a)',
            background: 'transparent',
          }}
        >
          <span
            style={{
              color: tc.status === 'failed' ? claudeColors.error : claudeColors.success,
              flexShrink: 0,
            }}
          >
            {tc.status === 'failed' ? '✗' : '✓'}
          </span>
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {formatToolSummary(tc).call}
          </span>
        </span>
      ))}
    </div>
  );
};

export interface AgentCardBodyProps {
  /** Last assistant message (raw markdown source, pre-excerpt). */
  text: string;
  /** Shown when there's no message yet ("No activity yet" / "Stopped…"). */
  fallback: string;
  active?: ToolCall;
  recent: ToolCall[];
  fileStats: { files: number; added: number; removed: number };
  /** Plan progress (null when the agent has no plan) — renders an N/M chip. */
  plan?: PlanProgress | null;
  /** True when an action zone (approval/question) is showing — tighter body. */
  compact: boolean;
}

const AgentCardBodyInner: React.FC<AgentCardBodyProps> = ({
  text,
  fallback,
  active,
  recent,
  fileStats,
  plan,
  compact,
}) => {
  useEffect(() => {
    ensureKeyframes();
  }, []);
  const excerpt = text ? excerptForCard(text) : '';
  const bodyRef = useRef<HTMLDivElement>(null);
  const maxHeight = compact ? 76 : 152;
  const overflowing = useIsOverflowing(bodyRef, [excerpt, maxHeight]);

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
      <ToolChipRow active={active} recent={recent} />
      <div
        ref={bodyRef}
        style={{
          padding: '0 14px',
          fontSize: '0.78rem',
          color: 'var(--wks-text-secondary)',
          lineHeight: 1.5,
          maxHeight,
          overflow: 'hidden',
          // Fade the clipped edge out instead of hard-cutting a line in half.
          ...(overflowing
            ? {
                WebkitMaskImage: 'linear-gradient(to bottom, black 72%, transparent 100%)',
                maskImage: 'linear-gradient(to bottom, black 72%, transparent 100%)',
              }
            : null),
        }}
      >
        {excerpt ? (
          <Markdown text={excerpt} />
        ) : (
          <span style={{ color: 'var(--wks-text-faint)' }}>{fallback}</span>
        )}
      </div>
      {(fileStats.files > 0 || plan) && (
        <div
          style={{
            padding: '6px 14px 0',
            fontSize: '0.64rem',
            fontFamily: 'var(--claude-mono-font, monospace)',
            fontVariantNumeric: 'tabular-nums',
            color: 'var(--wks-text-faint)',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          {plan && (
            <span
              title={plan.active?.activeForm ?? plan.active?.content ?? 'Plan progress'}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 4,
                color:
                  plan.done >= plan.total
                    ? 'var(--wks-success, #3fb950)'
                    : 'var(--wks-accent, #4a9eff)',
              }}
            >
              <span style={{ color: 'var(--wks-text-faint)' }}>plan</span>
              {plan.done}/{plan.total}
            </span>
          )}
          {fileStats.files > 0 && (
            <span style={{ display: 'inline-flex', gap: 6 }}>
              <span>
                ~ {fileStats.files} file{fileStats.files !== 1 ? 's' : ''}
              </span>
              {fileStats.added > 0 && (
                <span style={{ color: 'var(--wks-success, #3fb950)' }}>+{fileStats.added}</span>
              )}
              {fileStats.removed > 0 && (
                <span style={{ color: 'var(--wks-error, #f87171)' }}>−{fileStats.removed}</span>
              )}
            </span>
          )}
        </div>
      )}
    </div>
  );
};

const toolKey = (tc?: ToolCall) => (tc ? `${tc.id}:${tc.status}` : '');

function areEqual(prev: AgentCardBodyProps, next: AgentCardBodyProps): boolean {
  return (
    prev.text === next.text &&
    prev.fallback === next.fallback &&
    prev.compact === next.compact &&
    toolKey(prev.active) === toolKey(next.active) &&
    prev.recent.map(toolKey).join() === next.recent.map(toolKey).join() &&
    prev.fileStats.files === next.fileStats.files &&
    prev.fileStats.added === next.fileStats.added &&
    prev.fileStats.removed === next.fileStats.removed &&
    planKey(prev.plan) === planKey(next.plan)
  );
}

/** Stable identity for the plan prop: done/total + the active step's text. */
function planKey(p?: PlanProgress | null): string {
  if (!p) return '';
  return `${p.done}/${p.total}:${p.active?.content ?? ''}`;
}

export const AgentCardBody = React.memo(AgentCardBodyInner, areEqual);
export default AgentCardBody;
