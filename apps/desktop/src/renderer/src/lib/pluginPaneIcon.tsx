import React from 'react';
import { Puzzle } from 'lucide-react';

/**
 * Render a plugin pane's declared icon: an http(s) URL becomes a favicon-style
 * image, anything else falls back to the puzzle glyph. Mirrors the command
 * palette's `userIcon` handling so a plugin looks the same wherever it's listed
 * (palette, split menu, new-tab menu).
 */
export function PluginPaneIcon({ icon, size = 13 }: { icon?: string; size?: number }) {
  if (icon && /^https?:\/\//.test(icon)) {
    return (
      <img
        src={icon}
        width={size}
        height={size}
        style={{ borderRadius: 3, objectFit: 'contain' }}
        alt=""
      />
    );
  }
  return <Puzzle size={size} strokeWidth={1.75} />;
}
