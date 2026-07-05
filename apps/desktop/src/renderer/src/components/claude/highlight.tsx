/**
 * Shared syntax-highlighting helpers for the Claude pane (code blocks, Read
 * previews, diffs). Thin wrapper over the lazy shiki tokenizer in
 * lib/diff/highlight: tokenize asynchronously, render per-line token spans, and
 * fall back to plain text while a grammar loads or when the language is unknown
 * — so highlighting only ever upgrades the view, never blocks or empties it.
 */
import React, { useEffect, useState } from 'react';
import { tokenize, MAX_HIGHLIGHT_LINE_LENGTH, type TokenSpan } from '../../lib/diff/highlight';

/**
 * Tokenize `code` for `lang`, returning one TokenSpan[] per line once ready, or
 * null while loading / when the language is unknown (caller renders plain text).
 */
export function useHighlight(code: string, lang: string | null): TokenSpan[][] | null {
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  useEffect(() => {
    if (!lang || !code) {
      setTokens(null);
      return;
    }
    let cancelled = false;
    tokenize(code, lang)
      .then((result) => {
        if (!cancelled) setTokens(result);
      })
      .catch(() => {
        if (!cancelled) setTokens(null);
      });
    return () => {
      cancelled = true;
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
