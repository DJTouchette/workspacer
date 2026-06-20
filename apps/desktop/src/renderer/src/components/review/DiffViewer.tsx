/**
 * Virtualized unified diff renderer with shiki syntax highlighting.
 *
 * Speed model, in order of importance:
 *  1. Rows are virtualized (@tanstack/react-virtual, fixed row height) — DOM
 *     size is ~viewport, never diff size.
 *  2. Plain colored text paints immediately; syntax tokens arrive async
 *     per-hunk and swap in, so first paint never waits on a grammar.
 *  3. Each hunk is tokenized as two small virtual documents (old side / new
 *     side) so grammar state survives multi-line constructs but cost scales
 *     with the diff, not the whole file.
 *  4. Diffs past HIGHLIGHT_LINE_BUDGET lines skip highlighting entirely.
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { claudeColors as colors } from '../claude-shared';
import type { DiffLine, ParsedDiff } from '../../lib/diff/parseDiff';
import {
  langForPath,
  tokenize,
  MAX_HIGHLIGHT_LINE_LENGTH,
  type TokenSpan,
} from '../../lib/diff/highlight';
import { ensureReviewStyles } from './reviewStyles';

/** Above this many diff lines, skip syntax highlighting (still virtualized). */
const HIGHLIGHT_LINE_BUDGET = 10_000;

const ROW_H = 20;
const FONT_SIZE = '0.72rem';

type Row =
  | { type: 'hunk'; header: string }
  | { type: 'line'; line: DiffLine; key: string };

// Row backgrounds. The gutter needs the *opaque* mix (code scrolls beneath it
// horizontally); the code area can use the transparent tint.
const ADD_TINT = 'color-mix(in srgb, var(--wks-success) 11%, transparent)';
const DEL_TINT = 'color-mix(in srgb, var(--wks-error) 11%, transparent)';
const ADD_GUTTER = 'color-mix(in srgb, var(--wks-success) 11%, var(--wks-claude-bg))';
const DEL_GUTTER = 'color-mix(in srgb, var(--wks-error) 11%, var(--wks-claude-bg))';
const ADD_EMPH = 'color-mix(in srgb, var(--wks-success) 32%, transparent)';
const DEL_EMPH = 'color-mix(in srgb, var(--wks-error) 32%, transparent)';
const HUNK_TINT = 'color-mix(in srgb, var(--wks-accent-text) 8%, transparent)';

interface RenderSpan extends TokenSpan {
  emph?: boolean;
}

/** Split token spans at the emphasis boundaries so the changed chars get the
 * stronger background. */
function withEmphasis(spans: TokenSpan[], emph: [number, number] | undefined): RenderSpan[] {
  if (!emph) return spans;
  const [start, end] = emph;
  const out: RenderSpan[] = [];
  let offset = 0;
  for (const span of spans) {
    const spanEnd = offset + span.text.length;
    const cuts = [offset, Math.max(offset, Math.min(start, spanEnd)), Math.max(offset, Math.min(end, spanEnd)), spanEnd];
    for (let i = 0; i < 3; i++) {
      const from = cuts[i];
      const to = cuts[i + 1];
      if (to <= from) continue;
      out.push({
        text: span.text.slice(from - offset, to - offset),
        color: span.color,
        emph: from >= start && to <= end,
      });
    }
    offset = spanEnd;
  }
  return out;
}

export interface DiffViewerProps {
  diff: ParsedDiff;
  /** File path — drives grammar selection. */
  path: string;
}

