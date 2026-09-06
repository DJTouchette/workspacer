/**
 * The `wks-html-card` response-card envelope — the one place its fence tag,
 * version, limits and shape are defined.
 *
 * A card rides as an ORDINARY fenced code block inside assistant text, exactly
 * the way `wks-result` does (shared/structuredResult.ts). Nothing about the
 * wire changed: no provider content-kind, no bus method, no persistence field.
 * A client that has never heard of this feature sees an unknown language tag on
 * a code fence and renders the JSON as text — which is why `fallback` is
 * REQUIRED and must be a genuinely useful sentence, not a label.
 *
 * Why a sibling of structuredResult rather than a reuse of it: that module runs
 * in MAIN over host-constructed fleet-wake text, keeps the LAST tagged block
 * (an earlier one is a draft) and is gated behind a per-dispatch `resultSchema`.
 * A response card is per-message, renderer-side, always-available, and EVERY
 * card in a message renders — a reply that draws two tables is two cards, not
 * one card and one discarded draft. Same algorithm family, different rules; the
 * two must not be collapsed.
 *
 * This module is deliberately dependency-free and never throws, so both the
 * renderer (which renders the card) and main (which generates the skill text
 * that teaches the schema) can import it and quote the same numbers.
 */

/** The fence tag. `\`\`\`wks-html-card` and nothing else. */
export const HTML_CARD_FENCE = 'wks-html-card';

/** The only envelope version this build renders. An unknown version is a
 *  fallback, never a best-effort parse — the plugin-manifest rule (an
 *  apiVersion mismatch is fatal at load, not a warning) applies here for the
 *  same reason: a card whose meaning drifted is worse than a card that didn't
 *  render. */
export const HTML_CARD_VERSION = 1;

/**
 * Hard cap on the fenced block's UTF-8 size. A card is a summary of an answer,
 * not a document: 64 KiB is roughly 40 screens of dense table markup and still
 * cheap to hand a fresh iframe. Oversized is a REFUSAL, not a truncation —
 * truncated markup is exactly the "partially executed" state the contract
 * forbids.
 */
export const HTML_CARD_MAX_BYTES = 64 * 1024;

/** At most this many host actions per card. */
export const HTML_CARD_MAX_ACTIONS = 8;

/** Field caps. Titles and labels are chrome, so they are short by construction;
 *  a long one is a layout attack, not a description. */
export const HTML_CARD_MAX_TITLE = 120;
export const HTML_CARD_MAX_LABEL = 48;
export const HTML_CARD_MAX_FALLBACK = 4000;
/** A composer prefill is a message the user is about to read before sending. */
export const HTML_CARD_MAX_PREFILL = 4000;
/** Model-authored stylesheet text, counted inside the envelope cap as well. */
export const HTML_CARD_MAX_CSS = 16 * 1024;

/** The three host actions a card may declare. Every one of them is either
 *  read-only navigation or a composer PREFILL; there is deliberately no
 *  dispatch, config, permission or filesystem action, and no auto-send. */
export type HtmlCardActionKind = 'open_worker' | 'view_diff' | 'fill_composer';

export const HTML_CARD_ACTION_KINDS: readonly HtmlCardActionKind[] = [
  'open_worker',
  'view_diff',
  'fill_composer',
];

export interface HtmlCardOpenWorker {
  kind: 'open_worker';
  label: string;
  /** A session the host RE-VALIDATES at click time. It is a target reference,
   *  never a statement of what the card is allowed to do. */
  sessionId: string;
}

export interface HtmlCardViewDiff {
  kind: 'view_diff';
  label: string;
  /** Absolute, or relative to the OWNING pane's cwd (supplied by the host, not
   *  by the card). Containment is re-checked against that cwd at click time. */
  path: string;
}

export interface HtmlCardFillComposer {
  kind: 'fill_composer';
  label: string;
  /** Text placed in the composer for the user to read and send themselves. */
  text: string;
}

export type HtmlCardAction = HtmlCardOpenWorker | HtmlCardViewDiff | HtmlCardFillComposer;

export interface HtmlCardEnvelope {
  v: typeof HTML_CARD_VERSION;
  title: string;
  /** An inner FRAGMENT. Never a document: the host owns `<html>`, `<head>`, the
   *  CSP meta and the theme, and concatenates this after all of them. */
  bodyHtml: string;
  css?: string;
  actions: HtmlCardAction[];
  /** Required. What a reader gets when the card cannot render — an old client,
   *  a plain-text transport, a screen reader, or a refusal below. */
  fallback: string;
}

export type HtmlCardParse =
  | { ok: true; card: HtmlCardEnvelope }
  | {
      ok: false;
      /** Shown in the refusal chrome, so it says what happened in one clause. */
      reason: string;
      /** Salvaged when the JSON parsed and carried usable fallback prose —
       *  an unknown VERSION should still read as an answer. */
      fallback?: string;
    };

