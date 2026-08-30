import { useEffect } from 'react';
import { CHANGELOG } from '../lib/changelog.generated';
import { postNotification } from '../lib/notificationBus';

/** What this install last acknowledged. Bare version, never the nightly stamp. */
const STORAGE_KEY = 'wks.lastSeenVersion';

/** A nightly is `X.Y.Z-nightly.<stamp>`; its notes are those of `X.Y.Z`. */
export function baseVersion(v: string): string {
  return v.split('-')[0];
}

/**
 * Longest "taste" that still reads as a notification. A CHANGELOG entry is a
 * paragraph, not a line — the 0.160.0 lead entry is over 1000 characters — and
 * the caller appends a "see Settings → Updates" tail after this.
 */
const TASTE_MAX = 220;

/**
 * Build the notice body: the section titles and how many entries each has, then
 * the first entry as a taste. Deliberately NOT the whole release — the center is
 * a list of one-liners, and the full notes are one click away in Settings.
 */
export function whatsNewBody(version: string): string {
  const release = CHANGELOG.find((r) => r.version === version);
  if (!release) return '';
  const counts = release.sections
    .map((s) => `${s.items.length} ${s.title.toLowerCase()}`)
    .join(' · ');
  const first = release.sections[0]?.items[0] ?? '';
  // This body is PLAIN TEXT — an OS notification and an in-app toast, neither of
  // which renders markdown — so EVERY bold run has to be unwrapped, not just the
  // lead-in one an entry opens with. Entries routinely bold a term mid-sentence,
  // and a literal `**` in a notification reads as a rendering bug.
  const taste = clip(first.replace(/\*\*(.+?)\*\*/g, '$1').trim(), TASTE_MAX);
  return counts && taste ? `${counts} — ${taste}` : counts || taste;
}

/** Clip to `max`, at a word boundary when there is a plausible one. */
function clip(text: string, max: number): string {
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  const space = cut.lastIndexOf(' ');
  const kept = space > max * 0.6 ? cut.slice(0, space) : cut;
  return kept.replace(/[\s,;:.\u2014-]+$/, '') + '…';
}

/**
 * Post a one-time notice when the running version differs from the last one this
 * install acknowledged.
 *
 * Storage is localStorage, not config. This is a fact about THIS installation
 * ("has this copy shown you these notes"), it does not belong on a phone mirror,
 * and — the load-bearing half — a config write can fail: the bus config plane
 * went down for a whole session this month and every save was silently refused.
 * A notice whose "I have seen this" depends on that would re-fire on every
 * launch until the plane came back.
 *
 * A fresh install records the version and says nothing: "what's new" is only
 * meaningful against something you were running before.
 */
export function useWhatsNew(version: string): void {
  useEffect(() => {
    if (!version) return;
    const current = baseVersion(version);
    let seen: string | null = null;
    try {
      seen = localStorage.getItem(STORAGE_KEY);
    } catch {
      return; // no storage (private mode, locked-down webview): never nag
    }
    if (seen === current) return;
    try {
      localStorage.setItem(STORAGE_KEY, current);
    } catch {
      return; // could not record it, so do not post something that repeats
    }
    if (!seen) return; // first run of a fresh install
    const body = whatsNewBody(current);
    postNotification({
      key: `whats-new:${current}`,
      level: 'info',
      source: 'Workspacer',
      title: `Updated to v${current}`,
      body: body ? `${body} — see Settings → Updates for the full notes.` : undefined,
    });
  }, [version]);
}