const DiffViewer: React.FC<DiffViewerProps> = ({ diff, path }) => {
  ensureReviewStyles();
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    diff.hunks.forEach((hunk, h) => {
      out.push({ type: 'hunk', header: hunk.header });
      hunk.lines.forEach((line, i) => out.push({ type: 'line', line, key: `${h}:${i}` }));
    });
    return out;
  }, [diff]);

  const totalLines = rows.length;

  // Token map: "hunkIdx:lineIdx" → spans. Stored in a ref and exposed through
  // a version counter so a 10k-line diff doesn't rebuild a Map per hunk.
  const tokensRef = useRef<Map<string, TokenSpan[]>>(new Map());
  const [, setTokensVersion] = useState(0);

  useEffect(() => {
    tokensRef.current = new Map();
    setTokensVersion((v) => v + 1);

    const lang = langForPath(path);
    if (!lang || totalLines > HIGHLIGHT_LINE_BUDGET) return;

    let cancelled = false;
    void (async () => {
      for (let h = 0; h < diff.hunks.length; h++) {
        const hunk = diff.hunks[h];
        // Two virtual documents per hunk: grammar state flows across lines
        // within a side, and context lines borrow the new side's tokens.
        const oldIdx: number[] = [];
        const newIdx: number[] = [];
        const oldDoc: string[] = [];
        const newDoc: string[] = [];
        hunk.lines.forEach((line, i) => {
          const tooLong = line.text.length > MAX_HIGHLIGHT_LINE_LENGTH;
          if (line.kind !== 'add') {
            oldIdx[i] = tooLong ? -1 : oldDoc.push(line.text) - 1;
            if (tooLong) oldDoc.push('');
          }
          if (line.kind !== 'del') {
            newIdx[i] = tooLong ? -1 : newDoc.push(line.text) - 1;
            if (tooLong) newDoc.push('');
          }
        });

        const [oldTokens, newTokens] = await Promise.all([
          tokenize(oldDoc.join('\n'), lang),
          tokenize(newDoc.join('\n'), lang),
        ]);
        if (cancelled) return;
        if (!oldTokens && !newTokens) return; // grammar unavailable — stay plain

        hunk.lines.forEach((line, i) => {
          // -1 is the long-line sentinel: render plain.
          const idx = line.kind === 'del' ? oldIdx[i] : newIdx[i];
          if (idx === -1) return;
          const spans = line.kind === 'del' ? oldTokens?.[idx] : newTokens?.[idx];
          if (spans) tokensRef.current.set(`${h}:${i}`, spans);
        });
        setTokensVersion((v) => v + 1);
        // Yield between hunks so a long highlight pass never blocks scrolling.
        await new Promise((r) => setTimeout(r, 0));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [diff, path, totalLines]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 24,
  });

  // Gutter width scales with the largest line number.
  const maxLineNo = useMemo(() => {
    let max = 1;
    for (const hunk of diff.hunks) {
      for (const line of hunk.lines) {
        if (line.oldNo) max = Math.max(max, line.oldNo);
        if (line.newNo) max = Math.max(max, line.newNo);
      }
    }
    return max;
  }, [diff]);
  const numCh = Math.max(2, String(maxLineNo).length);
  const gutterWidth = `calc(${numCh * 2}ch + 34px)`;

  const renderLine = (row: Extract<Row, { type: 'line' }>): React.ReactNode => {
    const { line, key } = row;
    const tint = line.kind === 'add' ? ADD_TINT : line.kind === 'del' ? DEL_TINT : 'transparent';
    const gutterBg =
      line.kind === 'add' ? ADD_GUTTER : line.kind === 'del' ? DEL_GUTTER : 'var(--wks-claude-bg)';
    const emphBg = line.kind === 'add' ? ADD_EMPH : DEL_EMPH;
    const marker = line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ' ';
    const markerColor =
      line.kind === 'add' ? colors.success : line.kind === 'del' ? colors.error : colors.muted;

    const tokens = tokensRef.current.get(key);
    const spans: RenderSpan[] = tokens
      ? withEmphasis(tokens, line.emph)
      : withEmphasis([{ text: line.text }], line.emph);

    return (
      <>
        <span
          style={{
            position: 'sticky',
            left: 0,
            display: 'inline-flex',
            alignItems: 'center',
            width: gutterWidth,
            flexShrink: 0,
            background: gutterBg,
            color: colors.muted,
            fontSize: '0.64rem',
            fontVariantNumeric: 'tabular-nums',
            userSelect: 'none',
            borderRight: `1px solid ${colors.borderSubtle}`,
            boxSizing: 'border-box',
            zIndex: 1,
          }}
        >
          <span style={{ width: `${numCh}ch`, textAlign: 'right', paddingRight: 6 }}>
            {line.oldNo ?? ''}
          </span>
          <span style={{ width: `${numCh}ch`, textAlign: 'right', paddingRight: 6 }}>
            {line.newNo ?? ''}
          </span>
          <span style={{ color: markerColor, fontWeight: 700, paddingLeft: 2 }}>{marker}</span>
        </span>
        <span style={{ background: tint, flex: 1, paddingLeft: 8, whiteSpace: 'pre' }}>
          {spans.map((s, i) => (
            <span
              key={i}
              style={{
                color: s.color ?? colors.text,
                background: s.emph ? emphBg : undefined,
                borderRadius: s.emph ? 2 : undefined,
              }}
            >
              {s.text}
            </span>
          ))}
          {line.noNewline && (
            <span style={{ color: colors.muted, userSelect: 'none' }}> ⌐ no newline</span>
          )}
        </span>
      </>
    );
  };

  return (
    <div
      ref={scrollRef}
      className="wks-review-scroll"
      style={{
        height: '100%',
        overflow: 'auto',
        fontFamily: 'var(--wks-font-mono, monospace)',
        fontSize: FONT_SIZE,
        lineHeight: `${ROW_H}px`,
        contain: 'strict',
      }}
    >
      <div
        style={{
          height: virtualizer.getTotalSize(),
          minWidth: `calc(${Math.min(diff.maxLineLength, 500)}ch + ${numCh * 2}ch + 60px)`,
          width: '100%',
          position: 'relative',
        }}
      >
        {virtualizer.getVirtualItems().map((v) => {
          const row = rows[v.index];
          return (
            <div
              key={v.key}
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                width: '100%',
                height: ROW_H,
                transform: `translateY(${v.start}px)`,
                display: 'flex',
                alignItems: 'stretch',
                boxSizing: 'border-box',
                background: row.type === 'hunk' ? HUNK_TINT : undefined,
              }}
            >
              {row.type === 'hunk' ? (
                // Sticky so the header text stays readable while the wide
                // code area scrolls horizontally beneath it.
                <span
                  style={{
                    position: 'sticky',
                    left: 0,
                    maxWidth: '100%',
                    width: 'max-content',
                    color: colors.accent,
                    paddingLeft: 10,
                    paddingRight: 10,
                    whiteSpace: 'pre',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    fontSize: '0.66rem',
                    userSelect: 'none',
                  }}
                >
                  {row.header}
                </span>
              ) : (
                renderLine(row)
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};

export default DiffViewer;
