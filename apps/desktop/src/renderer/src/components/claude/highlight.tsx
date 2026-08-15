/**
 * Shared syntax-highlighting helpers for the Claude pane (code blocks, Read
 * previews, diffs). Thin wrapper over the lazy shiki tokenizer in
 * lib/diff/highlight: tokenize asynchronously, render per-line token spans, and
 * fall back to plain text while a grammar loads or when the language is unknown
 * — so highlighting only ever upgrades the view, never blocks or empties it.
 */
import React, { useEffect, useRef, useState } from 'react';
import { tokenize, MAX_HIGHLIGHT_LINE_LENGTH, type TokenSpan } from '../../lib/diff/highlight';

/**
 * How long the text must stop growing before an unterminated code fence is
 * tokenized again.
 *
 * A fence that is still streaming changes on every snapshot flush (~16ms), and
 * each pass re-tokenizes the WHOLE prefix — so the cost of a block is quadratic
 * in its length, paid synchronously on the main thread. The trailing edge is
 * what makes that bounded: while text is arriving, only settled lines are
 * highlighted, and the tail catches up once it stops.
 */
const GROWING_DEBOUNCE_MS = 200;

/**
 * Tokenize `code` for `lang`, returning one TokenSpan[] per line once ready, or
 * null while loading / when the language is unknown (caller renders plain text).
 *
 * Highlighting only ever upgrades the view, which is what makes the debounce
 * safe: the caller already renders plain text until tokens arrive, so a tail
 * that is 200ms behind reads as "not highlighted yet", exactly as the first
 * paint of any block does.
 */
export function useHighlight(code: string, lang: string | null): TokenSpan[][] | null {
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  // What we last tokenized for, so a growing string can be told from a settled
  // one without keeping the previous `code` in state (which would re-render).
  const lastCodeRef = useRef<string>('');

  useEffect(() => {
    if (!lang || !code) {
      setTokens(null);
      lastCodeRef.current = '';
      return;
    }
    let cancelled = false;

    // Still being appended to: highlight only up to the last newline, so the
    // partial final line doesn't invalidate the whole pass a frame later. The
    // slice is a prefix that recurs as more text lands, so the tokenizer's
    // result cache hits on it rather than redoing the block each time.
    const growing =
      code.startsWith(lastCodeRef.current) && code.length > lastCodeRef.current.length;
    const settledEnd = code.lastIndexOf('\n');
    const target = growing && settledEnd > 0 ? code.slice(0, settledEnd) : code;

    const run = () => {
      tokenize(target, lang)
        .then((result) => {
          if (cancelled) return;
          lastCodeRef.current = code;
          setTokens(result);
        })
        .catch(() => {
          if (!cancelled) setTokens(null);
        });
    };

    // A settled block (first render, or text that stopped changing) tokenizes
    // immediately — the debounce exists only to stop the streaming case from
    // queueing blocking passes behind each other.
    if (!growing) {
      run();
      return () => {
        cancelled = true;
      };
    }
    const timer = setTimeout(run, GROWING_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [code, lang]);

  return tokens;
}

/**
 * Render one line: tokenized spans when available, otherwise the raw line. Long
 * lines stay plain (matching the tokenizer's own cap) to avoid pathological
 * minified-blob rendering.
 */
export function renderLine(spans: TokenSpan[] | undefined, fallback: string): React.ReactNode {
  if (!spans || spans.length === 0 || fallback.length > MAX_HIGHLIGHT_LINE_LENGTH) return fallback;
  return spans.map((s, i) =>
    s.color ? (
      <span key={i} style={{ color: s.color }}>
        {s.text}
      </span>
    ) : (
      <React.Fragment key={i}>{s.text}</React.Fragment>
    ),
  );
}
