export type HtmlCardDiffResult =
  { ok: true; path: string; before: string; after: string } | { ok: false; error: string };
