import React, { useState } from 'react';
import { resolveProject } from '../lib/projectIdentity';
import type { ProjectIdentity } from '../hooks/useConfig';

/**
 * The at-a-glance mark for a project directory.
 *
 * Draws, in order of preference: a configured favicon, a configured emoji or
 * short string, or initials derived from the project's name on a colour derived
 * from its path. The last of those is why this is worth having — an
 * unconfigured fleet is still legible, because every project already has a
 * stable mark before anyone opens a settings page.
 *
 * A remote favicon is a plain `<img>`, exactly as `PluginPaneIcon` renders a
 * plugin's declared icon: Chromium fetches and caches it, so there is no
 * fetch-and-cache pipeline here to keep correct. A URL that fails to load falls
 * back to the derived mark rather than leaving a hole — an icon host being down
 * must not cost you the ability to tell your projects apart.
 */

/**
 * How much bigger a favicon draws than the letter plate it replaces.
 *
 * Two letters are legible at 14px because they're glyphs at a size the type
 * scale is built for; a logo at 14px is a smudge. So the icon gets the extra
 * pixels and the plate doesn't.
 *
 * The BOX is scaled either way, and only the mark inside it changes size. A list
 * mixing configured and unconfigured projects (the sidebar, always) would
 * otherwise indent its labels by whether each project happened to have an icon.
 */
const FAVICON_SCALE = 1.45;
export const ProjectMark: React.FC<{
  cwd?: string;
  projects?: Record<string, ProjectIdentity>;
  size?: number;
  /** Show the project's name beside the mark. */
  withLabel?: boolean;
  title?: string;
}> = ({ cwd, projects, size = 14, withLabel = false, title }) => {
  const [broken, setBroken] = useState(false);
  const p = resolveProject(cwd, projects);
  if (!p) return null;

  const showFavicon = Boolean(p.iconSrc) && !broken;
  // An emoji carries its own colour and needs no tinted plate behind it; a
  // letter mark does. Detected by "not ASCII", which is what an emoji is here.
  const isEmoji = Boolean(p.icon) && !/^[\x20-\x7e]+$/.test(p.icon ?? '');
  const glyph = p.icon ?? p.initials;
  const box = Math.round(size * FAVICON_SCALE);

  const inner = showFavicon ? (
    <img
      src={p.iconSrc}
      width={box}
      height={box}
      onError={() => setBroken(true)}
      style={{ borderRadius: 'var(--wks-radius-sm)', objectFit: 'contain', flex: 'none' }}
      alt=""
    />
  ) : (
    <span
      aria-hidden
      style={{
        // An emoji is a picture like a favicon is, so it takes the full box; the
        // letter plate is type and stays on the size the caller asked for.
        width: isEmoji ? box : size,
        height: isEmoji ? box : size,
        flex: 'none',
        display: 'grid',
        placeItems: 'center',
        borderRadius: 'var(--wks-radius-sm)',
        // Border OR fill, never both (DESIGN_LANGUAGE §5): a tinted plate, no
        // outline. The tint is a color-mix of the project's own colour rather
        // than a hand-rolled rgba, so it composites correctly on light themes.
        background: isEmoji ? 'transparent' : `color-mix(in srgb, ${p.color} 22%, transparent)`,
        color: p.color,
        // Initials are two characters in a ~14px box; the scale bottoms out at
        // 0.6rem, so this is a deliberate exception rather than drift.
        fontSize: isEmoji ? box * 0.82 : Math.max(7, size * 0.46),
        fontWeight: 700,
        lineHeight: 1,
        letterSpacing: isEmoji ? 0 : '-0.02em',
        fontFamily: isEmoji ? 'inherit' : 'var(--wks-font-sans)',
        userSelect: 'none',
      }}
    >
      {glyph}
    </span>
  );

  // The reserved box, identical whichever mark landed in it.
  const mark = (
    <span
      style={{
        width: box,
        height: box,
        flex: 'none',
        display: 'grid',
        placeItems: 'center',
      }}
    >
      {inner}
    </span>
  );

  if (!withLabel) {
    return (
      <span
        title={title ?? p.label}
        style={{ display: 'inline-flex', alignItems: 'center', flex: 'none' }}
      >
        {mark}
      </span>
    );
  }
  return (
    <span
      title={title ?? p.label}
      style={{ display: 'inline-flex', alignItems: 'center', gap: 6, minWidth: 0 }}
    >
      {mark}
      <span
        style={{
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {p.label}
      </span>
    </span>
  );
};
