import React, { useState } from 'react';
import { ContextMenu } from '../ContextMenu';
import { FileActionMenuItems, resolveWithCwd } from './FileLink';
import { useImagePreviews } from './imagePreviews';
import { requestOpenInBrowser, fileUrlFromPath } from '../../lib/browserBus';

/** Tile edge in the transcript. Larger than the composer's 56px chips — here
 *  it's the content, not a pending-attachment reminder. */
const TILE = 132;

/**
 * The images in a message, rendered inline.
 *
 * Only paths that actually decode get a tile: `useImagePreviews` returns
 * nothing for a file that's missing, too large, or not really an image, so a
 * stale path in an old transcript degrades to no tile rather than a broken one.
 * That also means this renders nothing at all until the previews land, which is
 * why it never reserves space.
 *
 * Click opens the image in a browser pane (in-app, no OS handler involved);
 * right-click gets the same file menu as any other path in the chat.
 */
export const MessageImages: React.FC<{
  paths: string[];
  /** Session cwd, for the rare relative path. */
  cwd?: string;
  /** Own margin, so the caller decides the spacing above. */
  style?: React.CSSProperties;
  /** Rendered instead of the tiles when nothing decodes. Without it an
   *  image-ONLY message would be an empty bubble: the marker was already
   *  stripped from the text on extension alone, before any decode was tried. */
  fallback?: React.ReactNode;
}> = ({ paths, cwd, style, fallback }) => {
  const [menu, setMenu] = useState<{ x: number; y: number; path: string } | null>(null);
  const absolute = paths.map((p) => resolveWithCwd(p, cwd));
  const { previews, settled } = useImagePreviews(absolute);
  const shown = absolute.filter((p) => previews[p]);
  // Nothing yet: stay empty while the reads are in flight, then hand over to
  // the fallback once we know none of them will ever render.
  if (shown.length === 0) return settled ? <>{fallback ?? null}</> : null;

  return (
    <>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, ...style }}>
        {shown.map((path) => {
          const name = path.replace(/\\/g, '/').split('/').pop() ?? path;
          return (
            <img
              key={path}
              src={previews[path]}
              alt={name}
              title={`${path} — click to open`}
              onClick={(e) => {
                e.stopPropagation();
                requestOpenInBrowser({ url: fileUrlFromPath(path), title: name });
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setMenu({ x: e.clientX, y: e.clientY, path });
              }}
              style={{
                maxWidth: TILE,
                maxHeight: TILE,
                borderRadius: 'var(--wks-radius-md)',
                border: '1px solid var(--wks-border-subtle)',
                objectFit: 'cover',
                cursor: 'pointer',
                display: 'block',
              }}
            />
          );
        })}
      </div>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <FileActionMenuItems path={menu.path} cwd={cwd} onClose={() => setMenu(null)} />
        </ContextMenu>
      )}
    </>
  );
};