/** UTF-8 byte length without allocating a Buffer (this runs in the renderer). */
export function utf8Bytes(text: string): number {
  // TextEncoder is present in every runtime this ships to (Chromium, Node 18+).
  return new TextEncoder().encode(text).length;
}

const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const str = (value: unknown): string | null => (typeof value === 'string' ? value : null);

/**
 * Validate one declared action. An unrecognized or malformed action returns
 * null; the caller refuses the whole envelope rather than partially interpreting it.
 */
function parseAction(raw: unknown): HtmlCardAction | null {
  if (!isPlainObject(raw)) return null;
  const kind = str(raw.kind);
  const label = str(raw.label)?.trim();
  if (!kind || !label || label.length > HTML_CARD_MAX_LABEL) return null;
  switch (kind) {
    case 'open_worker': {
      const sessionId = str(raw.sessionId)?.trim();
      if (!sessionId || sessionId.length > 200) return null;
      return { kind, label, sessionId };
    }
    case 'view_diff': {
      const path = str(raw.path)?.trim();
      if (!path || path.length > 4096) return null;
      return { kind, label, path };
    }
    case 'fill_composer': {
      const text = str(raw.text);
      if (!text || !text.trim() || text.length > HTML_CARD_MAX_PREFILL) return null;
      return { kind, label, text };
    }
    default:
      return null;
  }
}

/**
 * Parse the body of a CLOSED `wks-html-card` fence. Never throws: every
 * failure comes back as `{ok:false, reason}` so the caller renders readable
 * prose instead of an empty space or a thrown render.
 *
 * Order matters. Size is checked before JSON.parse (a 5 MB block should not be
 * parsed to be rejected), and the version is checked before any field is read,
 * so a v2 envelope is never half-interpreted through v1 eyes.
 */
export function parseHtmlCard(raw: string): HtmlCardParse {
  const text = raw.trim();
  if (!text) return { ok: false, reason: 'the card block was empty' };
  const bytes = utf8Bytes(raw);
  if (bytes > HTML_CARD_MAX_BYTES) {
    return {
      ok: false,
      reason: `the card is ${Math.round(bytes / 1024)} KB, over the ${
        HTML_CARD_MAX_BYTES / 1024
      } KB limit`,
    };
  }

  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { ok: false, reason: 'the card block is not valid JSON' };
  }
  if (!isPlainObject(value)) return { ok: false, reason: 'the card block is not a JSON object' };

  // Salvage the fallback prose FIRST, so every refusal below can still show the
  // author's own words rather than a bare error.
  const salvaged = str(value.fallback)?.trim();
  const fallback = salvaged && salvaged.length <= HTML_CARD_MAX_FALLBACK ? salvaged : undefined;

  if (value.v !== HTML_CARD_VERSION) {
    return {
      ok: false,
      reason:
        typeof value.v === 'number'
          ? `this card is version ${value.v}; this build renders version ${HTML_CARD_VERSION}`
          : 'the card is missing its version',
      fallback,
    };
  }

  const bodyHtml = str(value.bodyHtml);
  if (!bodyHtml || !bodyHtml.trim()) {
    return { ok: false, reason: 'the card has no bodyHtml', fallback };
  }
  if (!fallback) {
    return {
      ok: false,
      reason: 'the card has no fallback text, which every card must carry',
    };
  }
  const title = (str(value.title) ?? '').trim();
  if (title.length > HTML_CARD_MAX_TITLE)
    return { ok: false, reason: 'the card title is too long', fallback };
  if (value.css !== undefined && typeof value.css !== 'string')
    return { ok: false, reason: 'css must be text', fallback };
  if (value.actions !== undefined && !Array.isArray(value.actions))
    return { ok: false, reason: 'actions must be an array', fallback };
  if (!title) return { ok: false, reason: 'the card has no title', fallback };

  const cssRaw = str(value.css) ?? '';
  if (utf8Bytes(cssRaw) > HTML_CARD_MAX_CSS) {
    return { ok: false, reason: 'the card stylesheet is too large', fallback };
  }

  const declared = Array.isArray(value.actions) ? value.actions : [];
  if (declared.length > HTML_CARD_MAX_ACTIONS) {
    return {
      ok: false,
      reason: `the card declares ${declared.length} actions, over the limit of ${HTML_CARD_MAX_ACTIONS}`,
      fallback,
    };
  }
  const actions = declared.map(parseAction);
  if (actions.some((action) => action === null)) {
    return { ok: false, reason: 'the card has an invalid or unsupported action', fallback };
  }

  return {
    ok: true,
    card: {
      v: HTML_CARD_VERSION,
      title,
      bodyHtml,
      css: cssRaw || undefined,
      actions: actions as HtmlCardAction[],
      fallback,
    },
  };
}
