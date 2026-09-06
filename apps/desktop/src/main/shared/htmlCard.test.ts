/**
 * The `wks-html-card` envelope refuses more often than it accepts, and every
 * refusal has to stay READABLE — the reader gets prose, never a blank space and
 * never a half-interpreted card.
 */
import { describe, it, expect } from 'vitest';
import {
  HTML_CARD_MAX_ACTIONS,
  HTML_CARD_MAX_BYTES,
  HTML_CARD_VERSION,
  parseHtmlCard,
} from './htmlCard';

const good = {
  v: HTML_CARD_VERSION,
  title: 'Three findings',
  bodyHtml: '<p>hi</p>',
  fallback: 'High: token never expires. Medium: no SameSite. Low: dead exports.',
};
const block = (o: unknown) => JSON.stringify(o);

describe('parseHtmlCard', () => {
  it('accepts a complete envelope and normalizes the optional fields', () => {
    const res = parseHtmlCard(block(good));
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.card.title).toBe('Three findings');
    expect(res.card.actions).toEqual([]);
    expect(res.card.css).toBeUndefined();
  });

  it('never throws, whatever it is handed', () => {
    for (const input of ['', '   ', 'not json', '{', '[]', 'null', '"a string"', '{"v":1']) {
      expect(() => parseHtmlCard(input)).not.toThrow();
      expect(parseHtmlCard(input).ok).toBe(false);
    }
  });

  it('refuses an unknown version WITHOUT reading any other field', () => {
    // The card is otherwise perfect. A v2 envelope's `bodyHtml` may mean
    // something else entirely, so a best-effort parse is the bug.
    const res = parseHtmlCard(block({ ...good, v: 2 }));
    expect(res.ok).toBe(false);
    if (res.ok) return;
    expect(res.reason).toContain('version 2');
    // …but the author's own prose still reaches the reader.
    expect(res.fallback).toBe(good.fallback);
  });

  it('refuses a missing version', () => {
    const res = parseHtmlCard(block({ ...good, v: undefined }));
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.reason).toContain('missing its version');
  });

  it('requires bodyHtml, title and fallback', () => {
    expect(parseHtmlCard(block({ ...good, bodyHtml: '' })).ok).toBe(false);
    expect(parseHtmlCard(block({ ...good, bodyHtml: '   ' })).ok).toBe(false);
    expect(parseHtmlCard(block({ ...good, title: '' })).ok).toBe(false);
    const noFallback = parseHtmlCard(block({ ...good, fallback: undefined }));
    expect(noFallback.ok).toBe(false);
    if (!noFallback.ok) expect(noFallback.reason).toContain('fallback');
  });

  it('refuses an oversized block before parsing it', () => {
    const huge = block({ ...good, bodyHtml: '<p>' + 'x'.repeat(HTML_CARD_MAX_BYTES) + '</p>' });
    const res = parseHtmlCard(huge);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.reason).toContain('over the');
  });

  it('measures the cap in UTF-8 BYTES, not characters', () => {
    // 4-byte astral characters: a cap counted in JS string length would let
    // roughly four times the intended payload through.
    const body = '🙂'.repeat(HTML_CARD_MAX_BYTES / 4);
    expect(body.length).toBeLessThan(HTML_CARD_MAX_BYTES);
    expect(parseHtmlCard(block({ ...good, bodyHtml: body })).ok).toBe(false);
  });

  it('refuses a card that declares more actions than the limit', () => {
    const actions = Array.from({ length: HTML_CARD_MAX_ACTIONS + 1 }, (_, i) => ({
      kind: 'view_diff',
      label: `f${i}`,
      path: `a${i}.ts`,
    }));
    const res = parseHtmlCard(block({ ...good, actions }));
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.reason).toContain('over the limit');
  });

  it('refuses the whole envelope for an unknown or malformed action', () => {
    const res = parseHtmlCard(
      block({
        ...good,
        actions: [
          { kind: 'spawn_agent', label: 'Go', prompt: 'rm -rf /' },
          { kind: 'view_diff', label: 'Diff', path: 'src/a.ts' },
          { kind: 'fill_composer', label: 'Reply' }, // no text
          { kind: 'open_worker', label: '' }, // no label
          'not an object',
        ],
      }),
    );
    expect(res.ok).toBe(false);
  });

  it('refuses a prefill longer than the cap rather than truncating it', () => {
    const res = parseHtmlCard(
      block({
        ...good,
        actions: [{ kind: 'fill_composer', label: 'Reply', text: 'x'.repeat(9000) }],
      }),
    );
    expect(res.ok).toBe(false);
  });

  it('does not salvage a fallback that is itself absurd', () => {
    const res = parseHtmlCard(block({ v: 7, fallback: 'y'.repeat(50_000) }));
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.fallback).toBeUndefined();
  });
});
