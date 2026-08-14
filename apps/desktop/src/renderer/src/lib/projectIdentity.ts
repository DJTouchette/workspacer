/**
 * What a project looks like at a glance.
 *
 * Every agent card, tab and picker row belongs to a directory, and until now
 * they all looked identical — the fleet was a wall of same-shaped rows and the
 * only way to tell which repo a card belonged to was to read its path. This
 * resolves a directory to a small visual identity so that stops being true.
 *
 * The important property: it works with NO configuration. An unconfigured
 * project still gets stable initials and a stable colour derived from its own
 * path, so the fleet is legible the first time you look at it. `config.projects`
 * only records the parts a human chose to override.
 */
import type { ProjectIdentity } from '../hooks/useConfig';
import { resolveProjectKey } from './projectKey';

/** What to actually draw for a directory. */
export interface ResolvedProject {
  /** Display name — the configured label, else the directory's basename. */
  label: string;
  /** One or two characters, when there is no icon or favicon to draw. */
  initials: string;
  /** The badge tint. */
  color: string;
  /** A configured emoji / short string, if any. */
  icon?: string;
  /** A configured http(s) icon URL, if any. */
  favicon?: string;
}

/**
 * The palette derived marks are drawn from. Deliberately a fixed list rather
 * than a free hue rotation: these sit beside status colours (success/warning/
 * error/busy) that carry meaning, so a derived tint must never land close
 * enough to be mistaken for one. Hues are spread and kept mid-saturation so
 * they read as identity, not state, in both light and dark themes.
 */
const PALETTE = [
  '#6b8afd', // indigo
  '#c084fc', // violet
  '#f472b6', // pink
  '#fb923c', // orange — distinct from --wks-warning's yellow
  '#2dd4bf', // teal
  '#38bdf8', // sky
  '#a3a3f5', // periwinkle
  '#e879a6', // rose
];

/** A stable 32-bit hash of a string (FNV-1a). Same path → same colour, on every
 *  machine and across restarts, which is the whole point of deriving it. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** The last path segment — the name a human calls the project. */
export function basenameOf(dir: string): string {
  const parts = String(dir || '')
    .replace(/\\/g, '/')
    .replace(/\/+$/, '')
    .split('/');
  return parts[parts.length - 1] || '';
}

/**
 * One or two characters for a name. A hyphenated or underscored name gives up
 * its word initials (`work-spacer` → `WS`), which distinguishes sibling repos
 * that share a prefix far better than the first two letters would
 * (`api-gateway` and `api-worker` are `AG` and `AW`, not both `AP`).
 */
export function initialsOf(name: string): string {
  const words = String(name || '')
    .split(/[\s._-]+/)
    .filter(Boolean);
  if (!words.length) return '?';
  if (words.length === 1) {
    const w = words[0];
    // A camelCase single word still has a second word inside it.
    const camel = w.match(/^([a-z]+)([A-Z][a-z]*)/);
    if (camel) return (camel[1][0] + camel[2][0]).toUpperCase();
    return w.slice(0, 2).toUpperCase();
  }
  return (words[0][0] + words[1][0]).toUpperCase();
}

/**
 * Resolve a directory to what should be drawn for it. `projects` is
 * `config.projects`; a missing entry is normal and fully supported.
 */
export function resolveProject(
  dir: string | undefined,
  projects?: Record<string, ProjectIdentity>,
): ResolvedProject | null {
  if (!dir) return null;
  const key = resolveProjectKey(projects, dir);
  const entry = projects?.[key] ?? {};
  const base = basenameOf(dir);
  const label = (entry.label || '').trim() || base;
  return {
    label,
    // Initials follow the LABEL, so renaming a project renames its mark too.
    initials: initialsOf(label),
    // Derived from the KEY, not the label: renaming a project should not
    // re-colour it out from under you.
    color: (entry.color || '').trim() || PALETTE[hash(key) % PALETTE.length],
    icon: (entry.icon || '').trim() || undefined,
    favicon: (entry.favicon || '').trim() || undefined,
  };
}
